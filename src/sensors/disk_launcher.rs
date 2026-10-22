//! `disk` — opens a drive (SPEC §9.18). The bar shows the drive's shell icon; the `volumes`
//! topic tells whether the drive is there and holds a medium. An absent drive is inactive; an
//! optical or removable drive without a medium stays active and says so. The menu ejects a
//! drive that can be ejected, with the result as a notification.

use serde::Deserialize;
use serde_json::Value;

use super::common::eject;
use super::common::icon::{self, BarIcon, IconSize, IconStyle, Own};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, ClickResult, DefaultsCtx, DrawCtx, HintContent, LayoutCtx, MenuId, MenuItem, Props,
    Sensor, SensorCtx, SensorType, State, typed,
};
use crate::i18n::{t, t_args};
use crate::platform::{Canvas, Volume};
use crate::sampler::{Sample, Topic, TopicRequest, VOLUMES_EVERY};

/// The icon when the shell has none, e.g. for an absent drive (Appendix C).
const DRIVE: &[u8] = include_bytes!("../../assets/icons/drive.svg");
/// The drive when the machine lists none (and the static default).
const FALLBACK_DRIVE: &str = "C:\\";
/// The *Eject* menu item.
const EJECT: MenuId = 0;

const SCHEMA: &[PropField] = &[
    PropField::new(
        "drive",
        FieldKind::Drive {
            hide_optical: false,
        },
        "sensor.disk.field.drive",
        Lit::Str(FALLBACK_DRIVE),
    )
    .machine(),
    PropField::new(
        "title",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.diskLauncher.field.title",
        Lit::Str(""),
    )
    .describe("sensor.diskLauncher.field.title.help"),
    icon::image("sensor.diskLauncher.field.image.help"),
    icon::COLOR,
    icon::OPACITY,
];

/// SPEC §9.18.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("disk", "drive"),
    LegacyKey::new("title", "title"),
    icon::LEGACY_IMAGE,
    icon::LEGACY_COLOR,
    icon::LEGACY_OPACITY,
];

pub struct DiskType;

