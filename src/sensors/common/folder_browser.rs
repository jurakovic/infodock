//! The `folder-browser` popup (SPEC §9.19), shared by Folder Menu and Start Menu: a header with
//! the folder's title over a list of its folders and files. The folder is listed on a worker
//! thread; rows show a 16 px shell icon and the name. A click (or Enter) opens an item and
//! closes the popup, Alt reveals it in Explorer; the header opens the folder itself.

use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use super::icon::{self, Icon};
use crate::bar::theme::Theme;
use crate::i18n::t;
use crate::platform::{
    Bitmap, Canvas, Color, DirItem, Key, Platform, PointF, PointerButton, PopupInput, RectF, SizeF,
    TextLayout, TextStyle, UiWaker,
};
use crate::sensors::ShellIcon;
use crate::sensors::popup::{Popup, PopupAlign, PopupCtx, PopupReply};

/// Row icons when the shell has none (Appendix C).
const FOLDER: &[u8] = include_bytes!("../../../assets/icons/folder.svg");
const FILE: &[u8] = include_bytes!("../../../assets/icons/file.svg");

/// The header's height and the line below it, logical px.
const HEADER: f32 = 24.0;
const LINE: f32 = 2.0;
/// Where the list starts.
const LIST_TOP: f32 = HEADER + LINE + 2.0;
/// Rows, their icons, and where icon and name start.
const ROW: f32 = 20.0;
const ICON: f32 = 16.0;
const ICON_X: f32 = 4.0;
const TEXT_X: f32 = ICON_X + ICON + 6.0;
/// Space after the text, and before the header's.
const END: f32 = 6.0;
/// The popup's size limits (SPEC §9.19).
pub const MIN_SIZE: f32 = 130.0;
pub const MAX_WIDTH: f32 = 600.0;
/// The first layout waits this long for the listing, so most folders appear complete.
const FIRST_WAIT: Duration = Duration::from_millis(150);
/// A listing that takes longer shows that it is loading.
const LOADING_AFTER: Duration = Duration::from_millis(200);

/// A folder or file in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// As shown: `.lnk` and `.url` without their extension.
    pub name: String,
    pub path: String,
    pub folder: bool,
}

/// The name shown for an entry: links (`.lnk`, `.url`) without their extension.
pub fn display_name(name: &str, folder: bool) -> String {
    if !folder {
        let lower = name.to_ascii_lowercase();
        for ext in [".lnk", ".url"] {
            if lower.ends_with(ext) && name.len() > ext.len() {
                return name[..name.len() - ext.len()].to_owned();
            }
        }
    }
    name.to_owned()
}

/// A folder's entries as the list shows them (SPEC §9.19): `desktop.ini` left out always,
/// hidden and system items unless `show_hidden`; folders first, then files, each in the order
/// of `cmp`.
pub fn prepare(
    entries: Vec<DirItem>,
    show_hidden: bool,
    cmp: impl Fn(&str, &str) -> Ordering,
) -> Vec<Item> {
    let mut items: Vec<Item> = entries
        .into_iter()
        .filter(|e| !e.name.eq_ignore_ascii_case("desktop.ini"))
        .filter(|e| show_hidden || !e.hidden)
        .map(|e| Item {
            name: display_name(&e.name, e.folder),
            path: e.path,
            folder: e.folder,
        })
        .collect();
    items.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| cmp(&a.name, &b.name)));
    items
}

/// Lists `folder` on a worker thread, which wakes the bar thread when it is done.
fn list(
    folder: String,
    show_hidden: bool,
    platform: Arc<dyn Platform>,
    waker: UiWaker,
) -> Receiver<Result<Vec<Item>, String>> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("folder".into())
        .spawn(move || {
            let result = platform
                .list_dir(&folder)
                .map(|entries| prepare(entries, show_hidden, |a, b| platform.compare_names(a, b)));
            if let Err(e) = &result {
                tracing::info!("folder {folder:?} cannot be listed: {e}");
            }
            if tx.send(result).is_ok() {
                waker.wake();
            }
        });
    if let Err(e) = spawned {
        // The sender went with the closure: the listing reads as failed.
        tracing::warn!("could not start listing a folder: {e}");
    }
    rx
}

