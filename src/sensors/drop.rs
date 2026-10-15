//! Drops onto sensors (SPEC §6.3, §9.16, §9.19): the dragged items as the bar classified
//! them, what a sensor would do with them, and the wording of the floating drop hint.

use crate::i18n::t_count;
use crate::platform::{DropEffect, DropEffects, FileOp, Modifiers};

/// A dragged item the bar can take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    /// A drive root, `E:\`.
    Drive(String),
    Folder(String),
    File(String),
    /// An `http`, `https` or `ftp` address, with the link's text.
    Url {
        url: String,
        title: Option<String>,
    },
}

impl Dropped {
    /// The path of a file or folder.
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Folder(path) | Self::File(path) => Some(path),
            Self::Drive(_) | Self::Url { .. } => None,
        }
    }
}

/// The keys held and the effects the source allows, as the drag goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DragInfo {
    pub modifiers: Modifiers,
    pub allowed: DropEffects,
}

/// What dropping onto a sensor would do: the pointer's effect and the floating hint above the
/// sensor (`None` when nothing would happen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropAction {
    pub effect: DropEffect,
    pub hint: Option<String>,
}

impl DropAction {
    /// The sensor takes the drop but nothing can happen (e.g. moving a folder into itself).
    pub const REFUSED: Self = Self {
        effect: DropEffect::None,
        hint: None,
    };
}

/// The longest source text a drop hint shows (SPEC §9.19).
const LABEL_MAX: usize = 100;

/// What a hint calls `item`: a file or folder by its name, a link by its address, at most 100
/// characters.
pub fn label(item: &Dropped) -> String {
    let text = match item {
        Dropped::Drive(root) => root.as_str(),
        Dropped::Folder(path) | Dropped::File(path) => super::shortcut::file_name(path),
        Dropped::Url { url, .. } => url.as_str(),
    };
    if text.chars().count() > LABEL_MAX {
        let cut: String = text.chars().take(LABEL_MAX - 1).collect();
        format!("{cut}…")
    } else {
        text.to_owned()
    }
}

/// The floating hint for `items` (SPEC §9.19): `Move 'report.pdf' to 'Downloads'`, `Copy 3
/// items to 'Downloads'`, `Delete 2 items`; `key` names the message (`drop.move` …), whose
/// `one` form names the item.
pub fn hint(key: &str, items: &[Dropped], folder: &str) -> String {
    let item = items.first().map(label).unwrap_or_default();
    t_count(
        key,
        items.len() as u64,
        &[("item", &item), ("folder", folder)],
    )
}

/// The file operation for a drop into a folder (SPEC §9.19, as Explorer does it): Alt makes
/// shortcuts, Ctrl copies, Shift moves; otherwise items on the folder's volume move and others
/// are copied. What the source does not allow is not done; `None` when nothing is allowed.
pub fn file_op(modifiers: Modifiers, same_volume: bool, allowed: DropEffects) -> Option<FileOp> {
    let allows = |op: FileOp| allowed.allows(op.effect());
    let preferred = if modifiers.alt || (modifiers.ctrl && modifiers.shift) {
        return allows(FileOp::Link).then_some(FileOp::Link);
    } else if modifiers.ctrl {
        return allows(FileOp::Copy).then_some(FileOp::Copy);
    } else if modifiers.shift {
        return allows(FileOp::Move).then_some(FileOp::Move);
    } else if same_volume {
        [FileOp::Move, FileOp::Copy]
    } else {
        [FileOp::Copy, FileOp::Move]
    };
    preferred.into_iter().find(|&op| allows(op))
}