impl SensorType for DiskType {
    fn id(&self) -> &'static str {
        "disk"
    }

    fn category(&self) -> Category {
        Category::Launchers
    }

    fn name(&self) -> &'static str {
        "sensor.diskLauncher.name"
    }

    fn description(&self) -> &'static str {
        "sensor.diskLauncher.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockDisk"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    /// The first drive (SPEC §9.18).
    fn machine_defaults(&self, ctx: &DefaultsCtx) -> Props {
        let volumes = ctx.platform.volumes();
        let drive = volumes.first().map_or(FALLBACK_DRIVE, |v| v.root.as_str());
        let mut props = Props::new();
        props.insert("drive".into(), Value::from(drive));
        props
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Disk {
            props: typed(props)?,
            volume: None,
            refresh_icon: false,
            icon: BarIcon::default(),
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        props
            .get("drive")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiskProps {
    /// `X:\`.
    drive: String,
    /// Empty: the drive's name.
    title: String,
    /// Its image empty: the drive's shell icon.
    #[serde(flatten)]
    icon: IconStyle,
}

/// What the `volumes` topic says about the drive.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Present(Volume),
    Absent,
}

pub(crate) struct Disk {
    props: DiskProps,
    /// From the `volumes` topic; `None` before its first sample.
    volume: Option<Seen>,
    /// The drive arrived, went, or got or lost a medium: its icon is read again.
    refresh_icon: bool,
    icon: BarIcon,
}

impl Disk {
    fn present(&self) -> Option<&Volume> {
        match &self.volume {
            Some(Seen::Present(volume)) => Some(volume),
            _ => None,
        }
    }

    fn absent(&self) -> bool {
        self.volume == Some(Seen::Absent)
    }

    /// The title, or the drive's name (`Local Disk (C:)`); the drive itself until it is known.
    fn name(&self) -> String {
        if !self.props.title.is_empty() {
            return self.props.title.clone();
        }
        self.present()
            .map_or_else(|| self.props.drive.clone(), super::disk::volume_name)
    }
}

impl Sensor for Disk {
    /// The custom image, or the drive's shell icon (the built-in drive icon when it has none).
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let inactive = self.inactive();
        let own = Own::Shell {
            path: &self.props.drive,
            refresh: &mut self.refresh_icon,
            svg: DRIVE,
        };
        let look = self.props.icon.look(IconSize::Launcher, own);
        self.icon.measure(look.inactive(inactive), cx)
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        self.icon.draw(canvas, cx);
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        let text = if self.absent() {
            t_args(
                "sensor.diskLauncher.hint.absent",
                &[("drive", &self.props.drive)],
            )
        } else if self.present().is_some_and(|v| !v.media) {
            t_args(
                "sensor.diskLauncher.hint.noMedia",
                &[("name", &self.name())],
            )
        } else if cx.clicks() {
            t_args("sensor.diskLauncher.hint.open", &[("name", &self.name())])
        } else {
            self.name()
        };
        Some(HintContent::Text(text))
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        if self.absent() {
            return ClickResult::None;
        }
        cx.platform.open(&self.props.drive);
        ClickResult::Handled
    }

    /// While the drive is absent; a drive without a medium stays active (SPEC §9.18).
    fn inactive(&self) -> bool {
        self.absent()
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.name()
    }

    /// *Eject* for a drive that can be ejected and is there (SPEC §9.18).
    fn menu(&self, _cx: &SensorCtx) -> Vec<MenuItem> {
        if self.present().is_some_and(|v| v.ejectable) {
            vec![MenuItem::new(EJECT, t("sensor.diskLauncher.menu.eject"))]
        } else {
            Vec::new()
        }
    }

    fn on_menu(&mut self, item: MenuId, cx: &SensorCtx) -> bool {
        if item == EJECT {
            let name = self.name();
            cx.platform.eject(
                &self.props.drive,
                Box::new(move |result| eject::notice(&name, result)),
            );
        }
        false
    }

    fn topics(&self) -> Vec<TopicRequest> {
        vec![TopicRequest {
            topic: Topic::Volumes,
            interval: VOLUMES_EVERY,
        }]
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Sample::Volumes(volumes) = sample else {
            return false;
        };
        let seen = volumes
            .iter()
            .find(|v| v.root.eq_ignore_ascii_case(&self.props.drive))
            .map_or(Seen::Absent, |v| Seen::Present(v.clone()));
        if self.volume.as_ref() == Some(&seen) {
            return false;
        }
        // Arrived, gone, or a medium in or out: the shell shows another icon.
        let media = |s: &Seen| matches!(s, Seen::Present(v) if v.media);
        let before = self.volume.as_ref().map(|s| (*s == Seen::Absent, media(s)));
        self.refresh_icon |= before.is_some_and(|b| b != (seen == Seen::Absent, media(&seen)));
        self.volume = Some(seen);
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::model::ClicksWhenLocked;
    use crate::platform::recording::RecordingCanvas;
    use crate::platform::{Bitmap, DriveKind, RectF};
    use crate::sensors::test_support::{Harness, resolved, theme};
    use crate::sensors::{Fetched, IconSource};
    use crate::test_util::FakePlatform;

    fn create(stored: serde_json::Value) -> Disk {
        let props = resolved(SCHEMA, stored);
        Disk {
            props: typed(&props).expect("valid props"),
            volume: None,
            refresh_icon: false,
            icon: BarIcon::default(),
        }
    }

    /// A 24 px icon for `C:\` only; records what it is asked.
    #[derive(Default)]
    struct Icons(std::cell::RefCell<Vec<String>>);

    impl IconSource for Icons {
        fn shell_icon(&self, path: &str, px: u32) -> Fetched {
            self.0.borrow_mut().push(format!("icon {path}"));
            if path == "C:\\" {
                let pixels = vec![255; (px * px * 4) as usize];
                Fetched::Ready(Bitmap::from_rgba(px, px, pixels).expect("a bitmap"))
            } else {
                Fetched::None
            }
        }

        fn forget(&self, path: &str) {
            self.0.borrow_mut().push(format!("forget {path}"));
        }
    }

    fn measure_with(sensor: &mut Disk, icons: &Icons) -> f32 {
        let h = Harness::new();
        sensor.measure(&LayoutCtx {
            icons,
            ..h.layout()
        })
    }

    fn drawn(sensor: &Disk) -> String {
        let mut canvas = RecordingCanvas::new(1.0);
        sensor.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme(),
                rect: RectF::new(7.0, 0.0, 31.0, 28.0),
            },
        );
        canvas.dump()
    }

    fn volumes(list: Vec<Volume>) -> Sample {
        Sample::Volumes(list)
    }

    fn hint(h: &Harness, sensor: &Disk) -> String {
        match sensor.hint(&h.ctx()) {
            Some(HintContent::Text(text)) => text,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_drives_shell_icon_and_name() {
        let h = Harness::new();
        let icons = Icons::default();
        let mut sensor = create(json!({ "drive": "C:\\" }));
        assert_eq!(measure_with(&mut sensor, &icons), 24.0);
        assert_eq!(drawn(&sensor), "bitmap 24×24 into 7,2 24×24 opacity 1\n");
        assert_eq!(
            hint(&h, &sensor),
            "Open 'C:\\'",
            "the drive until the list arrives"
        );
        let c = Volume::new("C:\\", DriveKind::Fixed, "System");
        assert!(sensor.on_sample(&Topic::Volumes, &volumes(vec![c.clone()]), &h.ctx()));
        assert!(!sensor.on_sample(&Topic::Volumes, &volumes(vec![c]), &h.ctx()));
        assert_eq!(hint(&h, &sensor), "Open 'System (C:)'");
        assert_eq!(
            create(json!({ "drive": "C:\\", "title": "Work" })).name(),
            "Work"
        );
    }

    #[test]
    fn absent_drives_are_inactive_and_do_not_open() {
        let h = Harness::new();
        let icons = Icons::default();
        let mut sensor = create(json!({ "drive": "E:\\" }));
        let c = Volume::new("C:\\", DriveKind::Fixed, "");
        sensor.on_sample(&Topic::Volumes, &volumes(vec![c.clone()]), &h.ctx());
        assert!(sensor.inactive());
        assert_eq!(hint(&h, &sensor), "E:\\ is not connected");
        assert_eq!(sensor.click(&h.ctx()), ClickResult::None);
        measure_with(&mut sensor, &icons);
        // The built-in drive icon, faded.
        assert_eq!(drawn(&sensor), "bitmap 24×24 into 7,2 24×24 opacity 0.78\n");

        // Plugged in: active, its icon read again, and a click opens it.
        let e = Volume::new("E:\\", DriveKind::Removable, "STICK");
        assert!(sensor.on_sample(&Topic::Volumes, &volumes(vec![c, e]), &h.ctx()));
        assert!(!sensor.inactive());
        measure_with(&mut sensor, &icons);
        assert!(icons.0.borrow().contains(&"forget E:\\".into()));
        assert_eq!(sensor.click(&h.ctx()), ClickResult::Handled);
        assert_eq!(h.platform.log(), ["open E:\\"]);
    }

    #[test]
    fn a_drive_without_media_stays_active() {
        let mut h = Harness::new();
        let mut sensor = create(json!({ "drive": "D:\\" }));
        let empty = Volume {
            media: false,
            ..Volume::new("D:\\", DriveKind::Optical, "")
        };
        sensor.on_sample(&Topic::Volumes, &volumes(vec![empty]), &h.ctx());
        assert!(!sensor.inactive());
        assert_eq!(hint(&h, &sensor), "CD Drive (D:) — no media");
        // Clicks need Alt: just the name.
        let disc = Volume::new("D:\\", DriveKind::Optical, "MOVIE");
        sensor.on_sample(&Topic::Volumes, &volumes(vec![disc]), &h.ctx());
        assert!(sensor.refresh_icon, "a disc has its own icon");
        h.settings.locked = true;
        h.settings.clicks_when_locked = ClicksWhenLocked::AltOnly;
        assert_eq!(hint(&h, &sensor), "MOVIE (D:)");
    }

    #[test]
    fn ejectable_drives_offer_eject_with_a_notification() {
        let h = Harness::new();
        let mut sensor = create(json!({ "drive": "E:\\" }));
        assert!(sensor.menu(&h.ctx()).is_empty(), "not known yet");
        let stick = Volume::new("E:\\", DriveKind::Removable, "STICK");
        sensor.on_sample(&Topic::Volumes, &volumes(vec![stick]), &h.ctx());
        let menu = sensor.menu(&h.ctx());
        assert_eq!(menu, [MenuItem::new(EJECT, "Eject")]);
        assert!(!sensor.on_menu(EJECT, &h.ctx()));
        assert_eq!(
            h.platform.log(),
            ["eject E:\\: STICK (E:): STICK (E:) can be safely removed."]
        );
        // A fixed internal disk has none.
        let mut fixed = create(json!({ "drive": "C:\\" }));
        let c = Volume::new("C:\\", DriveKind::Fixed, "");
        fixed.on_sample(&Topic::Volumes, &volumes(vec![c]), &h.ctx());
        assert!(fixed.menu(&h.ctx()).is_empty());
    }

    #[test]
    fn machine_default_is_the_first_drive() {
        let drives = FakePlatform::default().with_volumes(vec![Volume::new(
            "A:\\",
            DriveKind::Removable,
            "",
        )]);
        let props = DiskType.machine_defaults(&DefaultsCtx { platform: &drives });
        assert_eq!(props["drive"], json!("A:\\"));
        let none = DiskType.machine_defaults(&DefaultsCtx {
            platform: &crate::test_util::NO_PLATFORM,
        });
        assert_eq!(none["drive"], json!("C:\\"));
    }
}