enum Listing {
    Pending {
        rx: Receiver<Result<Vec<Item>, String>>,
        since: Instant,
    },
    Ready(Vec<Item>),
    Failed,
}

/// What the browser shows and how big it may be.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub folder: String,
    /// The header's text.
    pub title: String,
    pub show_hidden: bool,
    /// The popup's size, logical px; the height is a most (a short list makes it lower).
    pub width: f32,
    pub height: f32,
}

/// Something the pointer or keyboard can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Header,
    Row(usize),
}

/// The laid-out content.
struct View {
    size: SizeF,
    header: Option<TextLayout>,
    /// The visible rows: index, icon (`None` while loading) and name.
    rows: Vec<(usize, Option<Bitmap>, TextLayout)>,
    /// *no items*, *Loading…* or why the folder cannot be read.
    message: Option<TextLayout>,
    list_height: f32,
    /// The row highlight.
    highlight: Color,
}

impl Default for View {
    fn default() -> Self {
        Self {
            size: SizeF::default(),
            header: None,
            rows: Vec::new(),
            message: None,
            list_height: 0.0,
            highlight: Color::TRANSPARENT,
        }
    }
}

pub struct FolderBrowser {
    options: Options,
    /// `None` until the first layout starts it.
    listing: Option<Listing>,
    /// The hovered or selected target.
    current: Option<Target>,
    /// Where the left button went down.
    pressed: Option<Target>,
    /// How far the list is scrolled, logical px.
    scroll: f32,
    view: View,
}

impl FolderBrowser {
    pub fn new(options: Options) -> Self {
        Self {
            options,
            listing: None,
            current: None,
            pressed: None,
            scroll: 0.0,
            view: View::default(),
        }
    }

    fn items(&self) -> &[Item] {
        match &self.listing {
            Some(Listing::Ready(items)) => items,
            _ => &[],
        }
    }

    /// Takes the listing's result if it is there; returns whether it arrived.
    fn receive(&mut self, wait: Duration) -> bool {
        let Some(Listing::Pending { rx, .. }) = &self.listing else {
            return false;
        };
        let result = if wait.is_zero() {
            rx.try_recv().map_err(|e| e == TryRecvError::Disconnected)
        } else {
            rx.recv_timeout(wait)
                .map_err(|e| e == mpsc::RecvTimeoutError::Disconnected)
        };
        self.listing = Some(match result {
            Ok(Ok(items)) => Listing::Ready(items),
            Ok(Err(_)) | Err(true) => Listing::Failed,
            Err(false) => return false,
        });
        true
    }

