//! `recycle-bin` — what the Recycle Bin holds (SPEC §9.16): an empty or full icon with the
//! title above the item count. A click opens the Recycle Bin; the menu opens or empties it, and
//! emptying makes the sampler read the `recycle-bin` topic again at once.

use std::time::Duration;

use serde::Deserialize;

use super::common::icon::{self, Icon};
use super::common::icon_label::IconLabel;
use super::common::two_line::TwoLine;
use super::drop::{self, DragInfo, DropAction, Dropped};
use super::format;
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Capability, Category, ClickResult, ColorChoice, DrawCtx, Fetched, HintContent, LayoutCtx,
    MenuId, MenuItem, Props, Sensor, SensorCtx, SensorType, State, typed,
};
use crate::i18n::{t, t_count};
use crate::platform::{Canvas, DropEffect, RecycleBin};
use crate::sampler::{Change, Sample, Topic, TopicRequest};

/// The built-in icons (Appendix C).
const EMPTY: &[u8] = include_bytes!("../../assets/icons/recycle-empty.svg");
const FULL: &[u8] = include_bytes!("../../assets/icons/recycle-full.svg");
/// What a click and *Open Recycle Bin* open.
const FOLDER: &str = "shell:RecycleBinFolder";
/// The menu items.
const OPEN: MenuId = 0;
const EMPTY_BIN: MenuId = 1;

const IMAGES: &[&str] = &["ico", "png", "gif", "jpg", "jpeg", "svg"];

const SCHEMA: &[PropField] = &[
    PropField::new(
        "title",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.recycleBin.field.title",
        Lit::Str(""),
    )
    .describe("sensor.recycleBin.field.title.help"),
    PropField::new(
        "imageEmpty",
        FieldKind::File { extensions: IMAGES },
        "sensor.recycleBin.field.imageEmpty",
        Lit::Str(""),
    )
    .describe("sensor.recycleBin.field.image.help"),
    PropField::new(
        "imageFull",
        FieldKind::File { extensions: IMAGES },
        "sensor.recycleBin.field.imageFull",
        Lit::Str(""),
    )
    .describe("sensor.recycleBin.field.image.help"),
    PropField::new(
        "color",
        FieldKind::ColorMode { dynamic: false },
        "sensor.field.color",
        Lit::ColorMode("standard", "#FFFFFF"),
    )
    .describe("sensor.shortcut.field.color.help"),
    PropField::new(
        "opacity",
        FieldKind::Opacity { min: 20 },
        "sensor.field.opacity",
        Lit::Int(100),
    ),
    PropField::new(
        "interval",
        FieldKind::Interval {
            min: 5.0,
            max: 120.0,
        },
        "sensor.field.interval",
        Lit::Int(10),
    ),
];

/// SPEC §9.16.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("title", "title"),
    LegacyKey::new("image-empty", "imageEmpty"),
    LegacyKey::new("image-full", "imageFull"),
    LegacyKey::new("opacity", "opacity"),
    LegacyKey::new("colorize", "color"),
];

pub struct RecycleBinType;

