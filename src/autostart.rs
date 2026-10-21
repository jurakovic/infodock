//! Starting with Windows (SPEC §10.1): while `settings.autostart` is on, the per-user `Run` key
//! starts this exe at sign-in under the value [`RUN_VALUE`]. The entry follows the setting at
//! start-up (also after the exe it named moved away) and whenever it changes; until the first-run welcome is
//! done, an existing entry is taken into the setting instead (SPEC §11.5), so an installer that
//! writes one needs nothing else from the app.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use crate::config::ConfigStore;
use crate::i18n::t_args;
use crate::platform::{AutostartChange, Platform};

/// The `Run` value's name. Fixed: the uninstaller removes it by this name (SPEC §15.3), and
/// later versions look for it.
pub const RUN_VALUE: &str = "Infodock";

/// Why the entry could not be written or removed the last time; Settings shows it.
static ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Why the `Run` entry does not follow the setting, as a sentence; `None` when it does.
pub fn error() -> Option<String> {
    ERROR.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Brings the setting and the entry together at start-up and makes the entry follow the setting
/// from then on.
pub fn init(store: &ConfigStore, platform: Arc<dyn Platform>) {
    let Some(command) = this_command() else {
        return;
    };
    let entry = platform.autostart(RUN_VALUE);
    let config = store.get();
    let mut enabled = config.settings.autostart;
    if let Some(taken) = taken(config.meta.first_run_done, enabled, entry.is_some()) {
        let result = store.update(|c| {
            c.settings.autostart = taken;
            Ok(())
        });
        match result {
            Ok(_) => enabled = taken,
            Err(e) => tracing::warn!("could not take the sign-in entry into the settings: {e}"),
        }
    }
    let named_exists = entry
        .as_deref()
        .is_some_and(|e| Path::new(program(e)).exists());
    if let Some(change) = at_start(enabled, entry.as_deref(), &command, named_exists) {
        apply(platform.as_ref(), &change);
    }
    store.on_change(Box::new(move |old, new| {
        let enabled = new.settings.autostart;
        if enabled != old.settings.autostart {
            let change = if enabled {
                AutostartChange::Enable(&command)
            } else {
                AutostartChange::Remove
            };
            apply(platform.as_ref(), &change);
        }
    }));
}

/// The command that starts this exe, `None` if its path is unknown.
fn this_command() -> Option<String> {
    match std::env::current_exe() {
        Ok(exe) => Some(command(&exe)),
        Err(e) => {
            tracing::warn!("autostart unavailable: the app's path is unknown: {e}");
            None
        }
    }
}

fn apply(platform: &dyn Platform, change: &AutostartChange) {
    let result = platform.set_autostart(RUN_VALUE, change);
    match &result {
        Ok(()) => tracing::info!("sign-in entry: {change:?}"),
        Err(e) => tracing::warn!("could not change the sign-in entry ({change:?}): {e}"),
    }
    let error = result
        .err()
        .map(|e| t_args("settings.general.autostart.error", &[("error", &e)]));
    *ERROR.lock().unwrap_or_else(PoisonError::into_inner) = error;
}

/// The `Run` entry's command for `exe`: its path in quotes.
pub fn command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// The setting an existing entry gives on first run (before the welcome page is done), if it
/// differs from the stored one: an entry means on, none off.
pub fn taken(first_run_done: bool, enabled: bool, entry: bool) -> Option<bool> {
    (!first_run_done && entry != enabled).then_some(entry)
}

/// What start-up does to the entry so it follows the setting: write a missing one, point one at
/// an exe that is gone (`named_exists` false: the app moved) to this one, keeping a "disabled"
/// mark from Task Manager, which the user set; remove one while the setting is off. An entry
/// for another copy that is still there stays: a copy started once (a portable exe tried, a
/// build) does not take it.
pub fn at_start<'a>(
    enabled: bool,
    entry: Option<&str>,
    command: &'a str,
    named_exists: bool,
) -> Option<AutostartChange<'a>> {
    match entry {
        None if enabled => Some(AutostartChange::Enable(command)),
        Some(entry) if enabled && !named_exists && !same_command(entry, command) => {
            Some(AutostartChange::Update(command))
        }
        Some(_) if !enabled => Some(AutostartChange::Remove),
        _ => None,
    }
}

/// The program a command starts: the path in its leading quotes, or the whole command.
fn program(command: &str) -> &str {
    let command = command.trim();
    match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => command,
    }
}

/// Two commands start the same program: equal but for surrounding spaces, quotes around the
/// whole and case (Windows paths ignore it).
fn same_command(a: &str, b: &str) -> bool {
    let plain = |s: &str| {
        let s = s.trim();
        let s = s
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(s);
        s.to_lowercase()
    };
    plain(a) == plain(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_quotes_the_path() {
        let exe = Path::new(r"C:\Users\Ana\AppData\Local\Infodock\infodock.exe");
        assert_eq!(
            command(exe),
            r#""C:\Users\Ana\AppData\Local\Infodock\infodock.exe""#
        );
    }

    #[test]
    fn an_entry_is_taken_into_the_setting_until_the_first_run_is_done() {
        assert_eq!(taken(false, false, true), Some(true));
        assert_eq!(taken(false, true, false), Some(false));
        assert_eq!(taken(false, true, true), None);
        assert_eq!(taken(false, false, false), None);
        // Afterwards the setting decides.
        assert_eq!(taken(true, false, true), None);
        assert_eq!(taken(true, true, false), None);
    }

    #[test]
    fn the_entry_follows_the_setting_at_start() {
        let this = r#""C:\Apps\Infodock\infodock.exe""#;
        assert_eq!(
            at_start(true, None, this, false),
            Some(AutostartChange::Enable(this))
        );
        assert_eq!(at_start(true, Some(this), this, true), None);
        // The same program written differently.
        let other_case = r"c:\apps\INFODOCK\infodock.exe";
        assert_eq!(at_start(true, Some(other_case), this, true), None);
        assert_eq!(at_start(true, Some(&format!(" {this} ")), this, true), None);
        // The exe it named moved away: the entry points to this one.
        let moved = r#""D:\Portable\infodock.exe""#;
        assert_eq!(
            at_start(true, Some(moved), this, false),
            Some(AutostartChange::Update(this))
        );
        // Another copy that is still there keeps it (review M4).
        assert_eq!(at_start(true, Some(moved), this, true), None);
        assert_eq!(
            at_start(false, Some(this), this, true),
            Some(AutostartChange::Remove)
        );
        assert_eq!(
            at_start(false, Some(moved), this, true),
            Some(AutostartChange::Remove)
        );
        assert_eq!(at_start(false, None, this, false), None);
    }

    #[test]
    fn the_program_of_a_command() {
        assert_eq!(
            program(r#""C:\A b\infodock.exe" --x"#),
            r"C:\A b\infodock.exe"
        );
        assert_eq!(program(r" C:\infodock.exe "), r"C:\infodock.exe");
        assert_eq!(program(r#""C:\x.exe"#), r"C:\x.exe");
    }
}
