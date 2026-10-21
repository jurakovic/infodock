//! The `eject-list` popup (SPEC §9.22) of Eject USB: one row per drive that can be ejected,
//! with its shell icon, name, size and an *Eject* button, then *Safely Remove Hardware…*. The
//! drives come from the sensor's `volumes` samples, so the list follows drives arriving and
//! going while it is open; sizes are read on a worker.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use super::eject;
use super::folder_browser::{centred, highlight};
use super::icon::{self, Icon};
use crate::bar::theme::Theme;
use crate::i18n::t;
use crate::platform::{
    Bitmap, Canvas, Color, Key, Launch, PointF, PointerButton, PopupInput, RectF, Shape, SizeF,
    TextLayout, TextStyle, Volume,
};
use crate::sampler::{Change, ChangeSink};
use crate::sensors::format::{self, Numbers};
use crate::sensors::popup::{Popup, PopupCtx, PopupReply};
use crate::sensors::{ShellIcon, disk};

/// A row's icon when the shell has none (Appendix C).
const DRIVE: &[u8] = include_bytes!("../../../assets/icons/drive.svg");

/// The popup's width, logical px.
const WIDTH: f32 = 300.0;
/// Rows (drives, the message, the footer) and their icons.
const ROW: f32 = 28.0;
const ICON: f32 = 16.0;
const ICON_X: f32 = 6.0;
const TEXT_X: f32 = ICON_X + ICON + 6.0;
/// Space after the last thing in a row, and between the name, size and button.
const END: f32 = 6.0;
const GAP: f32 = 8.0;
/// The *Eject* button: its height and the space either side of its text.
const BUTTON_HEIGHT: f32 = 22.0;
const BUTTON_PAD: f32 = 10.0;
const BUTTON_RADIUS: f32 = 4.0;
/// The line above the footer and the space around it.
const SEPARATOR: f32 = 9.0;
/// The system's *Safely Remove Hardware* dialog (SPEC §9.22).
const HOTPLUG: (&str, &str) = ("rundll32.exe", "shell32.dll,Control_RunDLL hotplug.dll");

/// The drives that can be ejected now, as the sensor last heard; shared with an open list.
#[derive(Debug, Default)]
pub struct Drives {
    pub volumes: Vec<Volume>,
    /// Counts the changes, so an open list sees that it is out of date.
    pub generation: u64,
}

/// The drives that can be ejected (SPEC §9.22): removable drives, disks on USB and the like,
/// and optical drives holding a disc; never one without a medium.
pub fn ejectable(volumes: &[Volume]) -> Vec<Volume> {
    volumes
        .iter()
        .filter(|v| v.ejectable && v.media)
        .cloned()
        .collect()
}

/// Something the pointer or keyboard can act on: a drive's *Eject* button (by its root, so a
/// mark stays on its drive when others come or go), or the footer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Eject(String),
    Footer,
}

/// A drive as its size is kept: a new medium in the same letter is another drive.
type SizeKey = (String, String);

fn size_key(volume: &Volume) -> SizeKey {
    (volume.root.clone(), volume.label.clone())
}

struct RowView {
    /// `None` while the shell icon loads.
    icon: Option<Bitmap>,
    name: TextLayout,
    size: Option<TextLayout>,
    button: TextLayout,
    ejecting: bool,
}

struct View {
    size: SizeF,
    rows: Vec<RowView>,
    /// *Nothing to eject*.
    message: Option<TextLayout>,
    footer: Option<TextLayout>,
    highlight: Color,
}

impl Default for View {
    fn default() -> Self {
        Self {
            size: SizeF::default(),
            rows: Vec::new(),
            message: None,
            footer: None,
            highlight: Color::TRANSPARENT,
        }
    }
}

pub struct EjectList {
    drives: Arc<Mutex<Drives>>,
    /// The drives as shown, and the generation they came from.
    volumes: Vec<Volume>,
    generation: Option<u64>,
    /// Total sizes as read; `None` when unknown (asked again when the drives change).
    sizes: HashMap<SizeKey, Option<u64>>,
    /// Sizes being read.
    asked: HashSet<SizeKey>,
    sizes_tx: Sender<(SizeKey, Option<u64>)>,
    sizes_rx: Receiver<(SizeKey, Option<u64>)>,
    /// Roots being ejected, until their result comes back.
    ejecting: HashSet<String>,
    ejected_tx: Sender<String>,
    ejected_rx: Receiver<String>,
    numbers: Numbers,
    changes: ChangeSink,
    current: Option<Target>,
    pressed: Option<Target>,
    view: View,
}

