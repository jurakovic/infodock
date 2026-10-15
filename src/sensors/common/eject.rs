//! Eject results as notifications (SPEC §7.4, §9.18, §9.22), for the Disk launcher and, later,
//! Eject USB: the drive's name as the title and what happened as the body.

use crate::i18n::t_args;
use crate::platform::EjectError;

/// The notification for ejecting the drive called `name`.
pub fn notice(name: &str, result: &Result<(), EjectError>) -> (String, String) {
    let body = match result {
        Ok(()) => t_args("eject.done", &[("name", name)]),
        Err(EjectError::InUse(Some(app))) => {
            t_args("eject.inUseBy", &[("name", name), ("app", app)])
        }
        Err(EjectError::InUse(None)) => t_args("eject.inUse", &[("name", name)]),
        Err(EjectError::NotEjectable) => t_args("eject.notEjectable", &[("name", name)]),
        Err(EjectError::Failed(reason)) => {
            tracing::warn!("ejecting {name} failed: {reason}");
            t_args("eject.failed", &[("name", name)])
        }
    };
    (name.to_owned(), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wording_of_each_result() {
        let name = "STICK (E:)";
        assert_eq!(
            notice(name, &Ok(())),
            (name.into(), "STICK (E:) can be safely removed.".into())
        );
        assert_eq!(
            notice(name, &Err(EjectError::InUse(Some("WINWORD.EXE".into())))).1,
            "STICK (E:) is in use by WINWORD.EXE. Close it and try again."
        );
        assert!(
            notice(name, &Err(EjectError::InUse(None)))
                .1
                .contains("is in use.")
        );
        assert_eq!(
            notice(name, &Err(EjectError::NotEjectable)).1,
            "STICK (E:) cannot be ejected."
        );
        assert_eq!(
            notice(name, &Err(EjectError::Failed("x".into()))).1,
            "STICK (E:) could not be ejected."
        );
    }
}
