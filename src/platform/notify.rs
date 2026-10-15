//! Notifications (SPEC §7.4): the toast's content, built here so it is tested on every OS.

/// `text` with the characters XML reserves escaped.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are not allowed in XML 1.0 (tabs and line breaks are).
            c if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// A toast with `title` and `body` in the generic template; the app's name comes from its
/// AppUserModelID.
pub fn toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text>\
         </binding></visual></toast>",
        escaped(title),
        escaped(body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_and_body_are_escaped() {
        assert_eq!(
            toast_xml("USB <Drive> (E:)", "Tom & Jerry's \"stick\"\u{7}"),
            "<toast><visual><binding template=\"ToastGeneric\"><text>USB &lt;Drive&gt; (E:)</text>\
             <text>Tom &amp; Jerry&apos;s &quot;stick&quot;</text></binding></visual></toast>"
        );
    }
}