    /// The most the list may scroll.
    fn max_scroll(&self) -> f32 {
        (self.items().len() as f32 * ROW - self.view.list_height).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// The target at `at` (content px).
    fn target_at(&self, at: PointF) -> Option<Target> {
        if at.x < 0.0 || at.x >= self.view.size.width || at.y < 0.0 {
            return None;
        }
        if at.y < HEADER {
            return Some(Target::Header);
        }
        let y = at.y - LIST_TOP;
        if y < 0.0 || y >= self.view.list_height {
            return None;
        }
        let index = ((y + self.scroll) / ROW).floor() as usize;
        (index < self.items().len()).then_some(Target::Row(index))
    }

    /// Scrolls so row `index` is in view.
    fn reveal_row(&mut self, index: usize) {
        let top = index as f32 * ROW;
        if top < self.scroll {
            self.scroll = top;
        } else if top + ROW > self.scroll + self.view.list_height {
            self.scroll = top + ROW - self.view.list_height;
        }
        self.clamp_scroll();
    }

    /// Moves the selection by `step` rows (or to the start or end) and keeps it in view.
    fn select(&mut self, key: Key) -> PopupReply {
        let count = self.items().len();
        if count == 0 {
            return PopupReply::Nothing;
        }
        let page = (self.view.list_height / ROW).floor().max(1.0) as usize;
        let current = match self.current {
            Some(Target::Row(i)) => Some(i),
            _ => None,
        };
        let next = match (key, current) {
            (Key::Home, _) | (Key::Down, None) => 0,
            (Key::End, _) | (Key::Up, None) => count - 1,
            (Key::Down, Some(i)) => (i + 1).min(count - 1),
            (Key::Up, Some(i)) => i.saturating_sub(1),
            (Key::PageDown, i) => i.map_or(0, |i| i + page).min(count - 1),
            (Key::PageUp, i) => i.map_or(0, |i| i.saturating_sub(page)),
            _ => return PopupReply::Nothing,
        };
        self.current = Some(Target::Row(next));
        self.reveal_row(next);
        PopupReply::Relayout
    }

    /// Opens (or with `alt`, reveals) `target`; the popup then closes.
    fn activate(&self, target: Target, alt: bool, platform: &dyn Platform) -> PopupReply {
        let path = match target {
            Target::Header => self.options.folder.as_str(),
            Target::Row(i) => match self.items().get(i) {
                Some(item) => item.path.as_str(),
                None => return PopupReply::Nothing,
            },
        };
        if alt {
            platform.reveal(path);
        } else {
            platform.open(path);
        }
        PopupReply::Close
    }
}

/// The row highlight on a popup background (SPEC §9.19): light blue on dark backgrounds, dark
/// blue on light ones.
fn highlight(background: Color) -> Color {
    let luminance = 0.299 * background.r + 0.587 * background.g + 0.114 * background.b;
    if luminance < 0.5 {
        Color::rgba(0xA9, 0xDF, 0xFF, 0.40)
    } else {
        Color::rgba(0x00, 0x64, 0x9E, 0.55)
    }
}

impl Popup for FolderBrowser {
    fn kind(&self) -> &'static str {
        "folder-browser"
    }

    fn align(&self) -> PopupAlign {
        PopupAlign::LeftEdge
    }

    fn layout(&mut self, cx: &PopupCtx) -> SizeF {
        if self.listing.is_none() {
            let rx = list(
                self.options.folder.clone(),
                self.options.show_hidden,
                Arc::clone(cx.platform),
                cx.waker.clone(),
            );
            self.listing = Some(Listing::Pending { rx, since: cx.now });
            self.receive(FIRST_WAIT);
        }
        let width = self
            .options
            .width
            .clamp(MIN_SIZE, MAX_WIDTH)
            .min(cx.room.width)
            .floor();
        let most = self.options.height.max(MIN_SIZE).min(cx.room.height);
        let count = self.items().len();
        let rows = (count as f32 * ROW).max(ROW);
        let list_height = rows.min((most - LIST_TOP).max(ROW));
        self.view.list_height = list_height;
        self.view.size = SizeF::new(width, LIST_TOP + list_height);
        self.clamp_scroll();

        let style = TextStyle::new(&cx.theme.popup_font, cx.theme.popup_font_size);
        let header = style.clone().max_width((width - 2.0 * END).max(1.0));
        self.view.header = Some(cx.text.layout(&self.options.title, &header));
        let row_style = style.clone().max_width((width - TEXT_X - END).max(1.0));
        let px = icon::physical(ICON, cx.scale);
        let first = (self.scroll / ROW).floor() as usize;
        let last = (((self.scroll + list_height) / ROW).ceil() as usize).min(count);
        let tint = cx.theme.popup_text;
        let rows: Vec<_> = (first..last.max(first))
            .map(|i| {
                let item = &self.items()[i];
                let icon = match cx.icons.shell_icon(&item.path, px) {
                    ShellIcon::Ready(icon) => Some(icon),
                    ShellIcon::Loading => None,
                    ShellIcon::None => Icon::svg(if item.folder { FOLDER } else { FILE }, px)
                        .map(|b| b.tinted(tint)),
                };
                (i, icon, cx.text.layout(&item.name, &row_style))
            })
            .collect();
        self.view.rows = rows;
        let message = match &self.listing {
            Some(Listing::Pending { since, .. })
                if cx.now.saturating_duration_since(*since) >= LOADING_AFTER =>
            {
                Some("folderBrowser.loading")
            }
            Some(Listing::Ready(items)) if items.is_empty() => Some("folderBrowser.empty"),
            Some(Listing::Failed) => Some("folderBrowser.failed"),
            _ => None,
        };
        self.view.message = message.map(|key| cx.text.layout(&t(key), &style));
        self.view.highlight = highlight(cx.theme.popup_background);
        self.view.size
    }

