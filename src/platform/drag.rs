//! Drag & drop from other apps onto the bar (SPEC §6.3): what is dragged, the effects, and the
//! clipboard formats it comes in, read here as plain bytes so they are tested on every OS.

use super::{Modifiers, PointF};

/// Something dragged onto the bar from another app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragItem {
    /// A file, folder or drive (`E:\`).
    Path(String),
    /// A web address, with the link's text when the source offers it.
    Url { url: String, title: Option<String> },
}

/// Drop effects (`DROPEFFECT_*`): the ones a source allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DropEffects {
    pub copy: bool,
    pub moves: bool,
    pub link: bool,
}

impl DropEffects {
    pub const ALL: Self = Self {
        copy: true,
        moves: true,
        link: true,
    };

    pub fn allows(self, effect: DropEffect) -> bool {
        match effect {
            DropEffect::None => true,
            DropEffect::Copy => self.copy,
            DropEffect::Move => self.moves,
            DropEffect::Link => self.link,
        }
    }
}

/// What a drop does, as the pointer shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DropEffect {
    /// Nothing: the no-drop pointer.
    #[default]
    None,
    Copy,
    Move,
    Link,
}

/// A drag from another app over the bar window, at logical px from its top-left corner, with
/// the modifier keys held and the effects the source allows.
#[derive(Debug, Clone, PartialEq)]
pub enum DragEvent {
    Enter {
        items: Vec<DragItem>,
        at: PointF,
        modifiers: Modifiers,
        allowed: DropEffects,
    },
    Over {
        at: PointF,
        modifiers: Modifiers,
        allowed: DropEffects,
    },
    Leave,
    Drop {
        at: PointF,
        modifiers: Modifiers,
        allowed: DropEffects,
    },
}

/// UTF-16 little-endian code units of `bytes`.
fn utf16(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// A NUL-terminated UTF-16 string at the start of `bytes` (`UniformResourceLocatorW`).
pub fn wide_string(bytes: &[u8]) -> String {
    let units = utf16(bytes);
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// A NUL-terminated string in the system's ANSI code page at the start of `bytes`
/// (`UniformResourceLocator`); URLs are ASCII, so anything else is read lossily.
pub fn narrow_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The paths of a `CF_HDROP` (`DROPFILES`: the offset of the list, a point, two flags, then
/// NUL-separated paths ending with an empty one, UTF-16 when `fWide`).
pub fn dropfiles(bytes: &[u8]) -> Vec<String> {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (Some(offset), Some(wide)) = (word(0), word(16)) else {
        return Vec::new();
    };
    let Some(list) = bytes.get(offset as usize..) else {
        return Vec::new();
    };
    let paths: Vec<String> = if wide != 0 {
        utf16(list)
            .split(|&u| u == 0)
            .take_while(|p| !p.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    } else {
        list.split(|&b| b == 0)
            .take_while(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect()
    };
    paths
}

/// The name of the first file a `FileGroupDescriptorW` describes, without `.url`: a browser
/// names the link file it offers after the page (SPEC §6.3).
pub fn link_file_title(bytes: &[u8]) -> Option<String> {
    // cItems (4 bytes), then FILEDESCRIPTORW with cFileName (260 UTF-16 units) at 72.
    const NAME: usize = 4 + 72;
    let count = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?);
    if count == 0 {
        return None;
    }
    let name = wide_string(bytes.get(NAME..(NAME + 520).min(bytes.len()))?);
    let lower = name.to_ascii_lowercase();
    let title = match lower.strip_suffix(".url") {
        Some(stem) => &name[..stem.len()],
        None => &name,
    };
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_owned())
}

/// The text of the first link in a `HTML Format` clipboard fragment, tags removed, entities
/// decoded and spaces collapsed; `None` when there is no link or it has no text.
pub fn html_link_text(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<a ").or_else(|| lower.find("<a>"))?;
    let open_end = start + lower[start..].find('>')? + 1;
    let close = open_end + lower[open_end..].find("</a")?;
    let mut text = String::new();
    let mut in_tag = false;
    for c in html[open_end..close].chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    let decoded = decode_entities(&text);
    let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, `&nbsp;` and numeric references.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';').filter(|&s| s <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..semi];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn hdrop_lists_wide_and_narrow_paths() {
        let mut bytes = vec![20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
        bytes.extend(wide("C:\\Data\\a.txt\0E:\\\0\0"));
        assert_eq!(dropfiles(&bytes), ["C:\\Data\\a.txt", "E:\\"]);
        let mut narrow = vec![20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        narrow.extend(b"C:\\x\0\0");
        assert_eq!(dropfiles(&narrow), ["C:\\x"]);
        assert!(dropfiles(&[1, 2]).is_empty());
        assert!(
            dropfiles(&[200, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]).is_empty()
        );
    }

    #[test]
    fn urls_in_either_width() {
        assert_eq!(
            wide_string(&wide("https://example.com/\0junk")),
            "https://example.com/"
        );
        assert_eq!(
            narrow_string(b"https://example.com/\0"),
            "https://example.com/"
        );
        assert_eq!(narrow_string(b"no terminator"), "no terminator");
    }

    #[test]
    fn the_link_file_names_the_page() {
        let mut bytes = vec![1, 0, 0, 0];
        bytes.extend(vec![0; 72]);
        let mut name = wide("Example Domain.URL");
        name.resize(520, 0);
        bytes.extend(name);
        assert_eq!(link_file_title(&bytes), Some("Example Domain".into()));
        let mut none = vec![0, 0, 0, 0];
        none.extend(vec![0; 600]);
        assert_eq!(link_file_title(&none), None);
        assert_eq!(link_file_title(&[1, 0]), None);
    }

    #[test]
    fn the_link_text_of_a_html_fragment() {
        let html = "Version:0.9\r\nStartHTML:1\r\n<html><body><!--StartFragment--><A \
                    href=\"https://example.com/\"><b>Example</b>\n  Domain &amp; more&#33;</a>\
                    <!--EndFragment--></body></html>";
        assert_eq!(html_link_text(html), Some("Example Domain & more!".into()));
        assert_eq!(html_link_text("<a href=\"x\"> </a>"), None);
        assert_eq!(html_link_text("<p>no link</p>"), None);
        assert_eq!(decode_entities("a & b &bogus; &#x41;"), "a & b &bogus; A");
    }

    #[test]
    fn effects_a_source_allows() {
        let copy_link = DropEffects {
            copy: true,
            moves: false,
            link: true,
        };
        assert!(copy_link.allows(DropEffect::Link));
        assert!(!copy_link.allows(DropEffect::Move));
        assert!(DropEffects::ALL.allows(DropEffect::Move));
    }
}
