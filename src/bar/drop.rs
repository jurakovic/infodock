//! Drops from other apps onto the bar (SPEC §6.3): what each dragged item is, and the sensor a
//! drop creates for it.

use serde_json::Value;

use crate::platform::{DragItem, DropEffect, DropEffects, PathKind};
use crate::sensors::Props;
pub use crate::sensors::drop::Dropped;

/// Whether `path` is a drive's root: `E:\` or `E:`.
pub fn is_drive_root(path: &str) -> bool {
    match path.as_bytes() {
        [letter, b':'] | [letter, b':', b'\\' | b'/'] => letter.is_ascii_alphabetic(),
        _ => false,
    }
}

/// Whether `url` is a web or FTP address (SPEC §6.3).
pub fn is_web(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://", "ftp://"]
        .iter()
        .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
}

/// What `item` is, asking `kind` what a path names; `None` for anything the bar does not take
/// (a missing path, another kind of address).
pub fn classify(item: &DragItem, kind: impl Fn(&str) -> PathKind) -> Option<Dropped> {
    match item {
        DragItem::Path(path) if is_drive_root(path) => {
            let letter = path[..1].to_ascii_uppercase();
            Some(Dropped::Drive(format!("{letter}:\\")))
        }
        DragItem::Path(path) => match kind(path) {
            PathKind::Folder => Some(Dropped::Folder(path.clone())),
            PathKind::File => Some(Dropped::File(path.clone())),
            PathKind::Missing => None,
        },
        DragItem::Url { url, title } if is_web(url) => Some(Dropped::Url {
            url: url.trim().to_owned(),
            title: title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty() && *t != url.trim())
                .map(str::to_owned),
        }),
        DragItem::Url { .. } => None,
    }
}

/// The sensor a drop on the bar creates for `dropped` (SPEC §6.3): its type and props.
pub fn new_sensor(dropped: &Dropped) -> (&'static str, Props) {
    let props = |pairs: &[(&str, &str)]| -> Props {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::from(*v)))
            .collect()
    };
    match dropped {
        Dropped::Drive(root) => ("disk", props(&[("drive", root)])),
        Dropped::Folder(path) => ("folder-menu", props(&[("folder", path)])),
        Dropped::File(path) => ("shortcut", props(&[("target", path)])),
        Dropped::Url { url, title: None } => ("shortcut", props(&[("url", url)])),
        Dropped::Url {
            url,
            title: Some(title),
        } => ("shortcut", props(&[("url", url), ("title", title)])),
    }
}

/// The effect of a drop that creates sensors: a link where the source allows it (the items
/// stay where they are), else a copy; nothing if it allows neither.
pub fn creation_effect(allowed: DropEffects) -> DropEffect {
    if allowed.link {
        DropEffect::Link
    } else if allowed.copy {
        DropEffect::Copy
    } else {
        DropEffect::None
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn kind(path: &str) -> PathKind {
        if path.ends_with(".txt") {
            PathKind::File
        } else if path.contains("Gone") {
            PathKind::Missing
        } else {
            PathKind::Folder
        }
    }

    fn path(p: &str) -> DragItem {
        DragItem::Path(p.into())
    }

    #[test]
    fn drives_folders_files_and_web_addresses() {
        assert_eq!(
            classify(&path("e:\\"), kind),
            Some(Dropped::Drive("E:\\".into()))
        );
        assert_eq!(
            classify(&path("E:"), kind),
            Some(Dropped::Drive("E:\\".into()))
        );
        assert_eq!(
            classify(&path("C:\\Data"), kind),
            Some(Dropped::Folder("C:\\Data".into()))
        );
        assert_eq!(
            classify(&path("C:\\a.txt"), kind),
            Some(Dropped::File("C:\\a.txt".into()))
        );
        assert_eq!(classify(&path("C:\\Gone"), kind), None);
        let url = |url: &str, title: Option<&str>| DragItem::Url {
            url: url.into(),
            title: title.map(str::to_owned),
        };
        assert_eq!(
            classify(&url("https://example.com/", Some(" Example ")), kind),
            Some(Dropped::Url {
                url: "https://example.com/".into(),
                title: Some("Example".into())
            })
        );
        assert_eq!(
            classify(&url("HTTP://a.b/", Some("HTTP://a.b/")), kind),
            Some(Dropped::Url {
                url: "HTTP://a.b/".into(),
                title: None
            }),
            "a title that is the address is none"
        );
        assert!(classify(&url("ftp://files.example.com/", None), kind).is_some());
        assert_eq!(classify(&url("mailto:me@example.com", None), kind), None);
        assert_eq!(classify(&url("https://", None), kind), None);
        assert!(!is_drive_root("C:\\Data"));
        assert!(!is_drive_root("1:\\"));
    }

    #[test]
    fn the_sensor_each_creates() {
        let new = |d: Dropped| {
            let (kind, props) = new_sensor(&d);
            (kind, Value::Object(props))
        };
        assert_eq!(
            new(Dropped::Drive("E:\\".into())),
            ("disk", json!({ "drive": "E:\\" }))
        );
        assert_eq!(
            new(Dropped::Folder("C:\\Data".into())),
            ("folder-menu", json!({ "folder": "C:\\Data" }))
        );
        assert_eq!(
            new(Dropped::File("C:\\a.txt".into())),
            ("shortcut", json!({ "target": "C:\\a.txt" }))
        );
        assert_eq!(
            new(Dropped::Url {
                url: "https://example.com/".into(),
                title: Some("Example".into())
            }),
            (
                "shortcut",
                json!({ "url": "https://example.com/", "title": "Example" })
            )
        );
    }

    #[test]
    fn creating_links_where_allowed() {
        assert_eq!(creation_effect(DropEffects::ALL), DropEffect::Link);
        let copy = DropEffects {
            copy: true,
            ..DropEffects::default()
        };
        assert_eq!(creation_effect(copy), DropEffect::Copy);
        assert_eq!(creation_effect(DropEffects::default()), DropEffect::None);
    }
}
