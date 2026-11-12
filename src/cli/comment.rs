//! `jui comment list|add|edit|delete`. See ticket 05 "jui CLI command tree", ticket 08
//! part 5 (the comment Template), and ticket 09 (the diff before a change of existing text).
//!
//! The rules:
//! - `add` with a text sends it at once. With no text (or with `-e`), jui opens the Template.
//! - `edit` with no text opens the Template with the comment. With a text, jui uses it.
//! - `--from <file>` uses the file. jui opens the editor only when the file is not valid.
//! - An edit shows the diff of the wiki text and asks. `--yes` skips the question.
//! - `delete` asks. `--yes` skips the question. A script without `--yes` stops.
//! - In a script, jui never opens the editor or asks.

use super::prompt;
use super::write::{Io, read_file};
use crate::core::comment::{self as cm, CommentPlan, CommentSession, CommentStep};
use crate::core::model::Comment;
use crate::core::template as tpl;
use crate::core::{Error, Jira, Result};

/// The result of `add` or `edit`.
#[derive(Debug, Clone)]
pub enum Outcome {
    Added(Comment),
    Edited(Comment),
    /// Nothing was sent. The text is the message.
    Cancelled(String),
}

/// The text of a `MARKDOWN|-` argument: `-` reads stdin.
fn text_arg(text: Option<&str>, stdin: Option<&str>) -> Result<Option<String>> {
    match text {
        Some("-") => {
            let t = stdin.ok_or_else(|| Error::input("`-` reads stdin, but stdin is empty."))?;
            Ok(Some(t.trim_end().to_string()))
        }
        Some(t) => Ok(Some(t.to_string())),
        None => Ok(None),
    }
}

/// Write a Draft and add its path to the hint of the error.
fn stop(s: &CommentSession, io: &mut Io<'_>, e: Error) -> Error {
    let message = e.to_string();
    match tpl::write_draft(&io.drafts, &s.label(), s.draft_text()) {
        Ok(path) => Error::input_hint(message, tpl::draft_hint(&path, &s.command())),
        Err(d) => Error::input_hint(message, format!("jui could not write a Draft: {d}")),
    }
}

/// Run the Template loop until jui can send a request, or it stops.
fn run_loop(s: &mut CommentSession, io: &mut Io<'_>, first: Option<String>) -> Result<Option<CommentPlan>> {
    let mut next = first;
    loop {
        let step = match next.take() {
            Some(text) => s.submit(&text),
            None => {
                let Some(ed) = io.editor.as_deref_mut() else { unreachable!("the caller gives a first text in a script") };
                match ed(s.text(), &s.label()) {
                    Ok(t) => s.submit(&t),
                    Err(e) => return Err(stop(s, io, e)),
                }
            }
        };
        match step {
            CommentStep::Send(p) => return Ok(Some(p)),
            CommentStep::Cancel(m) => {
                let _ = writeln!(io.out, "{m}");
                return Ok(None);
            }
            CommentStep::Reopen if io.editor.is_some() => continue,
            CommentStep::Reopen => {
                let msg = format!("The comment is not valid:\n{}", tpl::errors_text(s.errors()));
                return Err(stop(s, io, Error::input(msg)));
            }
            CommentStep::Stop(m) => return Err(stop(s, io, Error::input(m))),
        }
    }
}

/// Show the diff of a comment change and the warnings, and ask. False when the user says no.
fn confirm(p: &CommentPlan, yes: bool, io: &mut Io<'_>) -> Result<bool> {
    for w in &p.warnings {
        let _ = writeln!(io.out, "warning: {w}");
    }
    if !p.needs_diff() || yes {
        return Ok(true);
    }
    let _ = writeln!(io.out, "The comment changes:");
    for l in tpl::diff(p.old.as_deref().unwrap_or(""), &p.wiki) {
        let line = format!("{}{}", l.tag, l.text);
        let line = match (io.color, l.tag) {
            (true, '-') => format!("\x1b[31m{line}\x1b[0m"),
            (true, '+') => format!("\x1b[32m{line}\x1b[0m"),
            _ => line,
        };
        let _ = writeln!(io.out, "{line}");
    }
    let Some(input) = io.input.as_deref_mut() else {
        return Err(Error::input_hint("jui must ask before it changes a comment.", "Use --yes to send the change."));
    };
    let answer = prompt::ask(input, io.out, "Send this change? [y/N] ")?;
    Ok(matches!(answer.to_lowercase().as_str(), "y" | "yes"))
}

