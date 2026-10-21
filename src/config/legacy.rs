//! The legacy XML layout format (SPEC §11.4): reading a file into its sensors, and the value
//! encodings its properties use, as pure functions. Each sensor type turns the properties into
//! its props (`sensors::legacy`).

use std::collections::BTreeMap;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};

use super::exchange::{ImportError, read_bytes};
use super::model::{Zone, normalize_color};

/// A legacy sensor's properties by XML key. A key given twice keeps its last value.
pub type LegacyProps = BTreeMap<String, String>;

/// A `<sensor>` element.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacySensor {
    /// The zone its `side` names; `None` for a missing or unknown side.
    pub zone: Option<Zone>,
    /// The legacy type id, e.g. `DockText`.
    pub kind: String,
    pub props: LegacyProps,
}

/// Reads a legacy layout file; see [`parse`].
pub fn read(path: &Path) -> Result<Vec<LegacySensor>, ImportError> {
    parse(&decode(&read_bytes(path)?))
}

/// The sensors of the first `<sensors>…</sensors>` in `text`, which may be embedded in other
/// text (e.g. a settings dump), in document order.
pub fn parse(text: &str) -> Result<Vec<LegacySensor>, ImportError> {
    let xml = sensors_element(text).ok_or(ImportError::NoLegacyLayout)?;
    let mut reader = Reader::from_str(xml);
    let mut sensors = Vec::new();
    // The sensor being read, and the property being read in it (its name and text so far).
    let mut sensor: Option<LegacySensor> = None;
    let mut property: Option<(String, String)> = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| ImportError::Xml(e.to_string()))?;
        match event {
            Event::Start(e) if e.name().as_ref() == "sensor" => {
                sensor = Some(new_sensor(&e));
                property = None;
            }
            Event::Empty(e) if e.name().as_ref() == "sensor" => sensors.push(new_sensor(&e)),
            Event::Start(e) if e.name().as_ref() == "property" && sensor.is_some() => {
                property = attribute(&e, "name").map(|name| (name, String::new()));
            }
            Event::Empty(e) if e.name().as_ref() == "property" => {
                if let (Some(sensor), Some(name)) = (sensor.as_mut(), attribute(&e, "name")) {
                    sensor.props.insert(name, String::new());
                }
            }
            Event::Text(e) => {
                if let Some((_, value)) = property.as_mut() {
                    value.push_str(&e.xml10_content());
                }
            }
            Event::CData(e) => {
                if let Some((_, value)) = property.as_mut() {
                    value.push_str(&e);
                }
            }
            Event::GeneralRef(e) => {
                if let Some((_, value)) = property.as_mut() {
                    match e.resolve_char_ref() {
                        Ok(Some(c)) => value.push(c),
                        _ => match resolve_predefined_entity(&e.xml10_content()) {
                            Some(s) => value.push_str(s),
                            // An entity the file does not define: kept as written.
                            None => value.push_str(&format!("&{};", e.xml10_content())),
                        },
                    }
                }
            }
            Event::End(e) if e.name().as_ref() == "property" => {
                if let (Some(sensor), Some((name, value))) = (sensor.as_mut(), property.take()) {
                    sensor.props.insert(name, value);
                }
            }
            Event::End(e) if e.name().as_ref() == "sensor" => {
                property = None;
                sensors.extend(sensor.take());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(sensors)
}

fn new_sensor(tag: &BytesStart) -> LegacySensor {
    LegacySensor {
        zone: attribute(tag, "side").as_deref().and_then(zone),
        kind: attribute(tag, "type").unwrap_or_default(),
        props: LegacyProps::new(),
    }
}

/// The value of attribute `name`, entities resolved.
fn attribute(tag: &BytesStart, name: &str) -> Option<String> {
    let attribute = tag.try_get_attribute(name).ok()??;
    attribute
        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
        .ok()
        .map(|v| v.into_owned())
}

fn zone(side: &str) -> Option<Zone> {
    match side.trim().to_ascii_lowercase().as_str() {
        "left" => Some(Zone::Left),
        "center" | "centre" => Some(Zone::Center),
        "right" => Some(Zone::Right),
        _ => None,
    }
}

/// The first `<sensors>…</sensors>` (or `<sensors/>`) in `text`.
fn sensors_element(text: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(found) = text[from..].find("<sensors") {
        let start = from + found;
        let rest = &text[start + "<sensors".len()..];
        match rest.chars().next() {
            Some('>') | Some('/') | Some(' ' | '\t' | '\r' | '\n') => {
                let open_end = start + text[start..].find('>')? + 1;
                if text[..open_end].ends_with("/>") {
                    return Some(&text[start..open_end]);
                }
                let close = open_end + text[open_end..].find("</sensors>")? + "</sensors>".len();
                return Some(&text[start..close]);
            }
            _ => from = start + 1,
        }
    }
    None
}

/// The text of a legacy file: UTF-8 (with or without a byte order mark), UTF-16 with a byte
/// order mark, or else Windows-1252, which older Windows programs wrote.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(utf8) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(utf8).into_owned();
    }
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, u16::from_be_bytes);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