impl SensorType for RecycleBinType {
    fn id(&self) -> &'static str {
        "recycle-bin"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn name(&self) -> &'static str {
        "sensor.recycleBin.name"
    }

    fn description(&self) -> &'static str {
        "sensor.recycleBin.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockTrashMeter"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn requires(&self) -> &'static [Capability] {
        &[Capability::RecycleBin]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Bin {
            props: typed(props)?,
            bin: None,
            view: None,
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        let title = props.get("title").and_then(|v| v.as_str())?;
        (!title.is_empty()).then(|| title.to_owned())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinProps {
    /// Empty: *Recycle Bin*.
    title: String,
    /// Empty: the built-in icons.
    image_empty: String,
    image_full: String,
    /// Custom tints the icon (SPEC §8.4).
    color: ColorChoice,
    /// The icon's, percent.
    opacity: u32,
    /// Seconds.
    interval: u64,
}

struct Bin {
    props: BinProps,
    /// From the `recycle-bin` topic; `None` before its first sample.
    bin: Option<RecycleBin>,
    view: Option<IconLabel>,
}

impl Bin {
    fn title(&self) -> String {
        if self.props.title.is_empty() {
            t("sensor.recycleBin.title")
        } else {
            self.props.title.clone()
        }
    }

    /// `empty`, `1 item`, `12 items`; nothing before the first sample.
    fn value(&self) -> String {
        match self.bin {
            None => String::new(),
            Some(bin) if bin.items == 0 => t("sensor.recycleBin.empty"),
            Some(bin) => t_count("common.items", bin.items, &[]),
        }
    }

    fn full(&self) -> bool {
        self.bin.is_some_and(|b| b.items > 0)
    }
}

impl Sensor for Bin {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        // As the launchers' icons: bar height − 4, centred.
        let logical = cx.theme.bar_height.saturating_sub(4).max(1) as f32;
        let px = icon::physical(logical, cx.scale);
        let (path, svg) = if self.full() {
            (&self.props.image_full, FULL)
        } else {
            (&self.props.image_empty, EMPTY)
        };
        let bitmap = match icon::custom_image(path, px, cx.icons) {
            Fetched::Ready(image) => Some((image, false)),
            // Its square stays free until it arrives.
            Fetched::Loading => None,
            Fetched::None => Icon::svg(svg, px).map(|b| (b, true)),
        };
        let opacity = self.props.opacity.min(100) as f32 / 100.0;
        let icon = bitmap.map(|(bitmap, built_in)| {
            Icon::new(
                bitmap,
                icon::tint(&self.props.color, built_in, cx.theme),
                opacity,
                None,
            )
        });
        let icon_width = icon
            .as_ref()
            .map_or((px as f32 / cx.scale).ceil(), |i| i.width(cx.scale));
        let text = TwoLine::new(cx.text, cx.theme, &self.title(), &self.value());
        let view = IconLabel::new(icon, icon_width, text);
        let width = view.width();
        self.view = Some(view);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        if let Some(view) = &self.view {
            view.draw(canvas, cx, cx.theme.sensor_text);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        let mut text = self.title();
        if let Some(bin) = self.bin {
            let numbers = format::Numbers::from(&cx.platform.locale(cx.settings.date_locale));
            let size = format::bytes(bin.bytes, None, numbers);
            for (label, value) in [
                ("sensor.recycleBin.hint.items", bin.items.to_string()),
                ("sensor.recycleBin.hint.size", size),
            ] {
                text.push_str(&format!("\n{}\t{value}", t(label)));
            }
        }
        Some(HintContent::Text(
            cx.with_click_line(text, "sensor.recycleBin.hint.click"),
        ))
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open(FOLDER);
        ClickResult::Handled
    }

    /// *Open Recycle Bin* and *Empty Recycle Bin*, the latter greyed out while it is empty
    /// (SPEC §9.16).
    fn menu(&self, _cx: &SensorCtx) -> Vec<MenuItem> {
        let empty = self.bin.is_some_and(|b| b.items == 0);
        vec![
            MenuItem::new(OPEN, t("sensor.recycleBin.menu.open")),
            MenuItem::new(EMPTY_BIN, t("sensor.recycleBin.menu.empty")).enabled(!empty),
        ]
    }

    fn on_menu(&mut self, item: MenuId, cx: &SensorCtx) -> bool {
        match item {
            OPEN => cx.platform.open(FOLDER),
            EMPTY_BIN => {
                // Read again once the shell is done, rather than at the next interval.
                let changes = cx.changes.clone();
                cx.platform
                    .empty_recycle_bin(Box::new(move || changes.report(Change::RecycleBin)));
            }
            _ => {}
        }
        false
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.value()
    }

    /// Files and folders go to the Recycle Bin (SPEC §9.16); anything else is left to the bar.
    fn drop_action(
        &self,
        items: &[Dropped],
        drag: DragInfo,
        _cx: &SensorCtx,
    ) -> Option<DropAction> {
        if items.is_empty() || items.iter().any(|i| i.path().is_none()) {
            return None;
        }
        Some(if drag.allowed.moves {
            DropAction {
                effect: DropEffect::Move,
                hint: Some(drop::hint("drop.delete", items, "")),
            }
        } else {
            DropAction::REFUSED
        })
    }

    fn drop_items(&mut self, items: &[Dropped], drag: DragInfo, cx: &SensorCtx) {
        let paths: Option<Vec<String>> =
            items.iter().map(|i| i.path().map(str::to_owned)).collect();
        let Some(paths) = paths.filter(|p| !p.is_empty() && drag.allowed.moves) else {
            return;
        };
        // Read again once the shell is done, rather than at the next interval.
        let changes = cx.changes.clone();
        cx.platform
            .recycle(paths, Box::new(move || changes.report(Change::RecycleBin)));
    }

    fn topics(&self) -> Vec<TopicRequest> {
        vec![TopicRequest {
            topic: Topic::RecycleBin,
            interval: Duration::from_secs(self.props.interval.max(1)),
        }]
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Sample::RecycleBin(bin) = sample else {
            return false;
        };
        if self.bin == Some(*bin) {
            return false;
        }
        self.bin = Some(*bin);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::config::model::{ClicksWhenLocked, Settings};
    use crate::platform::Platform;
    use crate::sampler::ChangeSink;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, draw, measure};

    fn create(stored: serde_json::Value) -> Bin {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        Bin {
            props: typed(&props).expect("valid props"),
            bin: None,
            view: None,
        }
    }

    /// A platform that records what it opens and empties the bin at once.
    #[derive(Default)]
    struct Shell {
        clock: FakeClock,
        emptied: Mutex<u32>,
    }

    impl Platform for Shell {
        fn capabilities(&self) -> crate::platform::Capabilities {
            Default::default()
        }
        fn monitors(&self) -> Vec<crate::platform::MonitorInfo> {
            Vec::new()
        }
        fn appearance(&self) -> crate::platform::OsAppearance {
            Default::default()
        }
        fn uptime(&self) -> Duration {
            Duration::ZERO
        }
        fn open(&self, target: &str) {
            self.clock.open(target);
        }
        fn empty_recycle_bin(&self, done: Box<dyn FnOnce() + Send>) {
            *self.emptied.lock().expect("lock") += 1;
            done();
        }
        fn recycle(&self, paths: Vec<String>, done: Box<dyn FnOnce() + Send>) {
            self.clock.recycle(paths, done);
        }
    }

    struct Harness {
        platform: Shell,
        settings: Settings,
        changes: ChangeSink,
        reported: Arc<Mutex<Vec<Change>>>,
    }

    impl Harness {
        fn new() -> Self {
            let reported = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&reported);
            Self {
                platform: Shell::default(),
                settings: Settings::default(),
                changes: ChangeSink::new(move |c| sink.lock().expect("lock").push(c)),
                reported,
            }
        }

        fn ctx(&self) -> SensorCtx<'_> {
            SensorCtx {
                settings: &self.settings,
                locked: self.settings.locked,
                alt: false,
                animate: false,
                platform: &self.platform as &dyn Platform,
                changes: &self.changes,
            }
        }

        fn hint(&self, sensor: &Bin) -> String {
            match sensor.hint(&self.ctx()) {
                Some(HintContent::Text(text)) => text,
                other => panic!("{other:?}"),
            }
        }
    }

    fn sample(items: u64, bytes: u64) -> Sample {
        Sample::RecycleBin(RecycleBin { items, bytes })
    }

    #[test]
    fn counts_items_under_the_title() {
        let h = Harness::new();
        let mut bin = create(json!({}));
        // Before the first sample: the title over an empty line.
        assert_eq!(measure(&mut bin), (24.0 + 3.0 + 55.0_f32).ceil());
        assert_eq!(bin.accessible_value(&h.ctx()), "");
        assert!(bin.on_sample(&Topic::RecycleBin, &sample(0, 0), &h.ctx()));
        assert!(!bin.on_sample(&Topic::RecycleBin, &sample(0, 0), &h.ctx()));
        assert_eq!(bin.value(), "empty");
        assert!(bin.on_sample(&Topic::RecycleBin, &sample(1, 10), &h.ctx()));
        assert_eq!(bin.value(), "1 item");
        bin.on_sample(&Topic::RecycleBin, &sample(12, 10), &h.ctx());
        assert_eq!(bin.value(), "12 items");
        measure(&mut bin);
        let dump = draw(&bin, 82.0);
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines[0], "bitmap 24×24 into 7,2 24×24 opacity 1", "{dump}");
        assert!(lines[1].contains("\"Recycle Bin\""), "{dump}");
        assert!(lines[2].contains("\"12 items\""), "{dump}");
        assert_eq!(
            create(json!({ "title": "Trash" })).title(),
            "Trash",
            "a title of its own"
        );
    }

    #[test]
    fn the_icon_shows_whether_it_is_full() {
        let h = Harness::new();
        let pixels = |bin: &Bin| {
            let icon = bin
                .view
                .as_ref()
                .and_then(IconLabel::icon)
                .expect("an icon");
            icon.bitmap().pixels().to_vec()
        };
        let mut bin = create(json!({ "opacity": 50 }));
        measure(&mut bin);
        let empty = pixels(&bin);
        assert!(draw(&bin, 82.0).starts_with("bitmap 24×24 into 7,2 24×24 opacity 0.5"));
        bin.on_sample(&Topic::RecycleBin, &sample(3, 10), &h.ctx());
        measure(&mut bin);
        assert_ne!(pixels(&bin), empty);
        bin.on_sample(&Topic::RecycleBin, &sample(0, 0), &h.ctx());
        measure(&mut bin);
        assert_eq!(pixels(&bin), empty);
    }

    #[test]
    fn hint_lists_items_and_size() {
        let mut h = Harness::new();
        let mut bin = create(json!({}));
        assert_eq!(h.hint(&bin), "Recycle Bin\nClick to open the Recycle Bin");
        bin.on_sample(&Topic::RecycleBin, &sample(12, 431 << 20), &h.ctx());
        assert_eq!(
            h.hint(&bin),
            "Recycle Bin\nItems:\t12\nSpace used:\t431 MB\nClick to open the Recycle Bin"
        );
        h.settings.locked = true;
        h.settings.clicks_when_locked = ClicksWhenLocked::AltOnly;
        assert_eq!(h.hint(&bin), "Recycle Bin\nItems:\t12\nSpace used:\t431 MB");
    }

    #[test]
    fn opens_and_empties_and_asks_for_a_new_reading() {
        let h = Harness::new();
        let mut bin = create(json!({}));
        assert_eq!(bin.click(&h.ctx()), ClickResult::Handled);
        let menu = bin.menu(&h.ctx());
        assert_eq!(
            menu,
            [
                MenuItem::new(OPEN, "Open Recycle Bin"),
                MenuItem::new(EMPTY_BIN, "Empty Recycle Bin"),
            ]
        );
        assert!(!bin.on_menu(OPEN, &h.ctx()));
        assert_eq!(
            h.platform.clock.log(),
            ["open shell:RecycleBinFolder", "open shell:RecycleBinFolder"]
        );
        assert!(!bin.on_menu(EMPTY_BIN, &h.ctx()));
        assert_eq!(*h.platform.emptied.lock().expect("lock"), 1);
        assert_eq!(*h.reported.lock().expect("lock"), [Change::RecycleBin]);
        // Known to be empty: nothing to empty.
        bin.on_sample(&Topic::RecycleBin, &sample(0, 0), &h.ctx());
        assert!(!bin.menu(&h.ctx())[1].enabled);
    }

    #[test]
    fn dropped_files_go_to_the_bin() {
        use crate::platform::{DropEffects, Modifiers};
        let h = Harness::new();
        let mut bin = create(json!({}));
        let all = DragInfo {
            modifiers: Modifiers::default(),
            allowed: DropEffects::ALL,
        };
        let files = [
            Dropped::Path("C:\\a.txt".into()),
            Dropped::Path("C:\\Old".into()),
        ];
        assert_eq!(
            bin.drop_action(&files, all, &h.ctx()),
            Some(DropAction {
                effect: DropEffect::Move,
                hint: Some("Delete 2 items".into())
            })
        );
        let copy_only = DragInfo {
            allowed: DropEffects {
                copy: true,
                ..DropEffects::default()
            },
            ..all
        };
        assert_eq!(
            bin.drop_action(&files, copy_only, &h.ctx()),
            Some(DropAction::REFUSED)
        );
        let link = [Dropped::Url {
            url: "https://example.com/".into(),
            title: None,
        }];
        assert_eq!(bin.drop_action(&link, all, &h.ctx()), None, "a new sensor");
        bin.drop_items(&files, all, &h.ctx());
        assert_eq!(h.platform.clock.log(), ["recycle C:\\a.txt, C:\\Old"]);
        assert_eq!(*h.reported.lock().expect("lock"), [Change::RecycleBin]);
    }

    #[test]
    fn reads_at_its_interval() {
        let bin = create(json!({ "interval": 30 }));
        assert_eq!(
            bin.topics(),
            [TopicRequest {
                topic: Topic::RecycleBin,
                interval: Duration::from_secs(30),
            }]
        );
        assert_eq!(
            RecycleBinType.requires(),
            [Capability::RecycleBin],
            "hidden where there is no Recycle Bin"
        );
    }
}