    fn draw(&self, canvas: &mut dyn Canvas, origin: PointF, theme: &Theme) {
        let view = &self.view;
        let width = view.size.width;
        let snap_y = |canvas: &dyn Canvas, top: f32, height: f32, text: &TextLayout| {
            canvas.snap(top + (height - text.height()) / 2.0)
        };
        if self.current == Some(Target::Header) {
            canvas.fill_rect(
                RectF::new(origin.x, origin.y, origin.x + width, origin.y + HEADER),
                view.highlight,
            );
        }
        if let Some(header) = &view.header {
            let y = snap_y(canvas, origin.y, HEADER, header);
            canvas.draw_text(header, PointF::new(origin.x + END, y), theme.popup_text);
        }
        canvas.fill_rect(
            RectF::new(
                origin.x,
                origin.y + HEADER,
                origin.x + width,
                origin.y + HEADER + LINE,
            ),
            theme.popup_strong_text,
        );
        let list = RectF::new(
            origin.x,
            origin.y + LIST_TOP,
            origin.x + width,
            origin.y + LIST_TOP + view.list_height,
        );
        canvas.push_clip(list);
        for (index, icon, name) in &view.rows {
            let top = list.top + *index as f32 * ROW - self.scroll;
            if self.current == Some(Target::Row(*index)) {
                canvas.fill_rect(
                    RectF::new(list.left, top, list.right, top + ROW),
                    view.highlight,
                );
            }
            if let Some(icon) = icon {
                let scale = canvas.scale();
                let (w, h) = (icon.width() as f32 / scale, icon.height() as f32 / scale);
                let x = canvas.snap(list.left + ICON_X);
                let y = canvas.snap(top + (ROW - h) / 2.0);
                canvas.draw_bitmap(icon, RectF::new(x, y, x + w, y + h), 1.0);
            }
            let y = snap_y(canvas, top, ROW, name);
            canvas.draw_text(name, PointF::new(list.left + TEXT_X, y), theme.popup_text);
        }
        if let Some(message) = &view.message {
            let x = canvas.snap(list.left + (width - message.width()) / 2.0);
            let y = snap_y(canvas, list.top, view.list_height, message);
            canvas.draw_text(message, PointF::new(x, y), theme.popup_text);
        }
        canvas.pop();
    }