impl EjectList {
    pub fn new(drives: Arc<Mutex<Drives>>, numbers: Numbers, changes: ChangeSink) -> Self {
        let (sizes_tx, sizes_rx) = mpsc::channel();
        let (ejected_tx, ejected_rx) = mpsc::channel();
        Self {
            drives,
            volumes: Vec::new(),
            generation: None,
            sizes: HashMap::new(),
            asked: HashSet::new(),
            sizes_tx,
            sizes_rx,
            ejecting: HashSet::new(),
            ejected_tx,
            ejected_rx,
            numbers,
            changes,
            current: None,
            pressed: None,
            view: View::default(),
        }
    }

    /// Sizes known in advance, for screenshots.
    #[cfg(test)]
    pub fn preset_sizes(&mut self, sizes: &[(&str, u64)]) {
        let drives = self
            .drives
            .lock()
            .map(|d| d.volumes.clone())
            .unwrap_or_default();
        for (root, total) in sizes {
            if let Some(volume) = drives.iter().find(|v| v.root == *root) {
                self.sizes.insert(size_key(volume), Some(*total));
            }
        }
    }

    /// Takes the sensor's drives if they changed since; returns whether they did.
    fn refresh(&mut self) -> bool {
        let Ok(drives) = self.drives.lock() else {
            return false;
        };
        if self.generation == Some(drives.generation) {
            return false;
        }
        self.generation = Some(drives.generation);
        self.volumes = drives.volumes.clone();
        drop(drives);
        // The marked and pressed buttons stay on their drive, or go when the drive did.
        let present = |root: &str| self.volumes.iter().any(|v| v.root == root);
        for target in [&mut self.current, &mut self.pressed] {
            if matches!(target, Some(Target::Eject(root)) if !present(root)) {
                *target = None;
            }
        }
        self.ejecting.retain(|root| present(root));
        // Sizes of drives that went, and ones that could not be read, are read again.
        let keys: HashSet<SizeKey> = self.volumes.iter().map(size_key).collect();
        self.sizes
            .retain(|key, total| total.is_some() && keys.contains(key));
        true
    }

    /// Starts reading the sizes not known or asked for yet.
    fn read_sizes(&mut self, cx: &PopupCtx) {
        let keys: Vec<SizeKey> = self
            .volumes
            .iter()
            .map(size_key)
            .filter(|key| !self.sizes.contains_key(key) && !self.asked.contains(key))
            .collect();
        if keys.is_empty() {
            return;
        }
        self.asked.extend(keys.iter().cloned());
        let platform = Arc::clone(cx.platform);
        let waker = cx.waker.clone();
        let tx = self.sizes_tx.clone();
        let spawned = std::thread::Builder::new()
            .name("drive-sizes".into())
            .spawn(move || {
                for key in keys {
                    let total = platform.disk_space(&key.0).map(|s| s.total);
                    if tx.send((key, total)).is_err() {
                        return;
                    }
                }
                waker.wake();
            });
        if let Err(e) = spawned {
            tracing::warn!("could not start reading drive sizes: {e}");
        }
    }

    /// Ejects drive `root` in the background; the result is a notification (SPEC §9.22).
    fn eject(&mut self, root: &str, cx: &PopupCtx) -> PopupReply {
        let Some(volume) = self.volumes.iter().find(|v| v.root == root) else {
            return PopupReply::Nothing;
        };
        if !self.ejecting.insert(volume.root.clone()) {
            return PopupReply::Nothing;
        }
        let name = disk::volume_name(volume);
        let root = volume.root.clone();
        let tx = self.ejected_tx.clone();
        let changes = self.changes.clone();
        let waker = cx.waker.clone();
        cx.platform.eject(
            &volume.root,
            Box::new(move |result| {
                // Read the drives again at once: an ejected one goes from the list.
                changes.report(Change::Devices);
                if tx.send(root).is_ok() {
                    waker.wake();
                }
                eject::notice(&name, result)
            }),
        );
        PopupReply::Relayout
    }

    fn activate(&mut self, target: Target, cx: &PopupCtx) -> PopupReply {
        match target {
            Target::Eject(root) => self.eject(&root, cx),
            Target::Footer => {
                let (program, args) = HOTPLUG;
                cx.platform.launch(&Launch {
                    target: program,
                    args,
                    dir: None,
                    admin: false,
                });
                PopupReply::Close
            }
        }
    }