/// The file name of an Internet shortcut for `url` (SPEC §9.19): the page's title, or else the
/// address with `:/\&+?*=.` replaced by `-`; characters Windows does not allow in names are
/// replaced too. Without `.url`.
pub fn url_file_stem(url: &str, title: Option<&str>) -> String {
    let (text, replaced): (&str, &[char]) = match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(title) => (title, &['\\', '/', ':', '*', '?', '"', '<', '>', '|']),
        None => (
            url,
            &[
                ':', '/', '\\', '&', '+', '?', '*', '=', '.', '"', '<', '>', '|',
            ],
        ),
    };
    let name: String = text
        .chars()
        .map(|c| {
            if replaced.contains(&c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .take(LABEL_MAX)
        .collect();
    // Windows drops trailing dots and spaces from names.
    let name = name.trim_end_matches(['.', ' ']).trim_start();
    if name.is_empty() {
        "Link".to_owned()
    } else {
        name.to_owned()
    }
}

/// Whether `path` is `folder` or lies inside it (ignoring case and a trailing separator).
pub fn is_within(path: &str, folder: &str) -> bool {
    let norm = |p: &str| {
        p.trim_end_matches(['\\', '/'])
            .to_lowercase()
            .replace('/', "\\")
    };
    let (path, folder) = (norm(path), norm(folder));
    path == folder || path.starts_with(&format!("{folder}\\"))
}

/// Whether `a` and `b` name the same folder (ignoring case and a trailing separator).
pub fn same_folder(a: &str, b: &str) -> bool {
    let norm = |p: &str| {
        p.trim_end_matches(['\\', '/'])
            .to_lowercase()
            .replace('/', "\\")
    };
    norm(a) == norm(b)
}

/// The folder holding `path`.
pub fn parent(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        // Keep the separator of a drive root: `C:\a` → `C:\`.
        Some(i) if trimmed[..i].ends_with(':') => &trimmed[..=i],
        Some(i) => &trimmed[..i],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_name_one_item_or_count_them() {
        let one = [Dropped::File("C:\\Data\\report.pdf".into())];
        assert_eq!(
            hint("drop.move", &one, "Downloads"),
            "Move 'report.pdf' to 'Downloads'"
        );
        let three = [
            Dropped::File("C:\\a".into()),
            Dropped::Folder("C:\\b".into()),
            Dropped::File("C:\\c".into()),
        ];
        assert_eq!(
            hint("drop.copy", &three, "Downloads"),
            "Copy 3 items to 'Downloads'"
        );
        let long = format!("https://example.com/{}", "a".repeat(200));
        let link = [Dropped::Url {
            url: long,
            title: None,
        }];
        let text = hint("drop.urlShortcut", &link, "Downloads");
        assert!(
            text.starts_with("Create shortcut to 'https://example.com/aaa"),
            "{text}"
        );
        assert!(text.contains("a…' in 'Downloads'"), "{text}");
        assert_eq!(hint("drop.delete", &three, ""), "Delete 3 items");
        assert_eq!(hint("drop.delete", &one, ""), "Delete 'report.pdf'");
    }

    #[test]
    fn explorers_rules_for_move_copy_and_link() {
        let all = DropEffects::ALL;
        let keys = |ctrl, shift, alt| Modifiers { alt, ctrl, shift };
        let none = keys(false, false, false);
        assert_eq!(file_op(none, true, all), Some(FileOp::Move));
        assert_eq!(file_op(none, false, all), Some(FileOp::Copy));
        assert_eq!(
            file_op(keys(true, false, false), true, all),
            Some(FileOp::Copy)
        );
        assert_eq!(
            file_op(keys(false, true, false), false, all),
            Some(FileOp::Move)
        );
        assert_eq!(
            file_op(keys(false, false, true), true, all),
            Some(FileOp::Link)
        );
        assert_eq!(
            file_op(keys(true, true, false), true, all),
            Some(FileOp::Link)
        );
        let copy_only = DropEffects {
            copy: true,
            ..DropEffects::default()
        };
        assert_eq!(file_op(none, true, copy_only), Some(FileOp::Copy));
        assert_eq!(file_op(keys(false, true, false), true, copy_only), None);
        assert_eq!(file_op(none, true, DropEffects::default()), None);
    }

    #[test]
    fn internet_shortcut_names() {
        assert_eq!(
            url_file_stem("https://example.com/", Some("Example: Domain?")),
            "Example- Domain-"
        );
        assert_eq!(
            url_file_stem("https://example.com/a?b=1&c", None),
            "https---example-com-a-b-1-c"
        );
        assert_eq!(url_file_stem("https://x.y/", Some("  ")), "https---x-y-");
        assert_eq!(url_file_stem("https://x/", Some("...")), "Link");
    }

    #[test]
    fn folders_and_parents() {
        assert!(is_within("C:\\Data\\a.txt", "c:\\data\\"));
        assert!(is_within("C:\\Data", "C:\\Data"));
        assert!(!is_within("C:\\Database", "C:\\Data"));
        assert_eq!(parent("C:\\Data\\a.txt"), "C:\\Data");
        assert_eq!(parent("C:\\a.txt"), "C:\\");
        assert_eq!(parent("\\\\server\\share\\a"), "\\\\server\\share");
        assert!(same_folder(parent("C:\\a.txt"), "c:"));
        assert!(!same_folder("C:\\Data", "C:\\"));
    }
}