    fn input(&mut self, input: &PopupInput, cx: &PopupCtx) -> PopupReply {
        match input {
            PopupInput::PointerMove(at) => {
                let target = self.target_at(*at);
                if target.is_some() && target != self.current {
                    self.current = target;
                    return PopupReply::Redraw;
                }
                PopupReply::Nothing
            }
            PopupInput::PointerLeave => {
                self.pressed = None;
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
                modifiers,
            } => match (self.pressed.take(), self.target_at(*at)) {
                (Some(pressed), Some(target)) if pressed == target => {
                    self.activate(target, modifiers.alt, cx.platform.as_ref())
                }
                _ => PopupReply::Nothing,
            },
            PopupInput::Wheel { lines, .. } => {
                let before = self.scroll;
                self.scroll -= lines * ROW;
                self.clamp_scroll();
                if self.scroll == before {
                    PopupReply::Nothing
                } else {
                    PopupReply::Relayout
                }
            }
            PopupInput::Key {
                key: Key::Enter,
                modifiers,
            } => match self.current {
                Some(target) => self.activate(target, modifiers.alt, cx.platform.as_ref()),
                None => PopupReply::Nothing,
            },
            PopupInput::Key { key, .. } => self.select(*key),
            _ => PopupReply::Nothing,
        }
    }

    /// When *Loading…* is due.
    fn wake_at(&self) -> Option<Instant> {
        match &self.listing {
            Some(Listing::Pending { since, .. }) => Some(*since + LOADING_AFTER),
            _ => None,
        }
    }

    fn on_wake(&mut self, _cx: &PopupCtx) -> PopupReply {
        PopupReply::Relayout
    }

    fn poll(&mut self, _cx: &PopupCtx) -> PopupReply {
        if self.receive(Duration::ZERO) {
            PopupReply::Relayout
        } else {
            PopupReply::Nothing
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use super::*;
    use crate::config::model::Settings;
    use crate::platform::Modifiers;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sensors::NoIcons;
    use crate::sensors::test_support::{FakeClock, theme};

    fn entry(name: &str, folder: bool, hidden: bool) -> DirItem {
        DirItem {
            name: name.into(),
            path: format!("C:\\Data\\{name}"),
            folder,
            hidden,
        }
    }

    #[test]
    fn folders_first_in_natural_order_without_link_extensions() {
        let entries = vec![
            entry("file 10.txt", false, false),
            entry("Zeta", true, false),
            entry("file 2.txt", false, false),
            entry("Notepad.lnk", false, false),
            entry("desktop.ini", false, true),
            entry("Desktop.INI", false, false),
            entry("alpha", true, false),
            entry("secret", false, true),
            entry("Site.URL", false, false),
            entry(".lnk", false, false),
        ];
        let names = |items: Vec<Item>| items.into_iter().map(|i| i.name).collect::<Vec<_>>();
        let cmp = crate::platform::natural_cmp;
        assert_eq!(
            names(prepare(entries.clone(), false, cmp)),
            [
                "alpha",
                "Zeta",
                ".lnk",
                "file 2.txt",
                "file 10.txt",
                "Notepad",
                "Site"
            ]
        );
        assert!(names(prepare(entries, true, cmp)).contains(&"secret".to_owned()));
    }

    /// A folder with two subfolders and three files, one of them hidden.
    pub(crate) fn sample_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary folder");
        for folder in ["Projects", "archive"] {
            fs::create_dir(dir.path().join(folder)).expect("a folder");
        }
        for file in ["report 10.pdf", "report 2.pdf", "Editor.lnk", ".hidden"] {
            fs::write(dir.path().join(file), b"x").expect("a file");
        }
        dir
    }

    pub(crate) struct Harness {
        pub platform: Arc<FakeClock>,
        shared: Arc<dyn Platform>,
        theme: Theme,
        settings: Settings,
        waker: UiWaker,
        pub room: SizeF,
    }

    impl Harness {
        pub fn new() -> Self {
            let platform = Arc::new(FakeClock::default());
            Self {
                shared: Arc::clone(&platform) as Arc<dyn Platform>,
                platform,
                theme: theme(),
                settings: Settings::default(),
                waker: UiWaker::new(|| {}),
                room: SizeF::new(1000.0, 1000.0),
            }
        }

        pub fn ctx(&self) -> PopupCtx<'_> {
            PopupCtx {
                theme: &self.theme,
                settings: &self.settings,
                text: &FixedMetrics,
                scale: 1.0,
                platform: &self.shared,
                icons: &NoIcons,
                waker: &self.waker,
                room: self.room,
                now: Instant::now(),
            }
        }
    }

