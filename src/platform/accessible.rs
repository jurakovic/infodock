//! The bar as screen readers see it (SPEC §14.4): the bar logic describes its sensors after each
//! frame, and the platform's UI Automation provider (`windows/uia.rs`) answers from that, raises
//! the events that tell screen readers what changed ([`events`]) and hands what they ask for
//! back to the bar logic ([`AccessibleAction`]).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::RectF;

/// A sensor's value is announced at most this often, so graphs do not flood screen readers.
pub const VALUE_EVENTS: Duration = Duration::from_secs(1);

/// A sensor on the bar, as UI Automation lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessibleItem {
    /// The sensor's id: the element's automation id, stable across starts.
    pub id: String,
    /// Its type's name, e.g. "CPU Graph".
    pub name: String,
    /// What it shows now, read through the Value pattern (`Sensor::accessible_value`).
    pub value: String,
    /// Where it is, logical px from the bar window's top-left corner.
    pub rect: RectF,
    /// Whether its popup is open, for a sensor whose click opens one (the ExpandCollapse
    /// pattern); `None` for the others.
    pub popup: Option<bool>,
}

/// The bar's sensors in focus order (SPEC §14.4), and where the keyboard focus is.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccessibleBar {
    pub items: Vec<AccessibleItem>,
    /// The bar has the keyboard (the hotkey focused it).
    pub focused: bool,
    /// The item with the focus ring, an index into `items`; `None`: the bar itself.
    pub focus: Option<usize>,
}

impl AccessibleBar {
    /// The item with the keyboard focus, while the bar has it.
    pub fn focused_item(&self) -> Option<&AccessibleItem> {
        self.items.get(self.focus.filter(|_| self.focused)?)
    }

    /// The item with id `id`.
    pub fn item(&self, id: &str) -> Option<&AccessibleItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The item at `point` (logical px from the bar's top-left corner).
    pub fn item_at(&self, x: f32, y: f32) -> Option<&AccessibleItem> {
        self.items.iter().find(|i| {
            let r = i.rect;
            x >= r.left && x < r.right && y >= r.top && y < r.bottom
        })
    }

    /// The item before (`-1`) or after (`1`) item `id`.
    pub fn sibling(&self, id: &str, offset: isize) -> Option<&AccessibleItem> {
        let at = self.items.iter().position(|i| i.id == id)?;
        self.items.get(at.checked_add_signed(offset)?)
    }
}

/// What a screen reader asks of a sensor, by id; the bar logic does it (SPEC §14.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessibleAction {
    /// The Invoke pattern: a click.
    Invoke(String),
    /// The ExpandCollapse pattern: opens its popup, or closes it.
    Expand(String),
    Collapse(String),
    /// The focus ring goes to it.
    Focus(String),
}

/// What changed between two descriptions of the bar, for UI Automation's events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessibleEvent {
    /// Sensors were added, removed or moved.
    Structure,
    /// The keyboard focus went to this sensor.
    Focus(String),
    /// A sensor's value changed from `old` (as last announced) to `new`.
    Value {
        id: String,
        old: String,
        new: String,
    },
    /// A sensor's popup opened (`true`) or closed.
    Expanded { id: String, open: bool },
}

/// Which values were last announced and when, so each sensor's are announced at most once per
/// [`VALUE_EVENTS`].
#[derive(Debug, Default)]
pub struct Announced(HashMap<String, (String, Instant)>);

