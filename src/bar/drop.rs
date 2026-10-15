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

/// Whether `url` is a web or FTP address (SPEC §6.3), without control characters, which
/// could add lines to the `.url` file it may become.
pub fn is_web(url: &str) -> bool {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    !url.chars().any(char::is_control)
        && ["http://", "https://", "ftp://"]
            .iter()
            .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
}

/// What `item` is by its form (no drive is touched); `None` for anything the bar does not take
/// (another kind of address).
pub fn classify(item: &DragItem) -> Option<Dropped> {
    match item {
        DragItem::Path(path) if is_drive_root(path) => {
            let letter = path[..1].to_ascii_uppercase();
            Some(Dropped::Drive(format!("{letter}:\\")))
        }
        DragItem::Path(path) => Some(Dropped::Path(path.clone())),
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

/// The sensor a drop on the bar creates for `dropped` (SPEC §6.3): its type and props;
/// `kind` says what a path names, and a missing one makes nothing.
pub fn new_sensor(
    dropped: &Dropped,
    kind: impl Fn(&str) -> PathKind,
) -> Option<(&'static str, Props)> {
    let props = |pairs: &[(&str, &str)]| -> Props {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::from(*v)))
            .collect()
    };
    Some(match dropped {
        Dropped::Drive(root) => ("disk", props(&[("drive", root)])),
        Dropped::Path(path) => match kind(path) {
            PathKind::Folder => ("folder-menu", props(&[("folder", path)])),
            PathKind::File => ("shortcut", props(&[("target", path)])),
            PathKind::Missing => return None,
        },
        Dropped::Url { url, title: None } => ("shortcut", props(&[("url", url)])),
        Dropped::Url {
            url,
            title: Some(title),
        } => ("shortcut", props(&[("url", url), ("title", title)])),
    })
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
    fn drives_paths_and_web_addresses() {
        assert_eq!(classify(&path("e:\\")), Some(Dropped::Drive("E:\\".into())));
        assert_eq!(classify(&path("E:")), Some(Dropped::Drive("E:\\".into())));
        assert_eq!(
            classify(&path("C:\\Data")),
            Some(Dropped::Path("C:\\Data".into()))
        );
        let url = |url: &str, title: Option<&str>| DragItem::Url {
            url: url.into(),
            title: title.map(str::to_owned),
        };
        assert_eq!(
            classify(&url("https://example.com/", Some(" Example "))),
            Some(Dropped::Url {
                url: "https://example.com/".into(),
                title: Some("Example".into())
            })
        );
        assert_eq!(
            classify(&url("HTTP://a.b/", Some("HTTP://a.b/"))),
            Some(Dropped::Url {
                url: "HTTP://a.b/".into(),
                title: None
            }),
            "a title that is the address is none"
        );
        assert!(classify(&url("ftp://files.example.com/", None)).is_some());
        assert_eq!(classify(&url("mailto:me@example.com", None)), None);
        assert_eq!(classify(&url("https://", None)), None);
        assert_eq!(
            classify(&url("https://a.b/\r\nIconFile=\\\\host\\x.ico", None)),
            None,
            "no extra lines in a .url file"
        );
        assert!(!is_drive_root("C:\\Data"));
        assert!(!is_drive_root("1:\\"));
    }

    #[test]
    fn the_sensor_each_creates() {
        let new =
            |d: Dropped| new_sensor(&d, kind).map(|(kind, props)| (kind, Value::Object(props)));
        assert_eq!(
            new(Dropped::Drive("E:\\".into())),
            Some(("disk", json!({ "drive": "E:\\" })))
        );
        assert_eq!(
            new(Dropped::Path("C:\\Data".into())),
            Some(("folder-menu", json!({ "folder": "C:\\Data" })))
        );
        assert_eq!(
            new(Dropped::Path("C:\\a.txt".into())),
            Some(("shortcut", json!({ "target": "C:\\a.txt" })))
        );
        assert_eq!(new(Dropped::Path("C:\\Gone".into())), None);
        assert_eq!(
            new(Dropped::Url {
                url: "https://example.com/".into(),
                title: Some("Example".into())
            }),
            Some((
                "shortcut",
                json!({ "url": "https://example.com/", "title": "Example" })
            ))
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