    /// The list's height: a row per drive, or one for the message.
    fn list_height(&self) -> f32 {
        self.volumes.len().max(1) as f32 * ROW
    }

    fn footer_top(&self) -> f32 {
        self.list_height() + SEPARATOR
    }

    /// Row `index`'s *Eject* button, content px.
    fn button_rect(&self, index: usize) -> Option<RectF> {
        let button = &self.view.rows.get(index)?.button;
        let right = self.view.size.width - END;
        let left = right - button.width() - 2.0 * BUTTON_PAD;
        let top = index as f32 * ROW + (ROW - BUTTON_HEIGHT) / 2.0;
        Some(RectF::new(left, top, right, top + BUTTON_HEIGHT))
    }

    fn target_at(&self, at: PointF) -> Option<Target> {
        if at.x < 0.0 || at.x >= self.view.size.width || at.y < 0.0 {
            return None;
        }
        let footer = self.footer_top();
        if (footer..footer + ROW).contains(&at.y) {
            return Some(Target::Footer);
        }
        let index = (at.y / ROW).floor() as usize;
        let rect = self.button_rect(index)?;
        let inside =
            (rect.left..rect.right).contains(&at.x) && (rect.top..rect.bottom).contains(&at.y);
        (inside && !self.view.rows[index].ejecting)
            .then(|| Target::Eject(self.volumes[index].root.clone()))
    }

    /// The targets in keyboard order: the buttons of drives not being ejected, then the footer.
    fn targets(&self) -> Vec<Target> {
        self.volumes
            .iter()
            .enumerate()
            .filter(|(_, v)| !self.ejecting.contains(&v.root))
            .map(|(_, v)| Target::Eject(v.root.clone()))
            .chain(std::iter::once(Target::Footer))
            .collect()
    }

    fn select(&mut self, key: Key) -> PopupReply {
        let targets = self.targets();
        let at = self
            .current
            .as_ref()
            .and_then(|c| targets.iter().position(|t| t == c));
        let last = targets.len() - 1;
        let next = match (key, at) {
            (Key::Home | Key::PageUp, _) | (Key::Down, None) => 0,
            (Key::End | Key::PageDown, _) | (Key::Up, None) => last,
            (Key::Down, Some(i)) => (i + 1).min(last),
            (Key::Up, Some(i)) => i.saturating_sub(1),
            _ => return PopupReply::Nothing,
        };
        self.current = targets.into_iter().nth(next);
        PopupReply::Redraw
    }
}