/// The events that tell screen readers how the bar changed from `old` to `new` at `now`. A
/// value that changes again within [`VALUE_EVENTS`] of its last announcement waits for a
/// later description; the first value a sensor has is not announced (it is read with the
/// sensor).
pub fn events(
    old: &AccessibleBar,
    new: &AccessibleBar,
    announced: &mut Announced,
    now: Instant,
) -> Vec<AccessibleEvent> {
    let mut events = Vec::new();
    let ids = |bar: &AccessibleBar| bar.items.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    if ids(old) != ids(new) {
        events.push(AccessibleEvent::Structure);
    }
    announced.0.retain(|id, _| new.item(id).is_some());
    for item in &new.items {
        let Some((last, at)) = announced.0.get_mut(&item.id) else {
            announced
                .0
                .insert(item.id.clone(), (item.value.clone(), now));
            continue;
        };
        if *last != item.value && now.saturating_duration_since(*at) >= VALUE_EVENTS {
            events.push(AccessibleEvent::Value {
                id: item.id.clone(),
                old: std::mem::replace(last, item.value.clone()),
                new: item.value.clone(),
            });
            *at = now;
        }
        if let (Some(before), Some(open)) = (old.item(&item.id).and_then(|i| i.popup), item.popup)
            && before != open
        {
            events.push(AccessibleEvent::Expanded {
                id: item.id.clone(),
                open,
            });
        }
    }
    let focus = |bar: &AccessibleBar| bar.focused_item().map(|i| i.id.clone());
    if let Some(id) = focus(new)
        && focus(old) != Some(id.clone())
    {
        events.push(AccessibleEvent::Focus(id));
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> AccessibleBar {
        let item = |id: &str, left: f32| AccessibleItem {
            id: id.into(),
            name: id.to_uppercase(),
            value: String::new(),
            rect: RectF::new(left, 0.0, left + 40.0, 28.0),
            popup: None,
        };
        AccessibleBar {
            items: vec![item("a", 4.0), item("b", 52.0), item("c", 100.0)],
            focused: true,
            focus: Some(1),
        }
    }

    #[test]
    fn finds_items_by_id_point_and_neighbour() {
        let bar = bar();
        assert_eq!(bar.item("b").map(|i| i.name.as_str()), Some("B"));
        assert_eq!(bar.item_at(60.0, 10.0).map(|i| i.id.as_str()), Some("b"));
        assert_eq!(bar.item_at(46.0, 10.0), None, "in a gap");
        assert_eq!(bar.item_at(60.0, 30.0), None, "below the bar");
        assert_eq!(bar.sibling("b", 1).map(|i| i.id.as_str()), Some("c"));
        assert_eq!(bar.sibling("b", -1).map(|i| i.id.as_str()), Some("a"));
        assert_eq!(bar.sibling("a", -1), None);
        assert_eq!(bar.sibling("c", 1), None);
        assert_eq!(bar.sibling("x", 1), None);
    }

    #[test]
    fn the_focused_item_needs_the_bar_focused() {
        let mut bar = bar();
        assert_eq!(bar.focused_item().map(|i| i.id.as_str()), Some("b"));
        bar.focused = false;
        assert_eq!(bar.focused_item(), None);
        bar.focused = true;
        bar.focus = None;
        assert_eq!(bar.focused_item(), None, "the bar itself");
    }

    #[test]
    fn values_are_announced_at_most_once_a_second() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut announced = Announced::default();
        let mut old = bar();
        old.focused = false;
        let with = |value: &str| {
            let mut b = old.clone();
            b.items[0].value = value.into();
            b
        };
        // The first description announces nothing: values are read with their sensors.
        assert!(
            events(&AccessibleBar::default(), &old, &mut announced, at(0))
                .contains(&AccessibleEvent::Structure)
        );
        assert_eq!(
            events(&old, &with("37%"), &mut announced, at(500)),
            [],
            "too soon"
        );
        assert_eq!(
            events(&with("37%"), &with("38%"), &mut announced, at(1000)),
            [AccessibleEvent::Value {
                id: "a".into(),
                old: String::new(),
                new: "38%".into()
            }]
        );
        assert_eq!(
            events(&with("38%"), &with("40%"), &mut announced, at(1400)),
            []
        );
        // A second after the last: the newest value, against the one last announced.
        assert_eq!(
            events(&with("40%"), &with("41%"), &mut announced, at(2000)),
            [AccessibleEvent::Value {
                id: "a".into(),
                old: "38%".into(),
                new: "41%".into()
            }]
        );
        // Unchanged: nothing, however long.
        assert_eq!(
            events(&with("41%"), &with("41%"), &mut announced, at(9000)),
            []
        );
    }

    #[test]
    fn structure_focus_and_popups_are_announced() {
        let mut announced = Announced::default();
        let now = Instant::now();
        let old = bar();
        events(&AccessibleBar::default(), &old, &mut announced, now);
        // Moved: the structure changed; the focus stays on "b".
        let mut moved = old.clone();
        moved.items.swap(0, 2);
        assert_eq!(
            events(&old, &moved, &mut announced, now),
            [AccessibleEvent::Structure]
        );
        // The focus moves to "c".
        let mut next = old.clone();
        next.focus = Some(2);
        assert_eq!(
            events(&old, &next, &mut announced, now),
            [AccessibleEvent::Focus("c".into())]
        );
        // The bar takes the focus again, on the same sensor: announced again.
        let mut away = old.clone();
        away.focused = false;
        assert_eq!(events(&old, &away, &mut announced, now), []);
        assert_eq!(
            events(&away, &old, &mut announced, now),
            [AccessibleEvent::Focus("b".into())]
        );
        // A popup opens and closes.
        let (mut closed, mut open) = (old.clone(), old.clone());
        closed.items[0].popup = Some(false);
        open.items[0].popup = Some(true);
        assert_eq!(
            events(&closed, &open, &mut announced, now),
            [AccessibleEvent::Expanded {
                id: "a".into(),
                open: true
            }]
        );
        assert_eq!(
            events(&open, &closed, &mut announced, now),
            [AccessibleEvent::Expanded {
                id: "a".into(),
                open: false
            }]
        );
        // Removed: the structure; its value is forgotten.
        let mut fewer = old.clone();
        fewer.items.remove(0);
        fewer.focus = Some(0);
        assert_eq!(
            events(&old, &fewer, &mut announced, now),
            [AccessibleEvent::Structure]
        );
        assert!(!announced.0.contains_key("a"));
    }
}
