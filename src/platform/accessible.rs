//! The bar as screen readers see it (SPEC §14.4): the bar logic describes its sensors after each
//! frame, and the platform's UI Automation provider (`windows/uia.rs`) answers from that.

use super::RectF;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> AccessibleBar {
        let item = |id: &str, left: f32| AccessibleItem {
            id: id.into(),
            name: id.to_uppercase(),
            value: String::new(),
            rect: RectF::new(left, 0.0, left + 40.0, 28.0),
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
}
