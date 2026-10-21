//! Settings ▸ About (SPEC §10.7): the version, Infodock's licence, the crates of the Windows
//! build with their licences, Slint's attribution (`AboutSlint`) and the logs folder. The crates
//! come from `assets/third-party.tsv`, which `.github/scripts/third-party.py` writes from
//! `cargo tree`; a test keeps it in step with `Cargo.lock`.

use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, SharedString, StandardListViewItem, VecModel};

use crate::app::App;
use crate::i18n::t_args;
use crate::ui;

/// Every package of `Cargo.lock` but Infodock, with its licence, or `-` for one the Windows
/// build does not link (a build, dev, proc-macro or other-platform dependency).
const THIRD_PARTY: &str = include_str!("../../assets/third-party.tsv");
const LICENCE: &str = include_str!("../../LICENSE");

/// The packages `tsv` lists: name, version and licence, `None` for one not in the build.
fn packages(tsv: &str) -> impl Iterator<Item = (&str, &str, Option<&str>)> {
    // A checkout may have turned the line ends into CRLF.
    tsv.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let (name, version, licence) = (fields.next()?, fields.next()?, fields.next()?);
            Some((name, version, (licence != "-").then_some(licence)))
        })
}

/// The crates of the Windows build as the page lists them, e.g. `slint 1.18.1: GPL-3.0-only OR
/// LicenseRef-Slint-Royalty-free-2.0 OR …`.
fn crate_lines() -> Vec<String> {
    packages(THIRD_PARTY)
        .filter_map(|(name, version, licence)| {
            licence.map(|licence| {
                t_args(
                    "settings.about.crate",
                    &[("name", name), ("version", version), ("licence", licence)],
                )
            })
        })
        .collect()
}

/// `text` with each paragraph on one line, so the page wraps it to its own width.
fn reflow(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    text.trim()
        .split("\n\n")
        .map(|paragraph| paragraph.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    let state = window.global::<ui::SettingsState>();
    state.set_version(crate::VERSION.into());
    state.set_licence(reflow(LICENCE).into());
    let rows: Vec<StandardListViewItem> = crate_lines()
        .into_iter()
        .map(|line| SharedString::from(line).into())
        .collect();
    state.set_third_party(ModelRc::new(VecModel::from(rows)));
    let a = Arc::clone(app);
    state.on_open_logs_folder(move || {
        let logs = crate::platform::app_dirs(crate::IDENTIFIER).logs;
        a.platform.open(&logs.display().to_string());
    });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// The packages of `Cargo.lock` but Infodock, by name and version.
    fn locked() -> BTreeSet<(String, String)> {
        let lock = include_str!("../../Cargo.lock");
        lock.split("[[package]]")
            .skip(1)
            .filter_map(|block| {
                let field = |key: &str| {
                    block.lines().find_map(|line| {
                        let value = line.trim_end().strip_prefix(key)?.strip_prefix(" = ")?;
                        Some(value.trim_matches('"').to_owned())
                    })
                };
                Some((field("name")?, field("version")?))
            })
            .filter(|(name, _)| name != "infodock")
            .collect()
    }

    #[test]
    fn the_list_has_every_package_of_the_lock_file() {
        let listed: BTreeSet<(String, String)> = packages(THIRD_PARTY)
            .map(|(name, version, _)| (name.to_owned(), version.to_owned()))
            .collect();
        let locked = locked();
        assert!(locked.len() > 100, "{} packages", locked.len());
        let stale: Vec<_> = listed.difference(&locked).collect();
        let new: Vec<_> = locked.difference(&listed).collect();
        assert!(
            stale.is_empty() && new.is_empty(),
            "assets/third-party.tsv is out of date: run python3 .github/scripts/third-party.py \
             (not in Cargo.lock: {stale:?}; not listed: {new:?})"
        );
    }

    #[test]
    fn the_page_lists_the_windows_build_with_licences() {
        let lines = crate_lines();
        assert!(lines.len() > 100, "{} crates", lines.len());
        assert!(lines.iter().any(|l| l.starts_with("slint 1.")), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("windows ")), "{lines:?}");
        // Not linked: Slint's testing backend (a dev-dependency) and proc-macros.
        assert!(
            !lines
                .iter()
                .any(|l| l.starts_with("i-slint-backend-testing "))
        );
        assert!(!lines.iter().any(|l| l.starts_with("serde_derive ")));
        assert!(
            packages(THIRD_PARTY).all(|(_, _, licence)| licence != Some("")),
            "every crate names its licence"
        );
        assert!(LICENCE.starts_with("MIT License"));
    }

    #[test]
    fn the_licence_wraps_to_the_page() {
        assert_eq!(
            reflow("MIT License\r\n\r\nPermission is hereby granted,\r\nfree of charge.\r\n"),
            "MIT License\n\nPermission is hereby granted, free of charge."
        );
    }

    #[test]
    fn reads_the_list_with_any_line_ends() {
        let tsv = "# header\r\na\t1.0.0\tMIT\r\nb\t0.2.0\t-\r\n";
        assert_eq!(
            packages(tsv).collect::<Vec<_>>(),
            [("a", "1.0.0", Some("MIT")), ("b", "0.2.0", None)]
        );
    }
}
