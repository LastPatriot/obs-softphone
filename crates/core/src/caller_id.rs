// SPDX-License-Identifier: GPL-2.0-or-later
//! Caller name from the INVITE's From/remote URI, e.g. `"Mrs Ade" <sip:callin@host>`.

/// Returns the display name, else the URI's user part, else the input trimmed.
pub fn display_name(remote: &str) -> String {
    let s = remote.trim();

    if let Some(rest) = s.strip_prefix('"') {
        let mut name = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        name.push(escaped);
                    }
                }
                '"' => break,
                _ => name.push(c),
            }
        }
        let name = name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    } else if let Some(lt) = s.find('<') {
        let name = s[..lt].trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }

    let uri = match (s.find('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => &s[a + 1..b],
        _ => s,
    };
    let after_scheme = uri.split_once(':').map_or(uri, |(_, rest)| rest);
    match after_scheme.split_once('@') {
        Some((user, _)) if !user.is_empty() => user.to_string(),
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::display_name;

    #[test]
    fn quoted_name() {
        assert_eq!(display_name(r#""Mrs Ade" <sip:callin@radio.example>"#), "Mrs Ade");
    }

    #[test]
    fn name_with_location_and_escapes() {
        assert_eq!(
            display_name(r#""Tunde \"T\" — Lagos" <sip:callin@x>"#),
            r#"Tunde "T" — Lagos"#
        );
    }

    #[test]
    fn unquoted_name() {
        assert_eq!(display_name("Bola <sip:callin@x>"), "Bola");
    }

    #[test]
    fn no_name_falls_back_to_user() {
        assert_eq!(display_name("<sip:callin@x>"), "callin");
        assert_eq!(display_name("sip:callin@x;transport=tls"), "callin");
        assert_eq!(display_name(r#""" <sip:callin@x>"#), "callin");
    }

    #[test]
    fn garbage_is_returned_as_is() {
        assert_eq!(display_name("  weird  "), "weird");
    }
}