/// `jui comment add`.
pub async fn add(j: &Jira, a: &super::args::CommentAddArgs, key: &str, io: &mut Io<'_>) -> Result<Outcome> {
    let text = text_arg(a.text.as_deref(), io.stdin.as_deref())?;
    // A text with no `-e` goes at once, with no Template and no extra request.
    if let (Some(t), false) = (&text, a.editor) {
        let Some(p) = cm::to_plan(t, a.wiki) else {
            let _ = writeln!(io.out, "Cancelled. No comment added.");
            return Ok(Outcome::Cancelled("Cancelled. No comment added.".into()));
        };
        for w in &p.warnings {
            let _ = writeln!(io.out, "warning: {w}");
        }
        return j.add_comment(key, &p.wiki).await.map(Outcome::Added);
    }
    let first = match &a.from {
        Some(path) => Some(read_file(path)?),
        None if io.editor.is_some() => None,
        None => {
            return Err(Error::input_hint(
                "jui needs a terminal to open the comment Template.",
                format!("Give the text: `jui comment add {key} \"text\"`, `-` for stdin, or --from <file>."),
            ));
        }
    };
    let ctx = j.comment_context(key, None).await?;
    let mut s = CommentSession::add(&ctx, text.as_deref().unwrap_or(""), a.wiki);
    let mut first = first;
    loop {
        let Some(p) = run_loop(&mut s, io, first.take())? else {
            return Ok(Outcome::Cancelled("Cancelled. No comment added.".into()));
        };
        for w in &p.warnings {
            let _ = writeln!(io.out, "warning: {w}");
        }
        match j.add_comment(key, &p.wiki).await {
            Ok(c) => return Ok(Outcome::Added(c)),
            Err(e) => match s.rejected(e) {
                Ok(_) if io.editor.is_some() => continue,
                Ok(_) => {
                    let msg = format!("Jira did not accept the comment:\n{}", tpl::errors_text(s.errors()));
                    return Err(stop(&s, io, Error::input(msg)));
                }
                Err(e) => return Err(stop(&s, io, e)),
            },
        }
    }
}

/// `jui comment edit`.
pub async fn edit(j: &Jira, a: &super::args::CommentEditArgs, key: &str, io: &mut Io<'_>) -> Result<Outcome> {
    let text = text_arg(a.text.as_deref(), io.stdin.as_deref())?;
    let ctx = j.comment_context(key, Some(&a.id)).await?;
    let mut s = CommentSession::edit(&ctx, &a.id, a.wiki)?;
    let first = match (&a.from, &text) {
        (Some(path), _) => Some(read_file(path)?),
        (None, Some(t)) => Some(t.clone()),
        (None, None) if io.editor.is_some() => None,
        (None, None) => {
            return Err(Error::input_hint(
                "jui needs a terminal to open the comment Template.",
                format!("Give the new text: `jui comment edit {key} {} \"text\"`, `-` for stdin, or --from <file>.", a.id),
            ));
        }
    };
    let mut first = first;
    loop {
        let Some(p) = run_loop(&mut s, io, first.take())? else {
            return Ok(Outcome::Cancelled("No changes.".into()));
        };
        let ok = match confirm(&p, a.yes, io) {
            Ok(ok) => ok,
            Err(e) => {
                // Keep the hint (`--yes`) and add the Draft.
                let hint = e.hint().unwrap_or_default();
                let e = stop(&s, io, e);
                return Err(Error::input_hint(e.to_string(), format!("{hint} {}", e.hint().unwrap_or_default()).trim().to_string()));
            }
        };
        if !ok {
            if io.editor.is_some() {
                s.reopen_note("You did not send the change. Edit the text, or empty the file to cancel.");
                continue;
            }
            return Err(stop(&s, io, Error::Cancelled));
        }
        // The comment can have changed in Jira while the editor was open.
        let now = j.comment(key, &a.id).await?;
        if let Some(msg) = cm::changed_in_jira(key, &p, &now) {
            let _ = writeln!(io.out, "{msg}");
            let Some(input) = io.input.as_deref_mut() else {
                return Err(stop(&s, io, Error::input(msg)));
            };
            let answer = prompt::ask(input, io.out, "Send your change? [y/N] ")?;
            if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
                return Err(stop(&s, io, Error::Cancelled));
            }
        }
        match j.edit_comment(key, &a.id, &p.wiki).await {
            Ok(c) => return Ok(Outcome::Edited(c)),
            Err(e) => match s.rejected(e) {
                Ok(_) if io.editor.is_some() => continue,
                Ok(_) => {
                    let msg = format!("Jira did not accept the change:\n{}", tpl::errors_text(s.errors()));
                    return Err(stop(&s, io, Error::input(msg)));
                }
                Err(e) => return Err(stop(&s, io, e)),
            },
        }
    }
}

/// `jui comment delete`: ask first on a terminal. A script needs `--yes`.
/// Returns false when the user says no.
pub async fn delete(j: &Jira, key: &str, id: &str, yes: bool, io: &mut Io<'_>) -> Result<bool> {
    if !yes {
        let Some(input) = io.input.as_deref_mut() else {
            return Err(Error::input_hint(
                format!("jui must ask before it deletes comment {id} on {key}."),
                "Use --yes to delete it. A delete cannot be reversed.",
            ));
        };
        // Show the comment, so that the user knows what goes.
        let c = j.comment(key, id).await?;
        let _ = writeln!(io.out, "@{}: {}", cm::author(&c), cm::first_line(&c, 70));
        let answer = prompt::ask(input, io.out, &format!("Delete comment {id} on {key}? [y/N] "))?;
        if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
            return Ok(false);
        }
    }
    j.delete_comment(key, id).await?;
    Ok(true)
}