impl Popup for EjectList {
    fn kind(&self) -> &'static str {
        "eject-list"
    }

    fn name(&self) -> String {
        t("sensor.ejectUsb.name")
    }

    fn layout(&mut self, cx: &PopupCtx) -> SizeF {
        self.refresh();
        self.read_sizes(cx);
        let width = WIDTH.min(cx.room.width).floor();
        let height = (self.footer_top() + ROW).min(cx.room.height);
        self.view.size = SizeF::new(width, height);
        let style = TextStyle::new(&cx.theme.popup_font, cx.theme.popup_font_size);
        let px = icon::physical(ICON, cx.scale);
        let tint = cx.theme.popup_text;
        let eject_label = t("sensor.ejectUsb.eject");
        let ejecting_label = t("sensor.ejectUsb.ejecting");
        let rows = self
            .volumes
            .iter()
            .map(|volume| {
                let ejecting = self.ejecting.contains(&volume.root);
                let label = if ejecting {
                    &ejecting_label
                } else {
                    &eject_label
                };
                let button = cx.text.layout(label, &style);
                let size = self
                    .sizes
                    .get(&size_key(volume))
                    .copied()
                    .flatten()
                    .map(|total| {
                        cx.text
                            .layout(&format::bytes(total, None, self.numbers), &style)
                    });
                let used = button.width()
                    + 2.0 * BUTTON_PAD
                    + size.as_ref().map_or(0.0, |s| s.width() + GAP);
                let name_style = style
                    .clone()
                    .max_width((width - TEXT_X - GAP - used - END).max(1.0));
                let icon = match cx.icons.shell_icon(&volume.root, px) {
                    ShellIcon::Ready(icon) => Some(icon),
                    ShellIcon::Loading => None,
                    ShellIcon::None => Icon::svg(DRIVE, px).map(|b| b.tinted(tint)),
                };
                RowView {
                    icon,
                    name: cx.text.layout(&disk::volume_name(volume), &name_style),
                    size,
                    button,
                    ejecting,
                }
            })
            .collect();
        self.view.rows = rows;
        self.view.message = self
            .volumes
            .is_empty()
            .then(|| cx.text.layout(&t("sensor.ejectUsb.nothing"), &style));
        let footer_style = style.max_width((width - 2.0 * ICON_X).max(1.0));
        self.view.footer = Some(
            cx.text
                .layout(&t("sensor.ejectUsb.safelyRemove"), &footer_style),
        );
        self.view.highlight = highlight(cx.theme.popup_background);
        self.view.size
    }

    fn draw(&self, canvas: &mut dyn Canvas, origin: PointF, theme: &Theme) {
        let view = &self.view;
        let width = view.size.width;
        let text = theme.popup_text;
        canvas.push_clip(RectF::at(origin, view.size));
        for (i, row) in view.rows.iter().enumerate() {
            let top = origin.y + i as f32 * ROW;
            if let Some(icon) = &row.icon {
                let scale = canvas.scale();
                let (w, h) = (icon.width() as f32 / scale, icon.height() as f32 / scale);
                let x = canvas.snap(origin.x + ICON_X);
                let y = canvas.snap(top + (ROW - h) / 2.0);
                canvas.draw_bitmap(icon, RectF::new(x, y, x + w, y + h), 1.0);
            }
            let y = centred(canvas, top, ROW, &row.name);
            canvas.draw_text(&row.name, PointF::new(origin.x + TEXT_X, y), text);
            let Some(button) = self.button_rect(i) else {
                continue;
            };
            let button = RectF::new(
                origin.x + button.left,
                origin.y + button.top,
                origin.x + button.right,
                origin.y + button.bottom,
            );
            if let Some(size) = &row.size {
                let x = canvas.snap(button.left - GAP - size.width());
                let y = centred(canvas, top, ROW, size);
                canvas.draw_text(size, PointF::new(x, y), text.with_alpha(text.a * 0.7));
            }
            let shape = Shape::RoundedRect {
                rect: button,
                radius: BUTTON_RADIUS,
            };
            let marked =
                matches!(&self.current, Some(Target::Eject(root)) if *root == self.volumes[i].root);
            if marked {
                canvas.fill(&shape, &view.highlight.into());
            }
            let outline = text.with_alpha(text.a * if row.ejecting { 0.2 } else { 0.4 });
            canvas.stroke(&shape, &outline.into(), 1.0);
            let color = if row.ejecting {
                text.with_alpha(text.a * 0.5)
            } else {
                text
            };
            let x = canvas.snap(button.left + BUTTON_PAD);
            let y = centred(canvas, button.top, BUTTON_HEIGHT, &row.button);
            canvas.draw_text(&row.button, PointF::new(x, y), color);
        }
        if let Some(message) = &view.message {
            let x = canvas.snap(origin.x + (width - message.width()) / 2.0);
            let y = centred(canvas, origin.y, ROW, message);
            canvas.draw_text(message, PointF::new(x, y), text);
        }
        let line = origin.y + self.list_height() + (SEPARATOR - 1.0) / 2.0;
        canvas.fill_rect(
            RectF::new(origin.x, line, origin.x + width, line + 1.0),
            text.with_alpha(text.a * 0.25),
        );
        let footer = origin.y + self.footer_top();
        if self.current == Some(Target::Footer) {
            canvas.fill_rect(
                RectF::new(origin.x, footer, origin.x + width, footer + ROW),
                view.highlight,
            );
        }
        if let Some(label) = &view.footer {
            let y = centred(canvas, footer, ROW, label);
            canvas.draw_text(label, PointF::new(origin.x + ICON_X, y), text);
        }
        canvas.pop();
    }

    fn input(&mut self, input: &PopupInput, cx: &PopupCtx) -> PopupReply {
        match input {
            PopupInput::PointerMove(at) => {
                let target = self.target_at(*at);
                if target != self.current {
                    self.current = target;
                    return PopupReply::Redraw;
                }
                PopupReply::Nothing
            }
            PopupInput::PointerLeave => {
                if self.current.take().is_some() {
                    return PopupReply::Redraw;
                }
                PopupReply::Nothing
            }
            PopupInput::PointerDown {
                button: PointerButton::Left,
                at,
                ..
            } => {
                self.pressed = self.target_at(*at);
                PopupReply::Nothing
            }
            PopupInput::PointerUp {
                button: PointerButton::Left,
                at,
                ..
            } => match (self.pressed.take(), self.target_at(*at)) {
                (Some(pressed), Some(target)) if pressed == target => self.activate(target, cx),
                _ => PopupReply::Nothing,
            },
            PopupInput::Key {
                key: Key::Enter, ..
            } => match self.current.clone() {
                Some(target) => self.activate(target, cx),
                None => PopupReply::Nothing,
            },
            PopupInput::Key { key, .. } => self.select(*key),
            _ => PopupReply::Nothing,
        }
    }

    /// Drives that came or went, sizes read, and ejects that finished.
    fn poll(&mut self, _cx: &PopupCtx) -> PopupReply {
        let mut changed = self.refresh();
        while let Ok((key, total)) = self.sizes_rx.try_recv() {
            self.asked.remove(&key);
            self.sizes.insert(key, total);
            changed = true;
        }
        while let Ok(root) = self.ejected_rx.try_recv() {
            changed |= self.ejecting.remove(&root);
        }
        if changed {
            PopupReply::Relayout
        } else {
            PopupReply::Nothing
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::platform::recording::RecordingCanvas;
    use crate::platform::{DriveKind, Modifiers};
    use crate::sensors::common::folder_browser::tests::Harness;
    use crate::sensors::test_support::theme;

    fn stick(root: &str, label: &str) -> Volume {
        Volume::new(root, DriveKind::Removable, label)
    }

    fn list(volumes: Vec<Volume>) -> (EjectList, Arc<Mutex<Drives>>) {
        let drives = Arc::new(Mutex::new(Drives {
            volumes,
            generation: 1,
        }));
        let list = EjectList::new(Arc::clone(&drives), Numbers::default(), ChangeSink::NONE);
        (list, drives)
    }

    fn drawn(list: &EjectList) -> String {
        let mut canvas = RecordingCanvas::new(1.0);
        list.draw(&mut canvas, PointF::new(0.0, 0.0), &theme());
        canvas.dump()
    }

    fn texts(dump: &str) -> Vec<String> {
        dump.lines()
            .filter_map(|l| l.split('"').nth(1).map(str::to_owned))
            .collect()
    }

    fn click(list: &mut EjectList, h: &Harness, at: PointF) -> PopupReply {
        let modifiers = Modifiers::default();
        let down = PopupInput::PointerDown {
            button: PointerButton::Left,
            at,
            modifiers,
        };
        list.input(&down, &h.ctx());
        let up = PopupInput::PointerUp {
            button: PointerButton::Left,
            at,
            modifiers,
        };
        list.input(&up, &h.ctx())
    }

    /// The middle of row `i`'s button.
    fn button(list: &EjectList, i: usize) -> PointF {
        let rect = list.button_rect(i).expect("a button");
        PointF::new(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        )
    }

    #[test]
    fn only_drives_with_media_that_can_be_ejected() {
        let mut empty_reader = stick("F:\\", "");
        empty_reader.media = false;
        let volumes = vec![
            Volume::new("C:\\", DriveKind::Fixed, "System"),
            stick("E:\\", "STICK"),
            empty_reader,
            Volume::new("D:\\", DriveKind::Optical, "DISC"),
        ];
        let roots: Vec<_> = ejectable(&volumes).into_iter().map(|v| v.root).collect();
        assert_eq!(roots, ["E:\\", "D:\\"]);
    }

    #[test]
    fn rows_with_eject_buttons_and_the_footer() {
        let h = Harness::new();
        let (mut list, _) = list(vec![stick("E:\\", "STICK"), stick("G:\\", "")]);
        assert_eq!(
            list.layout(&h.ctx()),
            SizeF::new(300.0, 2.0 * ROW + 9.0 + ROW)
        );
        assert_eq!(
            texts(&drawn(&list)),
            [
                "STICK (E:)",
                "Eject",
                "USB Drive (G:)",
                "Eject",
                "Safely Remove Hardware…"
            ]
        );
        // The sizes arrive from the worker (none on the test platform).
        std::thread::sleep(Duration::from_millis(50));
        list.poll(&h.ctx());
        assert_eq!(list.kind(), "eject-list");
    }

    #[test]
    fn nothing_to_eject() {
        let h = Harness::new();
        let (mut list, _) = list(Vec::new());
        assert_eq!(list.layout(&h.ctx()), SizeF::new(300.0, ROW + 9.0 + ROW));
        assert_eq!(
            texts(&drawn(&list)),
            ["Nothing to eject", "Safely Remove Hardware…"]
        );
    }

    #[test]
    fn eject_reports_and_the_list_follows_the_drives() {
        let h = Harness::new();
        let (mut list, drives) = list(vec![stick("E:\\", "STICK"), stick("G:\\", "")]);
        list.layout(&h.ctx());
        let at = button(&list, 1);
        assert_eq!(
            list.input(&PopupInput::PointerMove(at), &h.ctx()),
            PopupReply::Redraw
        );
        assert_eq!(click(&mut list, &h, at), PopupReply::Relayout);
        assert_eq!(
            h.platform.log(),
            ["eject G:\\: USB Drive (G:): USB Drive (G:) can be safely removed."]
        );
        list.layout(&h.ctx());
        // The fake platform reports at once: the button is back.
        assert_eq!(list.poll(&h.ctx()), PopupReply::Relayout);
        list.layout(&h.ctx());
        assert!(!texts(&drawn(&list)).contains(&"Ejecting…".to_owned()));
        // The drive goes: the list follows.
        {
            let mut drives = drives.lock().expect("lock");
            drives.volumes.remove(1);
            drives.generation += 1;
        }
        assert_eq!(list.poll(&h.ctx()), PopupReply::Relayout);
        list.layout(&h.ctx());
        assert_eq!(
            texts(&drawn(&list)),
            ["STICK (E:)", "Eject", "Safely Remove Hardware…"]
        );
        // Once the size readings are in, nothing more changes.
        std::thread::sleep(Duration::from_millis(50));
        list.poll(&h.ctx());
        list.layout(&h.ctx());
        assert_eq!(list.poll(&h.ctx()), PopupReply::Nothing);
    }

    #[test]
    fn the_mark_stays_on_its_drive() {
        let h = Harness::new();
        let (mut list, drives) = list(vec![stick("E:\\", "STICK"), stick("G:\\", "")]);
        list.layout(&h.ctx());
        let key = |key| PopupInput::Key {
            key,
            modifiers: Modifiers::default(),
        };
        list.input(&key(Key::Down), &h.ctx());
        assert_eq!(list.current, Some(Target::Eject("E:\\".into())));
        // E: goes: nothing is marked, so Enter ejects nothing else.
        {
            let mut drives = drives.lock().expect("lock");
            drives.volumes.remove(0);
            drives.generation += 1;
        }
        list.poll(&h.ctx());
        assert_eq!(list.current, None);
        assert_eq!(list.input(&key(Key::Enter), &h.ctx()), PopupReply::Nothing);
        assert!(h.platform.log().is_empty());
    }

    #[test]
    fn a_drive_being_ejected_says_so() {
        let h = Harness::new();
        let (mut list, _) = list(vec![stick("E:\\", "STICK")]);
        list.ejecting.insert("E:\\".into());
        list.layout(&h.ctx());
        assert_eq!(
            texts(&drawn(&list)),
            ["STICK (E:)", "Ejecting…", "Safely Remove Hardware…"]
        );
        assert_eq!(list.targets(), [Target::Footer]);
    }

    #[test]
    fn keys_and_the_footer() {
        let h = Harness::new();
        let (mut list, _) = list(vec![stick("E:\\", "STICK")]);
        list.layout(&h.ctx());
        let key = |key| PopupInput::Key {
            key,
            modifiers: Modifiers::default(),
        };
        assert_eq!(list.input(&key(Key::Down), &h.ctx()), PopupReply::Redraw);
        assert_eq!(list.current, Some(Target::Eject("E:\\".into())));
        list.input(&key(Key::Down), &h.ctx());
        assert_eq!(list.current, Some(Target::Footer));
        let dump = drawn(&list);
        assert!(dump.contains("fill rect 0,37 300×28 #A9DFFF/40%"), "{dump}");
        assert_eq!(list.input(&key(Key::Enter), &h.ctx()), PopupReply::Close);
        assert_eq!(
            h.platform.log(),
            ["launch rundll32.exe args=shell32.dll,Control_RunDLL hotplug.dll dir= admin=false"]
        );
        list.input(&key(Key::Home), &h.ctx());
        assert_eq!(list.current, Some(Target::Eject("E:\\".into())));
    }
}
