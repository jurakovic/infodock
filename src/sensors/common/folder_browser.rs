//! The `folder-browser` popup (SPEC §9.19), shared by Folder Menu and Start Menu: a header with
//! the current folder's title over a list of its folders and files. Each folder is listed on a
//! worker thread; rows show a 16 px shell icon and the name. A click (or Enter) opens a file
//! and closes the popup, or goes into a folder (the list slides); Back, ← or Backspace go up
//! again. Alt reveals an item in Explorer; the header opens the current folder itself. Typing
//! jumps to a name, the wheel scrolls smoothly, and grips on the free edges resize the popup.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::icon::{self, Icon};
use crate::bar::theme::Theme;
use crate::config::model::Edge;
use crate::i18n::t;
use crate::platform::{
    Bitmap, Canvas, Color, DirItem, Key, Platform, PointF, PointerButton, PopupCursor, PopupInput,
    RectF, SizeF, TextLayout, TextStyle, UiWaker,
};
use crate::sensors::popup::{Popup, PopupAlign, PopupCtx, PopupReply};
use crate::sensors::{Props, ShellIcon};

/// Row icons when the shell has none, and the Back button (Appendix C).
const FOLDER: &[u8] = include_bytes!("../../../assets/icons/folder.svg");
const FILE: &[u8] = include_bytes!("../../../assets/icons/file.svg");
const BACK: &[u8] = include_bytes!("../../../assets/icons/back.svg");

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
/// The Back button's width at the header's right.
const BACK_WIDTH: f32 = HEADER;
/// The popup's size limits (SPEC §9.19): at least 130 px a side, at most 600 px wide and 70 %
/// of the monitor tall.
pub const MIN_SIZE: f32 = 130.0;
pub const MAX_WIDTH: f32 = 600.0;
const MAX_HEIGHT_SHARE: f32 = 0.7;
/// A grip reaches this far into the content from its edge, and over the bubble's padding
/// outside it.
const GRIP: f32 = 4.0;
/// The first layout of a folder waits this long for its listing, so most folders appear
/// complete; a listing that takes longer than [`LOADING_AFTER`] says it is loading.
const FIRST_WAIT: Duration = Duration::from_millis(150);
const LOADING_AFTER: Duration = Duration::from_millis(200);
/// The list slides this long into a folder and back out (SPEC §9.19).
const SLIDE: Duration = Duration::from_millis(250);
/// A wheel turn scrolls over this long.
const GLIDE: Duration = Duration::from_millis(100);
/// Typed characters within this long make one name to jump to.
const TYPE_AHEAD: Duration = Duration::from_secs(1);
/// The frame interval while something animates.
const FRAME: Duration = Duration::from_millis(16);

/// A folder or file in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// As shown: `.lnk` and `.url` without their extension.
    pub name: String,
    pub path: String,
    pub folder: bool,
    /// More folders merged into this one (Start Menu, SPEC §9.20); listed together with it.
    pub also: Vec<String>,
}

impl Item {
    /// The folders this item lists: its path and those merged into it.
    fn folders(&self) -> Vec<String> {
        std::iter::once(self.path.clone())
            .chain(self.also.iter().cloned())
            .collect()
    }
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
            also: Vec::new(),
        })
        .collect();
    sort(&mut items, cmp);
    items
}

/// Folders first, then files, each in the order of `cmp`.
fn sort(items: &mut [Item], cmp: impl Fn(&str, &str) -> Ordering) {
    items.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| cmp(&a.name, &b.name)));
}

/// Several folders' lists as one (SPEC §9.20): folders with the same name (ignoring case)
/// become one folder holding the union of their children, and of files with the same name
/// the earlier list's wins. Sorted as [`prepare`] sorts.
pub fn merge(lists: Vec<Vec<Item>>, cmp: impl Fn(&str, &str) -> Ordering) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut seen: HashMap<(bool, String), usize> = HashMap::new();
    for item in lists.into_iter().flatten() {
        match seen.entry((item.folder, item.name.to_lowercase())) {
            Entry::Occupied(same) => {
                if item.folder {
                    let same = &mut items[*same.get()];
                    same.also.push(item.path);
                    same.also.extend(item.also);
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(items.len());
                items.push(item);
            }
        }
    }
    sort(&mut items, cmp);
    items
}

/// Lists `folders` on a worker thread as one list, which wakes the bar thread when it is done.
/// Folders that cannot be read are left out, unless none can.
fn list(
    folders: Vec<String>,
    show_hidden: bool,
    platform: Arc<dyn Platform>,
    waker: UiWaker,
) -> Receiver<Result<Vec<Item>, String>> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("folder".into())
        .spawn(move || {
            let cmp = |a: &str, b: &str| platform.compare_names(a, b);
            let mut lists = Vec::new();
            let mut error = None;
            for folder in &folders {
                match platform.list_dir(folder) {
                    Ok(entries) => lists.push(prepare(entries, show_hidden, cmp)),
                    Err(e) => {
                        tracing::info!("folder {folder:?} cannot be listed: {e}");
                        error.get_or_insert(e);
                    }
                }
            }
            let result = match (lists.len(), error) {
                (0, Some(e)) => Err(e),
                (1, _) => Ok(lists.pop().unwrap_or_default()),
                _ => Ok(merge(lists, cmp)),
            };
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

/// `t` (0–1 through an animation) eased out: fast start, slow end.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// How far an animation that started at `start` and lasts `length` is at `now`, 0–1.
fn progress(start: Instant, length: Duration, now: Instant) -> f32 {
    now.saturating_duration_since(start).as_secs_f32() / length.as_secs_f32()
}

enum Listing {
    Pending {
        rx: Receiver<Result<Vec<Item>, String>>,
        since: Instant,
    },
    Ready(Vec<Item>),
    Failed,
}

/// What the browser shows first and how big it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The folder shown first; more folders merge into it (Start Menu, SPEC §9.20).
    pub folders: Vec<String>,
    /// The header's text.
    pub title: String,
    pub show_hidden: bool,
    /// The popup's size, logical px.
    pub width: f32,
    pub height: f32,
}

/// Something the pointer or keyboard can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Header,
    Back,
    Row(usize),
}

