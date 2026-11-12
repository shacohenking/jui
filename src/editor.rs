//! Start the editor on a temporary file and wait. The CLI and the TUI both use it.
//!
//! The editor comes from the Setting `editor.command`. When it is empty, jui uses
//! `$VISUAL`, then `$EDITOR`, then `vi`. The command goes through `sh -c`, so a
//! command with arguments (for example `code --wait`) works.
//!
//! The TUI must stop the terminal mode before it starts the editor, and start it
//! again after. Give the TUI functions to `edit_text_with` as `before` and `after`.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::core::{Error, Result};

/// The editor command: the Setting, then `$VISUAL`, then `$EDITOR`, then `vi`.
pub fn editor_command(setting: &str, visual: Option<String>, editor: Option<String>) -> String {
    [Some(setting.to_string()), visual, editor]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "vi".to_string())
}

/// The editor command from the Setting and the process environment.
pub fn editor_from_env(setting: &str) -> String {
    editor_command(setting, std::env::var("VISUAL").ok(), std::env::var("EDITOR").ok())
}

/// Start the editor on a file and wait until it stops.
pub fn run_editor(command: &str, path: &Path) -> Result<()> {
    // `"$@"` gives the path as one argument, also when it has spaces.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{command} \"$@\""))
        .arg("jui-editor")
        .arg(path)
        .status()
        .map_err(|e| Error::input_hint(format!("Cannot start the editor `{command}`: {e}"), "Set editor.command, $VISUAL, or $EDITOR."))?;
    if !status.success() {
        return Err(Error::input_hint(
            format!("The editor `{command}` stopped with {status}."),
            "jui did not use the text.",
        ));
    }
    Ok(())
}

/// Write `text` to a temporary file with `suffix` (for example `.md`), start the editor,
/// and return the text after the editor stops.
pub fn edit_text(command: &str, text: &str, suffix: &str) -> Result<String> {
    edit_text_with(command, text, "jui-", suffix, || Ok(()), || Ok(()))
}

/// The file name prefix of a Template: `jui-<key or project>-`.
pub fn template_prefix(label: &str) -> String {
    let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    format!("jui-{safe}-")
}

/// Like `edit_text`, but call `before` before the editor starts and `after` when it stops
/// (also when it fails). The TUI uses them to stop and start the terminal mode.
pub fn edit_text_with(
    command: &str,
    text: &str,
    prefix: &str,
    suffix: &str,
    before: impl FnOnce() -> std::io::Result<()>,
    after: impl FnOnce() -> std::io::Result<()>,
) -> Result<String> {
    let io = |e: std::io::Error| Error::input(format!("Cannot write the temporary file: {e}"));
    let mut file = tempfile::Builder::new().prefix(prefix).suffix(suffix).tempfile().map_err(io)?;
    file.write_all(text.as_bytes()).map_err(io)?;
    file.flush().map_err(io)?;

    before().map_err(|e| Error::input(format!("Cannot stop the terminal mode: {e}")))?;
    let result = run_editor(command, file.path());
    let restored = after();
    result?;
    restored.map_err(|e| Error::input(format!("Cannot start the terminal mode again: {e}")))?;

    std::fs::read_to_string(file.path()).map_err(|e| Error::input(format!("Cannot read the temporary file: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_priority() {
        assert_eq!(editor_command("hx", Some("code -w".into()), Some("nano".into())), "hx");
        assert_eq!(editor_command("", Some("code -w".into()), Some("nano".into())), "code -w");
        assert_eq!(editor_command(" ", None, Some("nano".into())), "nano");
        assert_eq!(editor_command("", Some("".into()), None), "vi");
    }

    #[test]
    fn edit_text_returns_the_changed_file() {
        // A fake editor: append a line to the file.
        let out = edit_text("sh -c 'echo changed >> \"$1\"' x", "first\n", ".md").unwrap();
        assert_eq!(out, "first\nchanged\n");
    }

    #[test]
    fn before_and_after_run_around_the_editor() {
        use std::cell::RefCell;
        let log = RefCell::new(Vec::new());
        let out = edit_text_with(
            "true",
            "x",
            "jui-",
            ".txt",
            || {
                log.borrow_mut().push("before");
                Ok(())
            },
            || {
                log.borrow_mut().push("after");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(out, "x");
        assert_eq!(*log.borrow(), vec!["before", "after"]);
    }

    #[test]
    fn a_failed_editor_is_an_error_and_after_still_runs() {
        let mut after_ran = false;
        let e = edit_text_with("false", "x", "jui-", ".txt", || Ok(()), || {
            after_ran = true;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(e.kind(), "input");
        assert!(after_ran);
    }
}
