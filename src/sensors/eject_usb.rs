//! `eject-usb` — Eject USB (SPEC §9.22): a USB icon whose click lists the drives that can be
//! ejected (`common/eject_list.rs`), each with an *Eject* button. The `volumes` topic keeps
//! the count in the hint and the open list up to date.

use std::sync::{Arc, Mutex};

use serde::Deserialize;

use super::common::eject_list::{Drives, EjectList, ejectable};
use super::common::icon::{self, Icon};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, ClickResult, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx, SensorType,
    State, typed,
};
use crate::i18n::{t, t_count};
use crate::platform::{Canvas, Capability};
use crate::sampler::{Sample, Topic, TopicRequest, VOLUMES_EVERY};

/// The built-in USB icon, 30×21 (Appendix C).
const USB: &[u8] = include_bytes!("../../assets/icons/usb.svg");
/// The icon's height, logical px (SPEC §9.22).
const ICON: f32 = 21.0;

const SCHEMA: &[PropField] = &[PropField::new(
    "opacity",
    FieldKind::Opacity { min: 20 },
    "sensor.field.opacity",
    Lit::Int(100),
)];

/// SPEC §9.22.
const LEGACY_KEYS: &[LegacyKey] = &[LegacyKey::new("opacity", "opacity")];

pub struct EjectUsbType;

impl SensorType for EjectUsbType {
    fn id(&self) -> &'static str {
        "eject-usb"
    }

    fn category(&self) -> Category {
        Category::Launchers
    }

    fn name(&self) -> &'static str {
        "sensor.ejectUsb.name"
    }

    fn description(&self) -> &'static str {
        "sensor.ejectUsb.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockUSB"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn requires(&self) -> &'static [Capability] {
        &[Capability::Eject]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(EjectUsb {
            props: typed(props)?,
            drives: Arc::default(),
            icon: None,
        }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EjectUsbProps {
    /// Percent.
    opacity: u32,
}

struct EjectUsb {
    props: EjectUsbProps,
    /// The drives that can be ejected, shared with an open list.
    drives: Arc<Mutex<Drives>>,
    icon: Option<Icon>,
}

impl EjectUsb {
    fn count(&self) -> usize {
        self.drives.lock().map_or(0, |d| d.volumes.len())
    }

    /// `Eject USB`, and how many drives there are to eject.
    fn text(&self) -> String {
        match self.count() {
            0 => t("sensor.ejectUsb.name"),
            n => format!(
                "{}\n{}",
                t("sensor.ejectUsb.name"),
                t_count("sensor.ejectUsb.hint.drives", n as u64, &[])
            ),
        }
    }
}

impl Sensor for EjectUsb {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let px = icon::physical(ICON, cx.scale);
        let opacity = self.props.opacity.min(100) as f32 / 100.0;
        self.icon = Icon::svg(USB, px)
            .map(|bitmap| Icon::new(bitmap, Some(cx.theme.sensor_text), opacity, None));
        self.icon
            .as_ref()
            .map_or((ICON * 30.0 / 21.0).ceil(), |i| i.width(cx.scale))
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        if let Some(icon) = &self.icon {
            icon.draw(canvas, cx);
        }
    }

    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        Some(HintContent::Text(self.text()))
    }

    /// Opens the `eject-list` popup (SPEC §9.22).
    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        ClickResult::Popup(Box::new(EjectList::new(
            Arc::clone(&self.drives),
            cx.changes.clone(),
        )))
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        self.text().replace('\n', ", ")
    }

    fn has_popup(&self) -> bool {
        true
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
        let volumes = ejectable(volumes);
        let Ok(mut drives) = self.drives.lock() else {
            return false;
        };
        if drives.volumes == volumes {
            return false;
        }
        drives.volumes = volumes;
        drives.generation += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::platform::{DriveKind, Volume};
    use crate::sensors::test_support::{Harness, draw, measure, resolved};

    fn create(stored: serde_json::Value) -> EjectUsb {
        let props = resolved(SCHEMA, stored);
        EjectUsb {
            props: typed(&props).expect("valid props"),
            drives: Arc::default(),
            icon: None,
        }
    }

    fn with_ctx<R>(f: impl FnOnce(&SensorCtx) -> R) -> R {
        f(&Harness::new().ctx())
    }

    #[test]
    fn a_usb_icon_counting_the_drives() {
        let mut usb = create(json!({ "opacity": 50 }));
        assert_eq!(measure(&mut usb), 30.0);
        assert_eq!(
            draw(&usb, 30.0),
            "bitmap 30×21 into 7,4 30×21 opacity 0.5\n"
        );
        with_ctx(|cx| {
            assert_eq!(usb.hint(cx), Some(HintContent::Text("Eject USB".into())));
            let volumes = vec![
                Volume::new("C:\\", DriveKind::Fixed, ""),
                Volume::new("E:\\", DriveKind::Removable, "STICK"),
            ];
            let sample = Sample::Volumes(volumes.clone());
            assert!(usb.on_sample(&Topic::Volumes, &sample, cx));
            assert!(!usb.on_sample(&Topic::Volumes, &sample, cx), "no change");
            assert_eq!(
                usb.hint(cx),
                Some(HintContent::Text("Eject USB\n1 removable drive".into()))
            );
            let mut two = volumes;
            two.push(Volume::new("D:\\", DriveKind::Optical, "DISC"));
            usb.on_sample(&Topic::Volumes, &Sample::Volumes(two), cx);
            assert_eq!(usb.accessible_value(cx), "Eject USB, 2 removable drives");
            match usb.click(cx) {
                ClickResult::Popup(popup) => assert_eq!(popup.kind(), "eject-list"),
                other => panic!("{other:?}"),
            }
            assert_eq!(usb.drives.lock().expect("lock").generation, 2);
        });
    }
}
