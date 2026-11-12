//! Copy text to the clipboard. The TUI uses it for `y`.
//!
//! jui uses no clipboard crate (ticket 20: `arboard` could not be downloaded, and it needs
//! native libraries). It tries these, in this order:
//! 1. A system command: `pbcopy` (macOS), `wl-copy` (Wayland), `xclip`, or `xsel` (X11).
//! 2. The OSC 52 terminal sequence. Most terminals (iTerm2, kitty, WezTerm, Alacritty,
//!    tmux with `set-clipboard on`) accept it, also over SSH.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::core::{Error, Result};

/// How jui copied the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Command(&'static str),
    Osc52,
}

/// The system commands, in the order that jui tries them.
const COMMANDS: &[(&str, &[&str])] = &[
    ("pbcopy", &[]),
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

/// Copy the text. `terminal` gets the OSC 52 sequence when no command works.
pub fn copy(text: &str, terminal: &mut dyn Write) -> Result<Via> {
    for (cmd, args) in COMMANDS {
        if run(cmd, args, text) {
            return Ok(Via::Command(cmd));
        }
    }
    terminal
        .write_all(osc52(text).as_bytes())
        .and_then(|_| terminal.flush())
        .map_err(|e| Error::input(format!("Cannot copy to the clipboard: {e}")))?;
    Ok(Via::Osc52)
}

/// Run one command with the text on its stdin. False when the command is not there or fails.
fn run(cmd: &str, args: &[&str], text: &str) -> bool {
    let child = Command::new(cmd).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else { return false };
    let wrote = child.stdin.take().is_some_and(|mut s| s.write_all(text.as_bytes()).is_ok());
    child.wait().is_ok_and(|s| s.success()) && wrote
}

/// The OSC 52 sequence that sets the clipboard: `ESC ] 52 ; c ; <base64> BEL`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Standard base64 with padding (RFC 4648). A key is short, so a crate is not necessary.
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"BCE-62886"), "QkNFLTYyODg2");
    }

    #[test]
    fn osc52_sequence() {
        assert_eq!(osc52("BCE-1"), "\x1b]52;c;QkNFLTE=\x07");
    }
}
