/// Bidirectional-override and isolate controls. Not `is_control()`, but they
/// reorder how surrounding text is displayed and can disguise content.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}')
}

/// Largest char boundary in `s` at or below `idx`, so slicing never panics.
pub fn floor_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A single-line form value with a cursor marker, sanitized for display.
pub fn value_with_cursor(value: &str, cursor: usize) -> String {
    let cursor = floor_char_boundary(value, cursor);
    let (before, after) = value.split_at(cursor);
    format!(
        "{}▏{}",
        sanitize_for_terminal(before, false),
        sanitize_for_terminal(after, false)
    )
}

/// Sanitizes untrusted text for safe terminal display.
///
/// Goals:
/// - prevent terminal escape injection (CSI/OSC/etc)
/// - remove other control characters (except optional newlines)
/// - neutralise bidirectional overrides that disguise text
/// - keep output reasonably readable/debuggable via placeholders
pub fn sanitize_for_terminal(input: &str, allow_newlines: bool) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // ESC
            '\x1b' => {
                match chars.peek().copied() {
                    // CSI: ESC [ ... <final>
                    Some('[') => {
                        let _ = chars.next();
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                        out.push_str("[CSI]");
                    }
                    // OSC: ESC ] ... BEL | ESC \
                    Some(']') => {
                        let _ = chars.next();
                        while let Some(c) = chars.next() {
                            if c == '\x07' {
                                break;
                            }
                            if c == '\x1b' && matches!(chars.peek().copied(), Some('\\')) {
                                let _ = chars.next();
                                break;
                            }
                        }
                        out.push_str("[OSC]");
                    }
                    // Other escape: consume a single following char if present
                    Some(_) => {
                        let _ = chars.next();
                        out.push_str("[ESC]");
                    }
                    None => out.push_str("[ESC]"),
                }
            }

            c if is_bidi_control(c) => out.push('�'),

            // Other control characters
            c if c.is_control() => {
                if (allow_newlines && c == '\n') || c == '\t' || c == '\r' {
                    out.push(c);
                } else {
                    out.push('�');
                }
            }

            _ => out.push(ch),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_terminal;

    #[test]
    fn strips_csi_and_osc_escape_sequences() {
        assert_eq!(
            sanitize_for_terminal("hello\x1b[31mred\x1b[0m world", false),
            "hello[CSI]red[CSI] world"
        );
        assert_eq!(
            sanitize_for_terminal("title\x1b]0;tab title\x07done", false),
            "title[OSC]done"
        );
    }

    #[test]
    fn preserves_allowed_whitespace_and_replaces_other_controls() {
        assert_eq!(sanitize_for_terminal("a\nb\t\rc\x00", true), "a\nb\t\rc�");
        assert_eq!(sanitize_for_terminal("a\nb", false), "a�b");
    }

    #[test]
    fn handles_unknown_escape_sequences() {
        assert_eq!(sanitize_for_terminal("x\x1bz", false), "x[ESC]");
        assert_eq!(sanitize_for_terminal("x\x1b", false), "x[ESC]");
    }

    #[test]
    fn strips_bidi_overrides() {
        assert_eq!(sanitize_for_terminal("abc\u{202E}fed", false), "abc�fed");
        assert_eq!(sanitize_for_terminal("x\u{2066}y", false), "x�y");
    }

    #[test]
    fn cursor_inside_a_multibyte_char_does_not_panic() {
        let s = "æøå";
        assert_eq!(super::floor_char_boundary(s, 1), 0);
        assert_eq!(super::floor_char_boundary(s, 3), 2);
        assert_eq!(super::floor_char_boundary(s, 99), s.len());
        assert_eq!(super::value_with_cursor(s, 3), "æ▏øå");
    }

    #[test]
    fn form_values_are_sanitized() {
        assert_eq!(super::value_with_cursor("a\x1b[31mb", 0), "▏a[CSI]b");
    }
}