    fn browser(folder: &str) -> FolderBrowser {
        FolderBrowser::new(Options {
            folder: folder.into(),
            title: "Data".into(),
            show_hidden: false,
            width: 240.0,
            height: 320.0,
        })
    }

    fn drawn(browser: &FolderBrowser) -> String {
        let mut canvas = RecordingCanvas::new(1.0);
        browser.draw(&mut canvas, PointF::new(0.0, 0.0), &theme());
        canvas.dump()
    }

    fn texts(dump: &str) -> Vec<String> {
        dump.lines()
            .filter_map(|l| l.split('"').nth(1).map(str::to_owned))
            .collect()
    }

    #[test]
    fn lists_the_folder_under_its_title() {
        let h = Harness::new();
        let dir = sample_folder();
        let mut b = browser(&dir.path().to_string_lossy());
        // Five rows of 20 px under the 28 px header.
        assert_eq!(b.layout(&h.ctx()), SizeF::new(240.0, 128.0));
        assert_eq!(
            texts(&drawn(&b)),
            [
                "Data",
                "archive",
                "Projects",
                "Editor",
                "report 2.pdf",
                "report 10.pdf"
            ]
        );
        let dump = drawn(&b);
        // The 2 px line below the header, in the strong text colour; rows clipped to the list.
        assert!(dump.contains("fill rect 0,24 240×2 #93ECFF"), "{dump}");
        assert!(dump.contains("clip 0,28 240×100"), "{dump}");
        assert_eq!(b.kind(), "folder-browser");
        assert_eq!(b.align(), PopupAlign::LeftEdge);
    }

    #[test]
    fn empty_missing_and_long_folders() {
        let h = Harness::new();
        let dir = tempfile::tempdir().expect("a temporary folder");
        let mut empty = browser(&dir.path().to_string_lossy());
        assert_eq!(empty.layout(&h.ctx()), SizeF::new(240.0, 48.0));
        assert_eq!(texts(&drawn(&empty)), ["Data", "no items"]);
        let mut missing = browser(&dir.path().join("gone").to_string_lossy());
        missing.layout(&h.ctx());
        assert_eq!(
            texts(&drawn(&missing)),
            ["Data", "This folder cannot be opened"]
        );
        // 40 files in a popup at most 320 px tall: the list scrolls.
        for i in 0..40 {
            fs::write(dir.path().join(format!("f{i}")), b"x").expect("a file");
        }
        let mut long = browser(&dir.path().to_string_lossy());
        assert_eq!(long.layout(&h.ctx()), SizeF::new(240.0, 320.0));
        assert_eq!(long.max_scroll(), 40.0 * 20.0 - 292.0);
        let wheel = PopupInput::Wheel {
            lines: -3.0,
            at: PointF::new(10.0, 100.0),
        };
        assert_eq!(long.input(&wheel, &h.ctx()), PopupReply::Relayout);
        assert_eq!(long.scroll, 60.0);
        long.layout(&h.ctx());
        assert_eq!(texts(&drawn(&long))[1], "f3", "scrolled by three rows");
        // Smaller room: smaller popup.
        let h = Harness {
            room: SizeF::new(200.0, 150.0),
            ..Harness::new()
        };
        assert_eq!(long.layout(&h.ctx()), SizeF::new(200.0, 150.0));
    }