fn windows_1252(byte: u8) -> char {
    /// 0x80–0x9F; the five bytes Windows-1252 leaves undefined map to the C1 controls.
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

// ---------------------------------------------------------------------------------------------
// Value encodings (SPEC §11.4)
// ---------------------------------------------------------------------------------------------

/// `yes` / `no` (also `true` / `false`, `1` / `0`).
pub fn bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// A colour mode: `off:#RRGGBB` (standard), `col:#RRGGBB` (custom), `dyn:#RRGGBB` (dynamic) or a
/// bare `#RRGGBB` (custom). The mode id and the colour; standard ignores its colour, so it has
/// none.
pub fn color_mode(value: &str) -> Option<(&'static str, Option<String>)> {
    let value = value.trim();
    let (mode, color) = match value.split_once(':') {
        Some((prefix, color)) => (
            match prefix.trim().to_ascii_lowercase().as_str() {
                "off" => "standard",
                "col" => "custom",
                "dyn" => "dynamic",
                _ => return None,
            },
            color,
        ),
        None => ("custom", value),
    };
    if mode == "standard" {
        return Some((mode, None));
    }
    Some((mode, Some(normalize_color(color.trim().to_owned())?)))
}

/// A number, e.g. an interval in seconds (may be fractional) or a size.
pub fn number(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// An opacity 0–255 as a percentage: round(v / 2.55).
pub fn opacity(value: &str) -> Option<u32> {
    number(value).map(|n| (n.clamp(0.0, 255.0) / 2.55).round() as u32)
}

/// A graph's width in px: rounded down to a multiple of 5, then clamped to 50–150.
pub fn graph_width(value: &str) -> Option<u32> {
    number(value).map(|n| ((n.max(0.0) as u32) / 5 * 5).clamp(50, 150))
}

/// A file or folder path, with `/` as `\` (Infodock's paths are Windows paths).
pub fn path(value: &str) -> String {
    value.trim().replace('/', "\\")
}

/// A drive: `C:\`, also written `C:` or `C:/`.
pub fn drive(value: &str) -> Option<String> {
    let path = path(value);
    let root = match path.len() {
        2 => format!("{path}\\"),
        _ => path,
    };
    let bytes = root.as_bytes();
    (bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && &root[1..] == ":\\")
        .then(|| root.to_ascii_uppercase())
}

/// A font style: `""` → normal, `bold`, `italic`, `bold, italic` → boldItalic.
pub fn font_style(value: &str) -> Option<&'static str> {
    let (mut bold, mut italic) = (false, false);
    for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "bold" => bold = true,
            "italic" => italic = true,
            "regular" | "normal" => {}
            _ => return None,
        }
    }
    Some(match (bold, italic) {
        (false, false) => "normal",
        (true, false) => "bold",
        (false, true) => "italic",
        (true, true) => "boldItalic",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, &str)]) -> LegacyProps {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn reads_sides_types_and_properties_in_document_order() {
        let sensors = parse(
            r#"<?xml version="1.0" ?>
            <sensors>
              <sensor side="right" type="DockText">
                <property name="text">Tom &amp; Jerry &#8364; &lt;3&gt;</property>
                <property name="font"></property>
                <property name="fontsize"/>
              </sensor>
              <sensor side="left" type="DockSpacer"><property name="width">12</property></sensor>
              <sensor side="right" type="DockImage"/>
              <sensor side="middle" type="DockText"></sensor>
              <sensor type="DockText"><property name="text"><![CDATA[a <b>]]></property></sensor>
            </sensors>"#,
        )
        .expect("parsed");
        assert_eq!(
            sensors,
            [
                LegacySensor {
                    zone: Some(Zone::Right),
                    kind: "DockText".into(),
                    props: props(&[
                        ("text", "Tom & Jerry € <3>"),
                        ("font", ""),
                        ("fontsize", "")
                    ]),
                },
                LegacySensor {
                    zone: Some(Zone::Left),
                    kind: "DockSpacer".into(),
                    props: props(&[("width", "12")]),
                },
                LegacySensor {
                    zone: Some(Zone::Right),
                    kind: "DockImage".into(),
                    props: props(&[]),
                },
                LegacySensor {
                    zone: None,
                    kind: "DockText".into(),
                    props: props(&[]),
                },
                LegacySensor {
                    zone: None,
                    kind: "DockText".into(),
                    props: props(&[("text", "a <b>")]),
                },
            ]
        );
    }

    #[test]
    fn keeps_spaces_and_lines_in_values() {
        let sensors = parse(
            "<sensors><sensor side=\"center\" type=\"DockText\">\
             <property name=\"text\">  two\r\nlines </property></sensor></sensors>",
        )
        .expect("parsed");
        assert_eq!(sensors[0].props["text"], "  two\nlines ");
    }

    #[test]
    fn finds_the_first_sensors_element_in_other_text() {
        let dump = "[settings]\nwidth=3\n<sensorsX>no</sensorsX>\n\
                    <sensors>\n<sensor side=\"left\" type=\"DockSpacer\"/>\n</sensors>\n\
                    trailing <sensors><sensor side=\"left\" type=\"DockText\"/></sensors>";
        let sensors = parse(dump).expect("parsed");
        assert_eq!(sensors.len(), 1);
        assert_eq!(sensors[0].kind, "DockSpacer");
        assert_eq!(parse("<sensors/>").expect("parsed"), []);
        assert_eq!(parse("<sensors >\n</sensors>").expect("parsed"), []);
    }

    #[test]
    fn refuses_text_without_a_layout_and_broken_xml() {
        for text in ["", "<sensor/>", "<sensors>", "{ \"layout\": {} }"] {
            let e = parse(text).expect_err(text);
            assert!(matches!(e, ImportError::NoLegacyLayout), "{text}: {e}");
            assert_eq!(e.message(), "The file holds no legacy XML layout.");
        }
        let e = parse("<sensors><sensor side=\"left\"></property></sensors>").expect_err("broken");
        assert!(matches!(e, ImportError::Xml(_)), "{e}");
        assert!(
            e.message()
                .starts_with("The legacy XML layout could not be read: ")
        );
    }

    #[test]
    fn decodes_the_files_text_encoding() {
        assert_eq!(decode("Grüße".as_bytes()), "Grüße");
        assert_eq!(decode(b"\xEF\xBB\xBFabc"), "abc");
        assert_eq!(decode(b"Gr\xFC\xDFe \x80 \x93x\x94"), "Grüße € “x”");
        let utf16le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("Grüße".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode(&utf16le), "Grüße");
        let utf16be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain("ok".encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        assert_eq!(decode(&utf16be), "ok");
    }

    #[test]
    fn reads_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("layout.xml");
        std::fs::write(
            &path,
            b"<sensors><sensor side=\"left\" type=\"DockText\">\
              <property name=\"text\">\xE9t\xE9</property></sensor></sensors>",
        )
        .expect("written");
        assert_eq!(read(&path).expect("read")[0].props["text"], "été");
    }

    #[test]
    fn booleans() {
        for (value, expected) in [
            ("yes", Some(true)),
            ("no", Some(false)),
            (" YES ", Some(true)),
            ("1", Some(true)),
            ("false", Some(false)),
            ("", None),
            ("maybe", None),
        ] {
            assert_eq!(bool(value), expected, "{value}");
        }
    }

    #[test]
    fn colour_modes() {
        assert_eq!(color_mode("off:#000000"), Some(("standard", None)));
        assert_eq!(color_mode("off:"), Some(("standard", None)));
        assert_eq!(
            color_mode("col:#c0c0c0"),
            Some(("custom", Some("#C0C0C0".into())))
        );
        assert_eq!(
            color_mode("dyn:#00FF00"),
            Some(("dynamic", Some("#00FF00".into())))
        );
        assert_eq!(
            color_mode("#123abc"),
            Some(("custom", Some("#123ABC".into())))
        );
        for bad in ["", "col:", "col:red", "xyz:#000000", "#12345"] {
            assert_eq!(color_mode(bad), None, "{bad}");
        }
    }

    #[test]
    fn opacities_become_percentages() {
        for (value, expected) in [
            ("196", 77),
            ("18", 7),
            ("137", 54),
            ("225", 88),
            ("255", 100),
            ("0", 0),
            ("300", 100),
            ("-4", 0),
        ] {
            assert_eq!(opacity(value), Some(expected), "{value}");
        }
        assert_eq!(opacity("half"), None);
    }

    #[test]
    fn numbers_and_intervals() {
        assert_eq!(number(" 10 "), Some(10.0));
        assert_eq!(number("0.5"), Some(0.5));
        assert_eq!(number(""), None);
        assert_eq!(number("NaN"), None);
        assert_eq!(number("1e999"), None);
    }

    #[test]
    fn graph_widths_snap_to_fives() {
        for (value, expected) in [
            ("70", 70),
            ("73", 70),
            ("79.9", 75),
            ("20", 50),
            ("400", 150),
            ("-3", 50),
        ] {
            assert_eq!(graph_width(value), Some(expected), "{value}");
        }
        assert_eq!(graph_width("wide"), None);
    }

    #[test]
    fn paths_and_drives() {
        assert_eq!(path("D:/Icons/Recycle.ico"), "D:\\Icons\\Recycle.ico");
        assert_eq!(path("C:\\x"), "C:\\x");
        assert_eq!(drive("C:\\"), Some("C:\\".into()));
        assert_eq!(drive("d:"), Some("D:\\".into()));
        assert_eq!(drive("e:/"), Some("E:\\".into()));
        assert_eq!(drive("C:\\Windows"), None);
        assert_eq!(drive(""), None);
    }

    #[test]
    fn font_styles() {
        for (value, expected) in [
            ("", Some("normal")),
            ("bold", Some("bold")),
            ("Italic", Some("italic")),
            ("bold, italic", Some("boldItalic")),
            ("italic,bold", Some("boldItalic")),
            ("underline", None),
        ] {
            assert_eq!(font_style(value), expected, "{value}");
        }
    }
}
