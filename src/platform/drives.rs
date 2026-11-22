//! The drive gate: no reading starts on a drive while Windows removes, locks or dismounts it,
//! and whoever closes the drive waits for the readings already running there (INTERNALS.md,
//! Windows data sources). Portable, so it is tested on every OS.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::path_root;

/// The drive letter of `path` by its root, upper-case (`C` for `c:\Data` or `c:`); `None` for a
/// UNC or relative path.
pub fn drive_letter(path: &str) -> Option<char> {
    let root = path_root(path)?;
    let mut chars = root.chars();
    let letter = chars.next()?;
    (letter.is_ascii_uppercase() && chars.next() == Some(':')).then_some(letter)
}

/// The slot of drive `letter`, either case.
fn slot_of(letter: char) -> Option<usize> {
    let letter = letter.to_ascii_uppercase();
    letter
        .is_ascii_uppercase()
        .then(|| usize::from(letter as u8 - b'A'))
}

#[derive(Clone, Copy)]
struct Slot {
    closed: bool,
    /// Readings of the drive running now.
    readings: usize,
}

const OPEN: Slot = Slot {
    closed: false,
    readings: 0,
};

/// One slot per drive letter A–Z: whether the drive is closed, and the readings running on it.
pub struct DriveGate {
    slots: Mutex<[Slot; 26]>,
    /// Told when the last reading of a drive ends.
    left: Condvar,
}

impl Default for DriveGate {
    fn default() -> Self {
        Self::new()
    }
}

impl DriveGate {
    /// Every drive open; `const`, so the gate can be a `static`.
    pub const fn new() -> Self {
        Self {
            slots: Mutex::new([OPEN; 26]),
            left: Condvar::new(),
        }
    }

    fn slots(&self) -> MutexGuard<'_, [Slot; 26]> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A reading of `path`'s drive, counted until the entry is dropped; `None` while the drive
    /// is closed. A path without a drive letter (UNC, relative) gets an entry that counts nothing.
    pub fn enter(&self, path: &str) -> Option<Entry<'_>> {
        let Some(slot) = drive_letter(path).and_then(slot_of) else {
            return Some(Entry {
                gate: self,
                slot: None,
            });
        };
        let mut slots = self.slots();
        if slots[slot].closed {
            return None;
        }
        slots[slot].readings += 1;
        Some(Entry {
            gate: self,
            slot: Some(slot),
        })
    }

    /// Closes drive `letter`, so no new reading starts there, then waits up to `wait` for the
    /// readings already running; `false` when some still run after it.
    pub fn close(&self, letter: char, wait: Duration) -> bool {
        let Some(slot) = slot_of(letter) else {
            return true;
        };
        let mut slots = self.slots();
        slots[slot].closed = true;
        let (slots, _) = self
            .left
            .wait_timeout_while(slots, wait, |slots| slots[slot].readings > 0)
            .unwrap_or_else(PoisonError::into_inner);
        slots[slot].readings == 0
    }

    /// Opens drive `letter` again.
    pub fn open(&self, letter: char) {
        if let Some(slot) = slot_of(letter) {
            self.slots()[slot].closed = false;
        }
    }
}

/// A reading of a drive, counted while it lives (see [`DriveGate::enter`]).
pub struct Entry<'a> {
    gate: &'a DriveGate,
    slot: Option<usize>,
}

impl Drop for Entry<'_> {
    fn drop(&mut self) {
        let Some(slot) = self.slot else {
            return;
        };
        let mut slots = self.gate.slots();
        slots[slot].readings = slots[slot].readings.saturating_sub(1);
        if slots[slot].readings == 0 {
            self.gate.left.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn letters_of_paths() {
        assert_eq!(drive_letter("C:\\x"), Some('C'));
        assert_eq!(drive_letter("c:"), Some('C'));
        assert_eq!(drive_letter("f:\\Photos\\a.jpg"), Some('F'));
        assert_eq!(drive_letter("\\\\server\\share"), None);
        assert_eq!(drive_letter("\\\\server\\share\\a.txt"), None);
        assert_eq!(drive_letter("relative\\a.txt"), None);
        assert_eq!(drive_letter(""), None);
    }

    #[test]
    fn a_closed_drive_refuses_entries_and_other_drives_do_not() {
        let gate = DriveGate::new();
        assert!(gate.close('F', Duration::ZERO), "nothing was reading F:");
        assert!(gate.enter("F:\\").is_none());
        assert!(gate.enter("f:\\Photos").is_none(), "either case");
        assert!(gate.enter("E:\\").is_some());
        assert!(
            gate.enter("\\\\server\\share\\a").is_some(),
            "no drive letter"
        );
        assert!(gate.enter("relative").is_some());
    }

    #[test]
    fn open_lets_entries_in_again() {
        let gate = DriveGate::new();
        gate.close('f', Duration::ZERO);
        assert!(gate.enter("F:\\").is_none());
        gate.open('F');
        assert!(gate.enter("F:\\").is_some());
        gate.open('G');
        assert!(
            gate.enter("G:\\").is_some(),
            "opening an open drive changes nothing"
        );
    }

    #[test]
    fn close_waits_for_a_reading_that_ends() {
        let gate = DriveGate::new();
        std::thread::scope(|s| {
            let entry = gate.enter("F:\\Photos").expect("open");
            s.spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                drop(entry);
            });
            let started = Instant::now();
            assert!(gate.close('F', Duration::from_secs(5)));
            let waited = started.elapsed();
            assert!(waited >= Duration::from_millis(40), "{waited:?}");
            assert!(waited < Duration::from_secs(5), "{waited:?}");
        });
        assert!(gate.enter("F:\\").is_none(), "still closed");
    }

    #[test]
    fn close_gives_up_when_the_wait_runs_out() {
        let gate = DriveGate::new();
        let entry = gate.enter("F:\\").expect("open");
        let other = gate.enter("E:\\").expect("open");
        let unlettered = gate.enter("\\\\server\\share").expect("counts nothing");
        let started = Instant::now();
        assert!(!gate.close('F', Duration::from_millis(30)));
        assert!(started.elapsed() >= Duration::from_millis(25));
        assert!(
            gate.close('G', Duration::ZERO),
            "readings of other drives do not count"
        );
        drop(other);
        drop(unlettered);
        drop(entry);
        assert!(gate.close('F', Duration::ZERO), "the reading ended");
    }
}