    #[test]
    fn hover_click_and_alt_click() {
        let h = Harness::new();
        let dir = sample_folder();
        let mut b = browser(&dir.path().to_string_lossy());
        b.layout(&h.ctx());
        let row = |i: usize| PointF::new(50.0, LIST_TOP + i as f32 * ROW + 5.0);
        assert_eq!(
            b.input(&PopupInput::PointerMove(row(1)), &h.ctx()),
            PopupReply::Redraw
        );
        let dump = drawn(&b);
        assert!(dump.contains("fill rect 0,48 240×20 #A9DFFF/40%"), "{dump}");
        let click = |b: &mut FolderBrowser, at: PointF, alt: bool| {
            let modifiers = Modifiers {
                alt,
                ..Modifiers::default()
            };
            let down = PopupInput::PointerDown {
                button: PointerButton::Left,
                at,
                modifiers,
            };
            b.input(&down, &h.ctx());
            let up = PopupInput::PointerUp {
                button: PointerButton::Left,
                at,
                modifiers,
            };
            b.input(&up, &h.ctx())
        };
        let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        assert_eq!(click(&mut b, row(3), false), PopupReply::Close);
        assert_eq!(click(&mut b, row(4), true), PopupReply::Close);
        assert_eq!(
            click(&mut b, PointF::new(10.0, 10.0), false),
            PopupReply::Close
        );
        assert_eq!(
            h.platform.log(),
            [
                format!("open {}", path("report 2.pdf")),
                format!("reveal {}", path("report 10.pdf")),
                format!("open {}", dir.path().to_string_lossy()),
            ]
        );
        // Below the last row: nothing.
        assert_eq!(click(&mut b, row(7), false), PopupReply::Nothing);
    }

    #[test]
    fn the_keyboard_selects_and_opens() {
        let h = Harness::new();
        let dir = sample_folder();
        let mut b = browser(&dir.path().to_string_lossy());
        b.layout(&h.ctx());
        let key = |key| PopupInput::Key {
            key,
            modifiers: Modifiers::default(),
        };
        assert_eq!(b.input(&key(Key::Down), &h.ctx()), PopupReply::Relayout);
        assert_eq!(b.current, Some(Target::Row(0)));
        b.input(&key(Key::End), &h.ctx());
        b.input(&key(Key::Up), &h.ctx());
        assert_eq!(b.current, Some(Target::Row(3)));
        assert_eq!(b.input(&key(Key::Enter), &h.ctx()), PopupReply::Close);
        let opened = dir.path().join("report 2.pdf");
        assert_eq!(
            h.platform.log(),
            [format!("open {}", opened.to_string_lossy())]
        );
        b.input(&key(Key::Home), &h.ctx());
        assert_eq!(b.current, Some(Target::Row(0)));
    }

    #[test]
    fn a_slow_listing_shows_that_it_is_loading() {
        let h = Harness::new();
        let mut b = browser("C:\\Slow");
        let (tx, rx) = mpsc::channel();
        let long_ago = Instant::now() - Duration::from_secs(1);
        b.listing = Some(Listing::Pending {
            rx,
            since: long_ago,
        });
        assert_eq!(b.wake_at(), Some(long_ago + LOADING_AFTER));
        b.layout(&h.ctx());
        assert_eq!(texts(&drawn(&b)), ["Data", "Loading…"]);
        assert_eq!(b.poll(&h.ctx()), PopupReply::Nothing);
        let item = Item {
            name: "a".into(),
            path: "C:\\Slow\\a".into(),
            folder: false,
        };
        tx.send(Ok(vec![item])).expect("sent");
        assert_eq!(b.poll(&h.ctx()), PopupReply::Relayout);
        b.layout(&h.ctx());
        assert_eq!(texts(&drawn(&b)), ["Data", "a"]);
        assert_eq!(b.wake_at(), None);
    }

    #[test]
    fn highlights_suit_the_background() {
        assert_eq!(
            highlight(Color::rgba(0x20, 0x20, 0x24, 0.92)),
            Color::rgba(0xA9, 0xDF, 0xFF, 0.40)
        );
        assert_eq!(
            highlight(Color::rgba(0xF3, 0xF3, 0xF3, 1.0)),
            Color::rgba(0x00, 0x64, 0x9E, 0.55)
        );
    }
}
