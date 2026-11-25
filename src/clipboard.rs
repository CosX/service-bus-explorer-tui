use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

use arboard::Clipboard;
use base64::Engine;

/// Process-wide clipboard handle. Kept alive so X11 selection ownership is not
/// dropped immediately after `set_text` (which would empty the clipboard).
fn shared_clipboard() -> &'static Mutex<Option<Clipboard>> {
    static CLIPBOARD: OnceLock<Mutex<Option<Clipboard>>> = OnceLock::new();
    CLIPBOARD.get_or_init(|| Mutex::new(None))
}

/// Copy text to the system clipboard.
///
/// Tries the native clipboard first (`arboard` — X11/Wayland/macOS/Windows),
/// then falls back to OSC 52 so copy can still work over SSH / remote terminals.
pub fn copy_text(text: &str) -> Result<(), String> {
    match copy_native(text) {
        Ok(()) => Ok(()),
        Err(native_err) => match copy_osc52(text) {
            Ok(()) => Ok(()),
            Err(osc_err) => Err(format!("{native_err}; OSC 52 fallback also failed: {osc_err}")),
        },
    }
}

fn copy_native(text: &str) -> Result<(), String> {
    let mut guard = shared_clipboard()
        .lock()
        .map_err(|_| "clipboard lock poisoned".to_string())?;

    if guard.is_none() {
        *guard = Some(Clipboard::new().map_err(|e| format!("clipboard init: {e}"))?);
    }

    guard
        .as_mut()
        .expect("clipboard just initialized")
        .set_text(text.to_owned())
        .map_err(|e| format!("clipboard set: {e}"))?;
    Ok(())
}

fn copy_osc52(text: &str) -> io::Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let seq = osc52_sequence_with_tmux(&encoded, std::env::var_os("TMUX").is_some());
    let mut out = io::stdout();
    out.write_all(seq.as_bytes())?;
    out.flush()?;
    Ok(())
}

fn osc52_sequence_with_tmux(encoded: &str, in_tmux: bool) -> String {
    // OSC 52: ESC ] 52 ; c ; <base64> BEL
    let osc = format!("\x1b]52;c;{}\x07", encoded);
    if in_tmux {
        // tmux DCS passthrough: ESC P tmux ; ESC <payload with ESC doubled> ESC \
        let escaped = osc.replace('\x1b', "\x1b\x1b");
        format!("\x1bPtmux;{escaped}\x1b\\")
    } else {
        osc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_basic_format() {
        let seq = osc52_sequence_with_tmux("aGVsbG8=", false);
        assert_eq!(seq, "\x1b]52;c;aGVsbG8=\x07");
    }

    #[test]
    fn osc52_tmux_wraps_escape_sequences() {
        let seq = osc52_sequence_with_tmux("dGVzdA==", true);
        assert!(seq.starts_with("\x1bPtmux;"));
        assert!(seq.ends_with("\x1b\\"));
        assert!(seq.contains("\x1b\x1b]52;c;dGVzdA==\x07"));
    }
}