/// A resize grip (SPEC §9.19): on the free vertical edge (width) or the edge away from the bar
/// (height).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grip {
    Width,
    Height,
}

/// A grip being dragged: which, and how far from its edge it was taken, logical px.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Resize {
    grip: Grip,
    grab: f32,
}

/// A folder the browser shows, or showed before going into a subfolder.
struct Level {
    /// The folder, and any merged into it; the header opens the first.
    folders: Vec<String>,
    title: String,
    /// `None` until the first layout starts it.
    listing: Option<Listing>,
    /// How far the list is scrolled, logical px.
    scroll: f32,
    /// The hovered or selected target.
    current: Option<Target>,
    /// *Loading…* is shown.
    loading: bool,
}

impl Level {
    fn new(folders: Vec<String>, title: String) -> Self {
        Self {
            folders,
            title,
            listing: None,
            scroll: 0.0,
            current: None,
            loading: false,
        }
    }

    fn items(&self) -> &[Item] {
        match &self.listing {
            Some(Listing::Ready(items)) => items,
            _ => &[],
        }
    }

    /// Takes the listing's result if it is there, waiting at most `wait`; returns whether it
    /// arrived.
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
        self.loading = false;
        true
    }
}

/// A visible row: its index, icon (`None` while the shell icon loads) and name.
type RowView = (usize, Option<Bitmap>, TextLayout);

/// The list sliding away while the next one slides in.
struct Slide {
    start: Instant,
    /// Into a subfolder (the lists move left); back out moves them right.
    forward: bool,
    /// The list going away, as it was drawn.
    rows: Vec<RowView>,
    scroll: f32,
    message: Option<TextLayout>,
    /// How far it has moved, 0–1, eased.
    progress: f32,
}

/// The list scrolling smoothly to `to`.
struct Glide {
    from: f32,
    to: f32,
    start: Instant,
}

/// The laid-out content.
struct View {
    size: SizeF,
    header: Option<TextLayout>,
    /// The Back button's icon, after going into a subfolder.
    back: Option<Bitmap>,
    rows: Vec<RowView>,
    /// *no items*, *Loading…* or that the folder cannot be read.
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
            back: None,
            rows: Vec::new(),
            message: None,
            list_height: 0.0,
            highlight: Color::TRANSPARENT,
        }
    }
}

pub struct FolderBrowser {
    show_hidden: bool,
    /// The popup's size as set, logical px; shown smaller where the room is smaller.
    width: f32,
    height: f32,
    /// The first folder, then each subfolder gone into; the last one shows.
    levels: Vec<Level>,
    /// Where the left button went down.
    pressed: Option<Target>,
    /// The grip under the pointer, and the one being dragged.
    hover_grip: Option<Grip>,
    resize: Option<Resize>,
    /// The size was dragged: it is to be saved.
    resized: bool,
    slide: Option<Slide>,
    glide: Option<Glide>,
    /// Typed so far, and when the last character came.
    typed: String,
    typed_at: Option<Instant>,
    /// The next animation frame.
    frame_at: Option<Instant>,
    view: View,
}

impl FolderBrowser {
    pub fn new(options: Options) -> Self {
        Self {
            show_hidden: options.show_hidden,
            width: options.width,
            height: options.height,
            levels: vec![Level::new(options.folders, options.title)],
            pressed: None,
            hover_grip: None,
            resize: None,
            resized: false,
            slide: None,
            glide: None,
            typed: String::new(),
            typed_at: None,
            frame_at: None,
            view: View::default(),
        }
    }

    /// The current folder's listing has arrived, or failed: for the budget test.
    #[cfg(all(test, windows))]
    pub fn listed(&self) -> bool {
        matches!(
            self.level().listing,
            Some(Listing::Ready(_) | Listing::Failed)
        )
    }

    fn level(&self) -> &Level {
        // Never empty: `back` keeps the first level.
        &self.levels[self.levels.len() - 1]
    }

    fn level_mut(&mut self) -> &mut Level {
        let last = self.levels.len() - 1;
        &mut self.levels[last]
    }

    fn items(&self) -> &[Item] {
        self.level().items()
    }

    /// The most the list may scroll.
    fn max_scroll(&self) -> f32 {
        (self.items().len() as f32 * ROW - self.view.list_height).max(0.0)
    }

    fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        let level = self.level_mut();
        level.scroll = level.scroll.clamp(0.0, max);
    }

    /// The target at `at` (content px).
    fn target_at(&self, at: PointF) -> Option<Target> {
        let width = self.view.size.width;
        if at.x < 0.0 || at.x >= width || at.y < 0.0 {
            return None;
        }
        if at.y < HEADER {
            let back = self.levels.len() > 1 && at.x >= width - BACK_WIDTH;
            return Some(if back { Target::Back } else { Target::Header });
        }
        let y = at.y - LIST_TOP;
        if y < 0.0 || y >= self.view.list_height {
            return None;
        }
        let index = ((y + self.level().scroll) / ROW).floor() as usize;
        (index < self.items().len()).then_some(Target::Row(index))
    }

    /// The grip at `at` (content px): the free vertical edge, or the edge away from the bar.
    fn grip_at(&self, at: PointF, cx: &PopupCtx) -> Option<Grip> {
        let SizeF { width, height } = self.view.size;
        let across = |v: f32, length: f32| (-GRIP..=length + GRIP).contains(&v);
        let near = |v: f32, edge: f32| (edge - GRIP..=edge + GRIP).contains(&v);
        let width_edge = if cx.free_left { 0.0 } else { width };
        let height_edge = match cx.edge {
            Edge::Bottom => 0.0,
            Edge::Top => height,
        };
        if near(at.x, width_edge) && across(at.y, height) {
            Some(Grip::Width)
        } else if near(at.y, height_edge) && across(at.x, width) {
            Some(Grip::Height)
        } else {
            None
        }
    }

    /// The largest the popup may be.
    fn max_size(cx: &PopupCtx) -> SizeF {
        let tallest = if cx.monitor_height > 0.0 {
            (cx.monitor_height * MAX_HEIGHT_SHARE).min(cx.room.height)
        } else {
            cx.room.height
        };
        SizeF::new(MAX_WIDTH.min(cx.room.width), tallest)
    }

    /// Drags the grip to `at` (content px): the free edge follows the pointer.
    fn drag(&mut self, resize: Resize, at: PointF, cx: &PopupCtx) -> PopupReply {
        let SizeF { width, height } = self.view.size;
        let max = Self::max_size(cx);
        let size = match resize.grip {
            Grip::Width if cx.free_left => width - (at.x - resize.grab),
            Grip::Width => at.x - resize.grab,
            Grip::Height if cx.edge == Edge::Bottom => height - (at.y - resize.grab),
            Grip::Height => at.y - resize.grab,
        };
        let (current, limit) = match resize.grip {
            Grip::Width => (&mut self.width, max.width),
            Grip::Height => (&mut self.height, max.height),
        };
        let size = size.min(limit).max(MIN_SIZE).round();
        if size == *current {
            return PopupReply::Nothing;
        }
        *current = size;
        PopupReply::Relayout
    }

    /// Where the pointer took `grip` at `at`, from its edge.
    fn grab(&self, grip: Grip, at: PointF, cx: &PopupCtx) -> f32 {
        let SizeF { width, height } = self.view.size;
        match grip {
            Grip::Width if cx.free_left => at.x,
            Grip::Width => at.x - width,
            Grip::Height if cx.edge == Edge::Bottom => at.y,
            Grip::Height => at.y - height,
        }
    }

    /// Scrolls so row `index` is in view.
    fn reveal_row(&mut self, index: usize) {
        self.glide = None;
        let list_height = self.view.list_height;
        let level = self.level_mut();
        let top = index as f32 * ROW;
        if top < level.scroll {
            level.scroll = top;
        } else if top + ROW > level.scroll + list_height {
            level.scroll = top + ROW - list_height;
        }
        self.clamp_scroll();
    }

    /// Marks row `index` and keeps it in view.
    fn select_row(&mut self, index: usize) -> PopupReply {
        self.level_mut().current = Some(Target::Row(index));
        self.reveal_row(index);
        PopupReply::Relayout
    }

    /// Moves the selection by a key (SPEC §9.19).
    fn select(&mut self, key: Key) -> PopupReply {
        let count = self.items().len();
        if count == 0 {
            return PopupReply::Nothing;
        }
        let page = (self.view.list_height / ROW).floor().max(1.0) as usize;
        let current = match self.level().current {
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
        self.select_row(next)
    }

    /// Jumps to the first name starting with what was typed (SPEC §9.19); a character that
    /// matches nothing with the earlier ones starts a new name.
    fn type_ahead(&mut self, c: char, now: Instant) -> PopupReply {
        if self
            .typed_at
            .is_none_or(|at| now.saturating_duration_since(at) > TYPE_AHEAD)
        {
            self.typed.clear();
        }
        self.typed_at = Some(now);
        self.typed.push(c);
        let find = |prefix: &str| {
            let prefix = prefix.to_lowercase();
            self.items()
                .iter()
                .position(|i| i.name.to_lowercase().starts_with(&prefix))
        };
        let found = match find(&self.typed) {
            Some(i) => Some(i),
            None => {
                let single = c.to_string();
                let i = find(&single);
                self.typed = single;
                i
            }
        };
        match found {
            Some(i) => self.select_row(i),
            None => PopupReply::Nothing,
        }
    }

    /// Starts sliding the shown list away, into a subfolder (`forward`) or back out.
    fn start_slide(&mut self, forward: bool, cx: &PopupCtx) {
        self.glide = None;
        self.slide = cx.animate.then(|| Slide {
            start: cx.now,
            forward,
            rows: std::mem::take(&mut self.view.rows),
            scroll: self.level().scroll,
            message: self.view.message.take(),
            progress: 0.0,
        });
    }

    /// Goes into the folder of row `index`.
    fn enter(&mut self, index: usize, cx: &PopupCtx) -> PopupReply {
        let Some(item) = self.items().get(index).filter(|i| i.folder).cloned() else {
            return PopupReply::Nothing;
        };
        // Coming back, the folder gone into is marked.
        self.level_mut().current = Some(Target::Row(index));
        self.start_slide(true, cx);
        self.levels.push(Level::new(item.folders(), item.name));
        PopupReply::Relayout
    }

    /// Goes back to the folder above, as it was left.
    fn back(&mut self, cx: &PopupCtx) -> PopupReply {
        if self.levels.len() < 2 {
            return PopupReply::Nothing;
        }
        self.start_slide(false, cx);
        self.levels.pop();
        PopupReply::Relayout
    }

    /// Scrolls by `delta` px, gliding there when transitions animate.
    fn scroll_by(&mut self, delta: f32, cx: &PopupCtx) -> PopupReply {
        let max = self.max_scroll();
        let from = self.level().scroll;
        let to = (self.glide.as_ref().map_or(from, |g| g.to) + delta).clamp(0.0, max);
        if to == from {
            return PopupReply::Nothing;
        }
        if cx.animate {
            self.glide = Some(Glide {
                from,
                to,
                start: cx.now,
            });
        } else {
            self.level_mut().scroll = to;
        }
        PopupReply::Relayout
    }

    /// Opens (or with `alt`, reveals) `target`, goes into a folder, or back.
    fn activate(&mut self, target: Target, alt: bool, cx: &PopupCtx) -> PopupReply {
        let path = match target {
            Target::Back => return self.back(cx),
            Target::Header => match self.level().folders.first() {
                Some(folder) => folder.clone(),
                None => return PopupReply::Nothing,
            },
            Target::Row(i) => match self.items().get(i) {
                Some(item) if item.folder && !alt => return self.enter(i, cx),
                Some(item) => item.path.clone(),
                None => return PopupReply::Nothing,
            },
        };
        if alt {
            cx.platform.reveal(&path);
        } else {
            cx.platform.open(&path);
        }
        PopupReply::Close
    }

    /// Moves the animations on to `now`; returns whether one still runs.
    fn animate(&mut self, now: Instant) -> bool {
        if let Some(slide) = &mut self.slide {
            let t = progress(slide.start, SLIDE, now);
            slide.progress = ease_out(t);
            if t >= 1.0 {
                self.slide = None;
            }
        }
        if let Some(glide) = &self.glide {
            let t = progress(glide.start, GLIDE, now);
            let scroll = glide.from + (glide.to - glide.from) * ease_out(t);
            if t >= 1.0 {
                self.glide = None;
            }
            self.level_mut().scroll = scroll;
        }
        self.slide.is_some() || self.glide.is_some()
    }
}

