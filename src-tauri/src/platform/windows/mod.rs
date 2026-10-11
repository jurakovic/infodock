//! Windows implementation of the platform layer (SPEC §4.6, Appendix A).

pub mod appbar;
mod monitors;

use super::{Capabilities, MonitorInfo, Platform};

pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            appbar: true,
            ..Capabilities::default()
        }
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        monitors::list()
    }
}

/// A NUL-terminated UTF-16 buffer as a `String`.
fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