/// The row highlight on a popup background (SPEC §9.19): light blue on dark backgrounds, dark
/// blue on light ones.
pub fn highlight(background: Color) -> Color {
    let luminance = 0.299 * background.r + 0.587 * background.g + 0.114 * background.b;
    if luminance < 0.5 {
        Color::rgba(0xA9, 0xDF, 0xFF, 0.40)
    } else {
        Color::rgba(0x00, 0x64, 0x9E, 0.55)
    }
}

/// Text `text` centred vertically in a band `height` tall from `top`, on whole px.
pub fn centred(canvas: &dyn Canvas, top: f32, height: f32, text: &TextLayout) -> f32 {
    canvas.snap(top + (height - text.height()) / 2.0)
}

/// Draws `rows` of a list scrolled by `scroll`, shifted `dx` sideways (a slide), with row
/// `current` highlighted.
fn draw_rows(
    canvas: &mut dyn Canvas,
    list: RectF,
    rows: &[RowView],
    (scroll, dx): (f32, f32),
    current: Option<usize>,
    (highlight, color): (Color, Color),
) {
    for (index, icon, name) in rows {
        let top = list.top + *index as f32 * ROW - scroll;
        let left = list.left + dx;
        if current == Some(*index) {
            canvas.fill_rect(
                RectF::new(left, top, left + list.width(), top + ROW),
                highlight,
            );
        }
        if let Some(icon) = icon {
            let scale = canvas.scale();
            let (w, h) = (icon.width() as f32 / scale, icon.height() as f32 / scale);
            let x = canvas.snap(left + ICON_X);
            let y = canvas.snap(top + (ROW - h) / 2.0);
            canvas.draw_bitmap(icon, RectF::new(x, y, x + w, y + h), 1.0);
        }
        let y = centred(canvas, top, ROW, name);
        canvas.draw_text(name, PointF::new(canvas.snap(left + TEXT_X), y), color);
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
        if self.level().listing.is_none() {
            let level = self.level();
            let rx = list(
                level.folders.clone(),
                self.show_hidden,
                Arc::clone(cx.platform),
                cx.waker.clone(),
            );
            let level = self.level_mut();
            level.listing = Some(Listing::Pending { rx, since: cx.now });
            level.receive(FIRST_WAIT);
        }
        let max = Self::max_size(cx);
        let width = self.width.clamp(MIN_SIZE, MAX_WIDTH).min(max.width).floor();
        let height = self.height.max(MIN_SIZE).min(max.height).floor();
        let list_height = (height - LIST_TOP).max(ROW);
        self.view.size = SizeF::new(width, LIST_TOP + list_height);
        self.view.list_height = list_height;
        let animating = self.animate(cx.now);
        self.frame_at = animating.then(|| cx.now + FRAME);
        self.clamp_scroll();

        let style = TextStyle::new(&cx.theme.popup_font, cx.theme.popup_font_size);
        let nested = self.levels.len() > 1;
        let back = if nested { BACK_WIDTH } else { 0.0 };
        let header = style.clone().max_width((width - 2.0 * END - back).max(1.0));
        self.view.header = Some(cx.text.layout(&self.level().title, &header));
        let tint = cx.theme.popup_text;
        let px = icon::physical(ICON, cx.scale);
        self.view.back = nested
            .then(|| Icon::svg(BACK, px).map(|b| b.tinted(tint)))
            .flatten();

        let row_style = style.clone().max_width((width - TEXT_X - END).max(1.0));
        let items = self.items();
        let scroll = self.level().scroll;
        let first = (scroll / ROW).floor() as usize;
        let last = (((scroll + list_height) / ROW).ceil() as usize).min(items.len());
        let rows: Vec<RowView> = (first..last.max(first))
            .map(|i| {
                let item = &items[i];
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
        let loading = matches!(
            &self.level().listing,
            Some(Listing::Pending { since, .. })
                if cx.now.saturating_duration_since(*since) >= LOADING_AFTER
        );
        self.level_mut().loading = loading;
        let message = match &self.level().listing {
            Some(Listing::Pending { .. }) if loading => Some("folderBrowser.loading"),
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
        let current = self.level().current;
        let back = view.back.is_some();
        let back_left = origin.x + width - BACK_WIDTH;
        let header_right = if back { back_left } else { origin.x + width };
        let header_band =
            |left: f32, right: f32| RectF::new(left, origin.y, right, origin.y + HEADER);
        match current {
            Some(Target::Header) => {
                canvas.fill_rect(header_band(origin.x, header_right), view.highlight);
            }
            Some(Target::Back) if back => {
                canvas.fill_rect(header_band(back_left, origin.x + width), view.highlight);
            }
            _ => {}
        }
        if let Some(header) = &view.header {
            let y = centred(canvas, origin.y, HEADER, header);
            canvas.draw_text(header, PointF::new(origin.x + END, y), theme.popup_text);
        }
        if let Some(icon) = &view.back {
            let scale = canvas.scale();
            let (w, h) = (icon.width() as f32 / scale, icon.height() as f32 / scale);
            let x = canvas.snap(back_left + (BACK_WIDTH - w) / 2.0);
            let y = canvas.snap(origin.y + (HEADER - h) / 2.0);
            canvas.draw_bitmap(icon, RectF::new(x, y, x + w, y + h), 1.0);
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
        // Sliding: the new list comes in from the side the old one leaves to.
        let (incoming, outgoing) = match &self.slide {
            Some(slide) => {
                let side = if slide.forward { 1.0 } else { -1.0 };
                (
                    side * (1.0 - slide.progress) * width,
                    -side * slide.progress * width,
                )
            }
            None => (0.0, 0.0),
        };
        let colors = (view.highlight, theme.popup_text);
        if let Some(slide) = &self.slide {
            draw_rows(
                canvas,
                list,
                &slide.rows,
                (slide.scroll, outgoing),
                None,
                colors,
            );
            if let Some(message) = &slide.message {
                let x = canvas.snap(list.left + outgoing + (width - message.width()) / 2.0);
                let y = centred(canvas, list.top, view.list_height, message);
                canvas.draw_text(message, PointF::new(x, y), theme.popup_text);
            }
        }
        let row = match current {
            Some(Target::Row(i)) => Some(i),
            _ => None,
        };
        let scroll = self.level().scroll;
        draw_rows(canvas, list, &view.rows, (scroll, incoming), row, colors);
        if let Some(message) = &view.message {
            let x = canvas.snap(list.left + incoming + (width - message.width()) / 2.0);
            let y = centred(canvas, list.top, view.list_height, message);
            canvas.draw_text(message, PointF::new(x, y), theme.popup_text);
        }
        canvas.pop();
    }

    fn input(&mut self, input: &PopupInput, cx: &PopupCtx) -> PopupReply {
        match input {
            PopupInput::PointerMove(at) => {
                if let Some(resize) = self.resize {
                    return self.drag(resize, *at, cx);
                }
                self.hover_grip = self.grip_at(*at, cx);
                let target = self.target_at(*at).filter(|_| self.hover_grip.is_none());
                if target.is_some() && target != self.level().current {
                    self.level_mut().current = target;
                    return PopupReply::Redraw;
                }
                PopupReply::Nothing
            }
            PopupInput::PointerLeave => {
                if self.resize.is_none() {
                    self.hover_grip = None;
                }
                PopupReply::Nothing
            }
            PopupInput::PointerDown {
                button: PointerButton::Left,
                at,
                ..
            } => {
                if let Some(grip) = self.grip_at(*at, cx) {
                    let grab = self.grab(grip, *at, cx);
                    self.resize = Some(Resize { grip, grab });
                    self.pressed = None;
                } else {
                    self.pressed = self.target_at(*at);
                }
                PopupReply::Nothing
            }
            PopupInput::PointerUp {
                button: PointerButton::Left,
                at,
                modifiers,
            } => {
                if self.resize.take().is_some() {
                    self.resized = true;
                    self.hover_grip = self.grip_at(*at, cx);
                    return PopupReply::Nothing;
                }
                match (self.pressed.take(), self.target_at(*at)) {
                    (Some(pressed), Some(target)) if pressed == target => {
                        self.activate(target, modifiers.alt, cx)
                    }
                    _ => PopupReply::Nothing,
                }
            }
            PopupInput::CaptureLost => {
                if self.resize.take().is_some() {
                    self.resized = true;
                }
                PopupReply::Nothing
            }
            PopupInput::Wheel { lines, .. } => self.scroll_by(-lines * ROW, cx),
            PopupInput::Key {
                key: Key::Enter,
                modifiers,
            } => match self.level().current {
                Some(target) => self.activate(target, modifiers.alt, cx),
                None => PopupReply::Nothing,
            },
            PopupInput::Key {
                key: Key::Right, ..
            } => match self.level().current {
                Some(Target::Row(i)) => self.enter(i, cx),
                _ => PopupReply::Nothing,
            },
            PopupInput::Key {
                key: Key::Left | Key::Backspace,
                ..
            } => self.back(cx),
            PopupInput::Key { key, .. } => self.select(*key),
            PopupInput::Char(c) => self.type_ahead(*c, cx.now),
            _ => PopupReply::Nothing,
        }
    }

    /// The next animation frame, or when *Loading…* is due.
    fn wake_at(&self) -> Option<Instant> {
        let level = self.level();
        let loading = match &level.listing {
            Some(Listing::Pending { since, .. }) if !level.loading => Some(*since + LOADING_AFTER),
            _ => None,
        };
        [self.frame_at, loading].into_iter().flatten().min()
    }

    fn on_wake(&mut self, _cx: &PopupCtx) -> PopupReply {
        PopupReply::Relayout
    }

    fn poll(&mut self, _cx: &PopupCtx) -> PopupReply {
        if self.level_mut().receive(Duration::ZERO) {
            PopupReply::Relayout
        } else {
            PopupReply::Nothing
        }
    }

    fn cursor(&self) -> PopupCursor {
        match self.resize.map(|r| r.grip).or(self.hover_grip) {
            Some(Grip::Width) => PopupCursor::ResizeWidth,
            Some(Grip::Height) => PopupCursor::ResizeHeight,
            None => PopupCursor::Arrow,
        }
    }

    fn captures(&self) -> bool {
        self.resize.is_some()
    }

    /// The dragged size, as `popupWidth` and `popupHeight`.
    fn take_props(&mut self) -> Option<Props> {
        if !std::mem::take(&mut self.resized) {
            return None;
        }
        let mut props = Props::new();
        props.insert("popupWidth".into(), Value::from(self.width.round() as i64));
        props.insert(
            "popupHeight".into(),
            Value::from(self.height.round() as i64),
        );
        Some(props)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;
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

    /// A folder with two subfolders (one holding a file) and three files, one of them hidden.
    pub(crate) fn sample_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary folder");
        for folder in ["Projects", "archive"] {
            fs::create_dir(dir.path().join(folder)).expect("a folder");
        }
        fs::write(dir.path().join("Projects").join("plan.txt"), b"x").expect("a file");
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
        pub edge: Edge,
        pub free_left: bool,
        pub animate: bool,
        pub now: Cell<Instant>,
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
                edge: Edge::Bottom,
                free_left: false,
                animate: false,
                now: Cell::new(Instant::now()),
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
                monitor_height: 1080.0,
                edge: self.edge,
                free_left: self.free_left,
                animate: self.animate,
                now: self.now.get(),
            }
        }

        /// Moves the clock on.
        pub fn wait(&self, ms: u64) {
            self.now.set(self.now.get() + Duration::from_millis(ms));
        }
    }

    fn browser(folder: &str) -> FolderBrowser {
        FolderBrowser::new(Options {
            folders: vec![folder.into()],
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

    fn key(key: Key) -> PopupInput {
        PopupInput::Key {
            key,
            modifiers: Modifiers::default(),
        }
    }

    fn row(i: usize) -> PointF {
        PointF::new(50.0, LIST_TOP + i as f32 * ROW + 5.0)
    }

    fn click(b: &mut FolderBrowser, h: &Harness, at: PointF, alt: bool) -> PopupReply {
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
    }

    #[test]
    fn lists_the_folder_under_its_title() {
        let h = Harness::new();
        let dir = sample_folder();
        let mut b = browser(&dir.path().to_string_lossy());
        assert_eq!(b.layout(&h.ctx()), SizeF::new(240.0, 320.0));
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
        assert!(dump.contains("clip 0,28 240×292"), "{dump}");
        assert_eq!(b.kind(), "folder-browser");
        assert_eq!(b.align(), PopupAlign::LeftEdge);
    }

    #[test]
    fn merges_same_named_folders_and_keeps_the_first_file() {
        let item = |name: &str, root: &str, folder| Item {
            name: name.into(),
            path: format!("{root}\\{name}"),
            folder,
            also: Vec::new(),
        };
        let user = vec![item("Tools", "U", true), item("Notes", "U", false)];
        let all = vec![
            item("tools", "A", true),
            item("notes", "A", false),
            item("Accessories", "A", true),
            item("Edge", "A", false),
        ];
        let merged = merge(vec![user, all], crate::platform::natural_cmp);
        let view: Vec<_> = merged
            .iter()
            .map(|i| (i.path.as_str(), i.also.clone()))
            .collect();
        assert_eq!(
            view,
            [
                ("A\\Accessories", vec![]),
                ("U\\Tools", vec!["A\\tools".to_owned()]),
                ("A\\Edge", vec![]),
                ("U\\Notes", vec![]),
            ]
        );
    }

    #[test]
    fn merged_folders_list_their_union() {
        let h = Harness::new();
        let user = tempfile::tempdir().expect("a temporary folder");
        let all = tempfile::tempdir().expect("a temporary folder");
        for root in [&user, &all] {
            fs::create_dir(root.path().join("Tools")).expect("a folder");
        }
        fs::write(user.path().join("Tools").join("Mine.lnk"), b"x").expect("a file");
        fs::write(all.path().join("Tools").join("Shared.lnk"), b"x").expect("a file");
        fs::write(all.path().join("Tools").join("mine.lnk"), b"x").expect("a file");
        fs::write(all.path().join("Edge.lnk"), b"x").expect("a file");
        let path = |dir: &tempfile::TempDir| dir.path().to_string_lossy().into_owned();
        let mut b = FolderBrowser::new(Options {
            folders: vec![path(&user), path(&all), path(&user).replace("tmp", "gone")],
            title: "Start Menu".into(),
            show_hidden: false,
            width: 240.0,
            height: 320.0,
        });
        b.layout(&h.ctx());
        assert_eq!(texts(&drawn(&b)), ["Start Menu", "Tools", "Edge"]);
        assert_eq!(click(&mut b, &h, row(0), false), PopupReply::Relayout);
        b.layout(&h.ctx());
        assert_eq!(texts(&drawn(&b)), ["Tools", "Mine", "Shared"]);
        // The header opens the user's folder.
        assert_eq!(
            click(&mut b, &h, PointF::new(10.0, 10.0), false),
            PopupReply::Close
        );
        assert_eq!(
            h.platform.log(),
            [format!(
                "open {}",
                user.path().join("Tools").to_string_lossy()
            )]
        );
    }

    #[test]
    fn empty_missing_and_long_folders() {
        let h = Harness::new();
        let dir = tempfile::tempdir().expect("a temporary folder");
        let mut empty = browser(&dir.path().to_string_lossy());
        empty.layout(&h.ctx());
        assert_eq!(texts(&drawn(&empty)), ["Data", "no items"]);
        let mut missing = browser(&dir.path().join("gone").to_string_lossy());
        missing.layout(&h.ctx());
        assert_eq!(
            texts(&drawn(&missing)),
            ["Data", "This folder cannot be opened"]
        );
        // 40 files in a popup 320 px tall: the list scrolls.
        for i in 0..40 {
            fs::write(dir.path().join(format!("f{i}")), b"x").expect("a file");
        }
        let mut long = browser(&dir.path().to_string_lossy());
        long.layout(&h.ctx());
        assert_eq!(long.max_scroll(), 40.0 * 20.0 - 292.0);
        let wheel = PopupInput::Wheel {
            lines: -3.0,
            at: PointF::new(10.0, 100.0),
        };
        assert_eq!(long.input(&wheel, &h.ctx()), PopupReply::Relayout);
        assert_eq!(long.level().scroll, 60.0, "without animations: at once");
        long.layout(&h.ctx());
        assert_eq!(texts(&drawn(&long))[1], "f3", "scrolled by three rows");
        // Less room: a smaller popup.
        let h = Harness {
            room: SizeF::new(200.0, 150.0),
            ..Harness::new()
        };
        assert_eq!(long.layout(&h.ctx()), SizeF::new(200.0, 150.0));
    }

    #[test]
    fn the_wheel_scrolls_smoothly() {
        let h = Harness {
            animate: true,
            ..Harness::new()
        };
        let dir = tempfile::tempdir().expect("a temporary folder");
        for i in 0..40 {
            fs::write(dir.path().join(format!("f{i}")), b"x").expect("a file");
        }
        let mut b = browser(&dir.path().to_string_lossy());
        b.layout(&h.ctx());
        let wheel = |lines| PopupInput::Wheel {
            lines,
            at: PointF::new(10.0, 100.0),
        };
        b.input(&wheel(-3.0), &h.ctx());
        b.input(&wheel(-3.0), &h.ctx());
        assert!(
            b.wake_at().is_none(),
            "the first frame comes with the layout"
        );
        h.wait(50);
        b.layout(&h.ctx());
        let halfway = b.level().scroll;
        assert!(halfway > 60.0 && halfway < 120.0, "{halfway}");
        assert!(b.wake_at().is_some(), "another frame");
        h.wait(60);
        b.layout(&h.ctx());
        assert_eq!(b.level().scroll, 120.0, "two turns of three lines");
        assert!(b.wake_at().is_none());
    }

    #[test]
    fn hover_click_and_alt_click() {
        let h = Harness::new();
        let dir = sample_folder();
        let mut b = browser(&dir.path().to_string_lossy());
        b.layout(&h.ctx());
        assert_eq!(
            b.input(&PopupInput::PointerMove(row(1)), &h.ctx()),
            PopupReply::Redraw
        );
        let dump = drawn(&b);
        assert!(dump.contains("fill rect 0,48 240×20 #A9DFFF/40%"), "{dump}");
        let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        assert_eq!(click(&mut b, &h, row(3), false), PopupReply::Close);
        assert_eq!(click(&mut b, &h, row(4), true), PopupReply::Close);
        assert_eq!(
            click(&mut b, &h, row(1), true),
            PopupReply::Close,
            "Alt: reveal"
        );
        assert_eq!(
            click(&mut b, &h, PointF::new(10.0, 10.0), false),
            PopupReply::Close
        );
        assert_eq!(
            h.platform.log(),
            [
                format!("open {}", path("report 2.pdf")),
                format!("reveal {}", path("report 10.pdf")),
                format!("reveal {}", path("Projects")),
                format!("open {}", dir.path().to_string_lossy()),
            ]
        );
        // Below the last row: nothing.
        assert_eq!(click(&mut b, &h, row(7), false), PopupReply::Nothing);
    }

    #[test]
    fn goes_into_folders_and_back_with_a_slide() {
        let h = Harness {
            animate: true,
            ..Harness::new()
        };
        let dir = sample_folder();
        let mut b = browser(&dir.path().to_string_lossy());
        b.layout(&h.ctx());
        b.input(&key(Key::Down), &h.ctx());
        // A click on Projects goes into it; the lists slide left.
        assert_eq!(click(&mut b, &h, row(1), false), PopupReply::Relayout);
        b.layout(&h.ctx());
        let dump = drawn(&b);
        assert_eq!(
            texts(&dump),
            [
                "Projects",
                "archive",
                "Projects",
                "Editor",
                "report 2.pdf",
                "report 10.pdf",
                "plan.txt"
            ]
        );
        assert!(b.view.back.is_some(), "a Back button");
        h.wait(125);
        b.layout(&h.ctx());
        assert!(
            b.slide.as_ref().is_some_and(|s| s.progress > 0.5),
            "eased out"
        );
        h.wait(150);
        b.layout(&h.ctx());
        assert!(b.slide.is_none(), "done after 250 ms");
        assert_eq!(texts(&drawn(&b)), ["Projects", "plan.txt"]);
        // Back (Backspace): the folder above, as it was left.
        assert_eq!(
            b.input(&key(Key::Backspace), &h.ctx()),
            PopupReply::Relayout
        );
        h.wait(300);
        b.layout(&h.ctx());
        assert_eq!(texts(&drawn(&b))[0], "Data");
        assert_eq!(
            b.level().current,
            Some(Target::Row(1)),
            "Projects still marked"
        );
        // → goes in, ← comes back; the Back button too.
        assert_eq!(b.input(&key(Key::Right), &h.ctx()), PopupReply::Relayout);
        h.wait(300);
        b.layout(&h.ctx());
        assert_eq!(b.level().title, "Projects");
        assert_eq!(b.input(&key(Key::Left), &h.ctx()), PopupReply::Relayout);
        assert_eq!(
            b.input(&key(Key::Left), &h.ctx()),
            PopupReply::Nothing,
            "at the top"
        );
        b.input(&key(Key::Right), &h.ctx());
        h.wait(300);
        b.layout(&h.ctx());
        let back = PointF::new(240.0 - 10.0, 10.0);
        assert_eq!(click(&mut b, &h, back, false), PopupReply::Relayout);
        assert_eq!(b.level().title, "Data");
        // A file row does not go anywhere with →.
        b.input(&key(Key::End), &h.ctx());
        assert_eq!(b.input(&key(Key::Right), &h.ctx()), PopupReply::Nothing);
    }

    #[test]
    fn the_keyboard_selects_opens_and_jumps_to_names() {
        let h = Harness::new();
        let dir = sample_folder();
        let mut b = browser(&dir.path().to_string_lossy());
        b.layout(&h.ctx());
        assert_eq!(b.input(&key(Key::Down), &h.ctx()), PopupReply::Relayout);
        assert_eq!(b.level().current, Some(Target::Row(0)));
        b.input(&key(Key::End), &h.ctx());
        b.input(&key(Key::Up), &h.ctx());
        assert_eq!(b.level().current, Some(Target::Row(3)));
        // Typing jumps: "E" to Editor, then "re" (a new name, since "Er" matches nothing) to
        // the first report; after a pause "a" to archive.
        b.input(&PopupInput::Char('E'), &h.ctx());
        assert_eq!(b.level().current, Some(Target::Row(2)));
        b.input(&PopupInput::Char('r'), &h.ctx());
        b.input(&PopupInput::Char('e'), &h.ctx());
        assert_eq!(b.level().current, Some(Target::Row(3)));
        h.wait(1100);
        b.input(&PopupInput::Char('a'), &h.ctx());
        assert_eq!(b.level().current, Some(Target::Row(0)));
        b.input(&key(Key::End), &h.ctx());
        assert_eq!(b.input(&key(Key::Enter), &h.ctx()), PopupReply::Close);
        let opened = dir.path().join("report 10.pdf");
        assert_eq!(
            h.platform.log(),
            [format!("open {}", opened.to_string_lossy())]
        );
    }

    #[test]
    fn grips_resize_and_the_size_is_saved() {
        let h = Harness::new();
        let dir = sample_folder();
        let mut b = browser(&dir.path().to_string_lossy());
        b.layout(&h.ctx());
        assert_eq!(b.cursor(), PopupCursor::Arrow);
        let press = |b: &mut FolderBrowser, h: &Harness, at: PointF| {
            b.input(
                &PopupInput::PointerDown {
                    button: PointerButton::Left,
                    at,
                    modifiers: Modifiers::default(),
                },
                &h.ctx(),
            )
        };
        // Above a bottom bar, lined up on the left: the right edge and the top are free.
        let right = PointF::new(239.0, 100.0);
        b.input(&PopupInput::PointerMove(right), &h.ctx());
        assert_eq!(b.cursor(), PopupCursor::ResizeWidth);
        press(&mut b, &h, right);
        assert!(b.captures());
        let moved = PopupInput::PointerMove(PointF::new(289.0, 100.0));
        assert_eq!(b.input(&moved, &h.ctx()), PopupReply::Relayout);
        assert_eq!(b.layout(&h.ctx()).width, 290.0);
        // Far beyond the limit: at most 600 px.
        b.input(
            &PopupInput::PointerMove(PointF::new(2000.0, 100.0)),
            &h.ctx(),
        );
        assert_eq!(b.layout(&h.ctx()).width, 600.0);
        b.input(
            &PopupInput::PointerUp {
                button: PointerButton::Left,
                at: PointF::new(2000.0, 100.0),
                modifiers: Modifiers::default(),
            },
            &h.ctx(),
        );
        assert!(!b.captures());
        // The top edge: 50 px up makes it 50 px taller.
        press(&mut b, &h, PointF::new(100.0, 1.0));
        assert_eq!(b.cursor(), PopupCursor::ResizeHeight);
        b.input(
            &PopupInput::PointerMove(PointF::new(100.0, -49.0)),
            &h.ctx(),
        );
        b.input(&PopupInput::CaptureLost, &h.ctx());
        assert_eq!(b.layout(&h.ctx()).height, 370.0);
        let saved = b.take_props().expect("the size is saved");
        assert_eq!(
            Value::Object(saved),
            serde_json::json!({ "popupWidth": 600, "popupHeight": 370 })
        );
        assert_eq!(b.take_props(), None, "taken once");
        // Below a top bar, lined up on the right: the left edge and the bottom are free.
        let h = Harness {
            edge: Edge::Top,
            free_left: true,
            ..Harness::new()
        };
        let mut b = browser(&dir.path().to_string_lossy());
        b.layout(&h.ctx());
        assert_eq!(
            b.grip_at(PointF::new(1.0, 100.0), &h.ctx()),
            Some(Grip::Width)
        );
        assert_eq!(b.grip_at(PointF::new(239.0, 100.0), &h.ctx()), None);
        assert_eq!(
            b.grip_at(PointF::new(100.0, 319.0), &h.ctx()),
            Some(Grip::Height)
        );
        // Never smaller than 130 px.
        press(&mut b, &h, PointF::new(1.0, 100.0));
        b.input(
            &PopupInput::PointerMove(PointF::new(500.0, 100.0)),
            &h.ctx(),
        );
        assert_eq!(b.layout(&h.ctx()).width, 130.0);
    }

    #[test]
    fn a_slow_listing_shows_that_it_is_loading() {
        let h = Harness::new();
        let mut b = browser("C:\\Slow");
        let (tx, rx) = mpsc::channel();
        let since = h.now.get();
        b.level_mut().listing = Some(Listing::Pending { rx, since });
        assert_eq!(b.wake_at(), Some(since + LOADING_AFTER));
        h.wait(300);
        b.layout(&h.ctx());
        assert_eq!(texts(&drawn(&b)), ["Data", "Loading…"]);
        assert_eq!(b.wake_at(), None, "shown: no more wake-ups for it");
        assert_eq!(b.poll(&h.ctx()), PopupReply::Nothing);
        let item = Item {
            name: "a".into(),
            path: "C:\\Slow\\a".into(),
            folder: false,
            also: Vec::new(),
        };
        tx.send(Ok(vec![item])).expect("sent");
        assert_eq!(b.poll(&h.ctx()), PopupReply::Relayout);
        b.layout(&h.ctx());
        assert_eq!(texts(&drawn(&b)), ["Data", "a"]);
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
