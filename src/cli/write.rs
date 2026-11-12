//! `jui new` and `jui edit`: the flags, the Template in the editor, `--from`, the diff,
//! and the Drafts. See ticket 08 "Template format for create and edit" and ticket 09
//! "Edit policy for Markdown text and wiki markup".
//!
//! The rules:
//! - `new` opens the Template when `-s` is not given. `edit` opens it when no field flag is given.
//! - `-e` always opens the Template, filled with the flags.
//! - `--from <file>` uses the file. jui opens the editor only when the file is not valid.
//! - In a script, jui never opens the editor. A Template that is not valid stops with the
//!   errors and a Draft.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use super::args::{EditArgs, FieldArgs, NewArgs, WriteArgs};
use super::prompt;
use crate::core::jira::AssignScope;
use crate::core::model::{EditData, looks_like_key, normalize_key};
use crate::core::template::{self as tpl, DescChange, FlagValue, Preset, Request, Session, Step};
use crate::core::{Error, Jira, Result};

/// Start the editor on a Template (the text and the label) and return the saved text.
pub type EditorFn<'a> = &'a mut dyn FnMut(&str, &str) -> Result<String>;

/// The things that the driver needs from the terminal. Tests give fakes.
pub struct Io<'a> {
    /// Start the editor on the text and return the saved text. `None` in a script.
    pub editor: Option<EditorFn<'a>>,
    /// Answers to questions. `None` in a script.
    pub input: Option<&'a mut dyn BufRead>,
    /// Messages, questions, and diffs (stderr).
    pub out: &'a mut dyn Write,
    /// Use colors in the diff.
    pub color: bool,
    /// The Draft directory.
    pub drafts: PathBuf,
    /// The text of stdin for `-d -`.
    pub stdin: Option<String>,
}

/// The result of `new` or `edit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Created(String),
    Edited { key: String, changed: Vec<String> },
    /// Nothing was sent. The text is the message.
    Cancelled(String),
}

/// The Preset from the field flags.
pub fn preset(f: &FieldArgs, stdin: Option<&str>) -> Result<Preset> {
    let mut p = Preset::default();
    let mut put = |k: &str, v: FlagValue| p.values.push((k.to_string(), v));
    if let Some(s) = &f.summary {
        put("summary", FlagValue::Text(s.clone()));
    }
    if !f.components.is_empty() {
        put("components", FlagValue::List(f.components.clone()));
    }
    if let Some(a) = &f.assignee {
        put("assignee", FlagValue::Text(a.clone()));
    }
    if let Some(e) = &f.epic {
        put("epic", FlagValue::Text(e.clone()));
    }
    if !f.labels.is_empty() {
        put("labels", FlagValue::List(f.labels.clone()));
    }
    if let Some(pr) = &f.priority {
        put("priority", FlagValue::Text(pr.clone()));
    }
    for s in &f.set {
        p.set(s)?;
    }
    p.description = match f.description.as_deref() {
        Some("-") => Some(
            stdin
                .map(str::to_string)
                .ok_or_else(|| Error::input("`-d -` reads stdin, but stdin is empty."))?
                .trim_end()
                .to_string(),
        ),
        Some(d) => Some(d.to_string()),
        None => None,
    };
    Ok(p)
}

/// Change the `-a` text to one username with the user rules (`me` and `none` stay).
/// Only a search text sends a request. More than one match asks on a terminal.
async fn resolve_assignee(j: &Jira, p: &mut Preset, scope: AssignScope, io: &mut Io<'_>) -> Result<()> {
    let Some(i) = p.values.iter().position(|(k, _)| k == "assignee") else { return Ok(()) };
    let FlagValue::Text(arg) = p.values[i].1.clone() else { return Ok(()) };
    let mut ask = super::people::Ask { input: io.input.as_deref_mut(), out: &mut *io.out };
    let name = super::people::assignee_flag(j, &scope, &arg, &mut ask).await?;
    p.values[i].1 = FlagValue::Text(name);
    Ok(())
}

/// Read stdin when `-d -` is given.
pub fn read_stdin_if(f: &FieldArgs) -> Option<String> {
    read_stdin_when(f.description.as_deref() == Some("-"))
}

/// Read all of stdin when `want` is true.
pub fn read_stdin_when(want: bool) -> Option<String> {
    if !want {
        return None;
    }
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).ok().map(|_| s)
}

pub fn read_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| Error::input(format!("Cannot read {}: {e}", path.display())))
}

/// Run the Template loop until jui can send a request, or it stops.
/// `first` is the text to check first (from `--from` or from the flags); with no `first`,
/// jui opens the editor at once.
fn run_loop(s: &mut Session, io: &mut Io<'_>, first: Option<String>, cmd: &str) -> Result<Option<Request>> {
    let mut next = first;
    loop {
        let step = match next.take() {
            Some(text) => s.submit_given(&text),
            None => {
                let Some(ed) = io.editor.as_deref_mut() else {
                    unreachable!("the caller gives a first text in a script")
                };
                let saved = match ed(s.text(), s.label()) {
                    Ok(t) => t,
                    Err(e) => return Err(stop(s, io, cmd, e)),
                };
                s.submit(&saved)
            }
        };
        match step {
            Step::Send(r) => return Ok(Some(r)),
            Step::Cancel(m) => {
                let _ = writeln!(io.out, "{m}");
                return Ok(None);
            }
            Step::Reopen if io.editor.is_some() => continue,
            Step::Reopen => {
                let msg = format!("The Template is not valid:\n{}", tpl::errors_text(s.errors()));
                return Err(stop(s, io, cmd, Error::input(msg)));
            }
            Step::Stop(m) => return Err(stop(s, io, cmd, Error::input(m))),
        }
    }
}

/// Write a Draft and add its path to the hint of the error.
fn stop(s: &Session, io: &mut Io<'_>, cmd: &str, e: Error) -> Error {
    let message = e.to_string();
    match tpl::write_draft(&io.drafts, s.label(), s.draft_text()) {
        Ok(path) => Error::input_hint(message, tpl::draft_hint(&path, cmd)),
        Err(d) => Error::input_hint(message, format!("jui could not write a Draft: {d}")),
    }
}

/// Show the diff of a description change, and ask. Returns false when the user says no.
fn confirm_desc(d: &DescChange, yes: bool, io: &mut Io<'_>) -> Result<bool> {
    if !d.needs_diff() || yes {
        return Ok(true);
    }
    let _ = writeln!(io.out, "The description changes:");
    for l in tpl::diff(&d.old, &d.new) {
        let line = format!("{}{}", l.tag, l.text);
        let line = match (io.color, l.tag) {
            (true, '-') => format!("\x1b[31m{line}\x1b[0m"),
            (true, '+') => format!("\x1b[32m{line}\x1b[0m"),
            _ => line,
        };
        let _ = writeln!(io.out, "{line}");
    }
    for w in &d.warnings {
        let _ = writeln!(io.out, "warning: {w}");
    }
    let Some(input) = io.input.as_deref_mut() else {
        return Err(Error::input_hint("jui must ask before it changes the description.", "Use --yes to send the change."));
    };
    let answer = prompt::ask(input, io.out, "Send this change? [y/N] ")?;
    Ok(matches!(answer.to_lowercase().as_str(), "y" | "yes"))
}

/// True when jui must open the editor first.
fn wants_editor(w: &WriteArgs, need_flags: bool) -> bool {
    w.editor || (w.from.is_none() && need_flags)
}

/// `jui new`.
pub async fn new(j: &Jira, a: &NewArgs, default_project: &str, default_type: &str, io: &mut Io<'_>) -> Result<Outcome> {
    let mut p = preset(&a.common.fields, io.stdin.as_deref())?;
    let parent = a.parent.as_deref().map(|k| normalize_key(k, default_project));
    if let Some(k) = &parent {
        if !looks_like_key(k) {
            return Err(Error::input(format!("`{k}` is not a Ticket key.")));
        }
        p.values.push(("parent".into(), FlagValue::Text(k.clone())));
    }
    // A parent gives a Sub-task in the project of the parent, unless the flags tell otherwise.
    let project = match (&a.project, &parent) {
        (Some(pr), _) => pr.clone(),
        (None, Some(k)) => k.split('-').next().unwrap_or(default_project).to_string(),
        (None, None) => default_project.to_string(),
    };
    let kind = match (&a.kind, &parent) {
        (Some(t), _) => t.clone(),
        (None, Some(_)) => "subtask".to_string(),
        (None, None) => default_type.to_string(),
    };
    resolve_assignee(j, &mut p, AssignScope::Project(project.trim().to_uppercase()), io).await?;
    let (meta, me) = tokio::try_join!(j.create_meta(&project, &kind), j.myself())?;
    let mut s = Session::create(meta, &me, &p);
    let editor = wants_editor(&a.common, a.common.fields.summary.is_none());
    let first = match &a.common.from {
        Some(path) => Some(read_file(path)?),
        None if editor && io.editor.is_some() => None,
        None if editor => {
            return Err(Error::input_hint(
                "jui needs a terminal to open the Template.",
                "Give -s and the required fields with flags (--set), or use --from <file>.",
            ));
        }
        None => Some(s.text().to_string()),
    };
    let mut first = first;
    loop {
        let Some(r) = run_loop(&mut s, io, first.take(), "new")? else {
            return Ok(Outcome::Cancelled("Cancelled. No Ticket created.".into()));
        };
        let Request::Create(body) = r else { unreachable!() };
        match j.create(&body).await {
            Ok(key) => return Ok(Outcome::Created(key)),
            Err(e) => match s.rejected(e) {
                Ok(_) if io.editor.is_some() => continue,
                Ok(_) => {
                    let msg = format!("Jira did not accept the Ticket:\n{}", tpl::errors_text(s.errors()));
                    return Err(stop(&s, io, "new", Error::input(msg)));
                }
                Err(e) => return Err(stop(&s, io, "new", e)),
            },
        }
    }
}

/// `jui edit`.
pub async fn edit(j: &Jira, a: &EditArgs, key: &str, io: &mut Io<'_>) -> Result<Outcome> {
    let mut p = preset(&a.common.fields, io.stdin.as_deref())?;
    resolve_assignee(j, &mut p, AssignScope::Ticket(key.to_string()), io).await?;
    let (data, me) = tokio::try_join!(j.edit_data(key), j.myself())?;
    let mut s = Session::edit(data.clone(), &me, a.wiki, &p);
    let editor = wants_editor(&a.common, a.common.fields.is_empty());
    let first = match &a.common.from {
        Some(path) => Some(read_file(path)?),
        None if editor && io.editor.is_some() => None,
        None if editor => {
            return Err(Error::input_hint(
                "jui needs a terminal to open the Template.",
                "Give the fields with flags (for example -s or --set), or use --from <file>.",
            ));
        }
        None => Some(s.text().to_string()),
    };
    let mut first = first;
    loop {
        let Some(r) = run_loop(&mut s, io, first.take(), "edit")? else {
            return Ok(Outcome::Cancelled("No changes.".into()));
        };
        let Request::Edit(plan) = r else { unreachable!() };
        if let Some(d) = &plan.description
            && !confirm_desc(d, a.common.yes, io)?
        {
            if io.editor.is_some() {
                // `n` opens the editor again with the text of the user and one error line.
                let note = "You did not send the change. Edit the text, or empty the file to cancel.";
                s.reopen_note(note);
                continue;
            }
            return Err(stop(&s, io, &format!("edit {key}"), Error::Cancelled));
        }
        check_changed_in_jira(j, &data, key, &s, io).await?;
        match j.edit(key, &plan.body()).await {
            Ok(()) => return Ok(Outcome::Edited { key: key.to_string(), changed: changed_names(&plan) }),
            Err(e) => match s.rejected(e) {
                Ok(_) if io.editor.is_some() => continue,
                Ok(_) => {
                    let msg = format!("Jira did not accept the change:\n{}", tpl::errors_text(s.errors()));
                    return Err(stop(&s, io, &format!("edit {key}"), Error::input(msg)));
                }
                Err(e) => return Err(stop(&s, io, &format!("edit {key}"), e)),
            },
        }
    }
}

fn changed_names(plan: &tpl::EditPlan) -> Vec<String> {
    let mut c = plan.changed.clone();
    if plan.description.is_some() {
        c.push("description".into());
    }
    c
}

/// Read the Ticket again. If it changed in Jira during the edit, tell the user and ask.
async fn check_changed_in_jira(j: &Jira, before: &EditData, key: &str, s: &Session, io: &mut Io<'_>) -> Result<()> {
    let now = j.edit_data(key).await?;
    if now.updated == before.updated {
        return Ok(());
    }
    let fields = tpl::changed_in_jira(before, &now);
    let what = if fields.is_empty() { String::new() } else { format!(" Changed: {}.", fields.join(", ")) };
    let _ = writeln!(io.out, "{key} changed in Jira while you edited it.{what}");
    let Some(input) = io.input.as_deref_mut() else {
        return Err(stop(s, io, &format!("edit {key}"), Error::input(format!("{key} changed in Jira while you edited it.{what}"))));
    };
    let answer = prompt::ask(input, io.out, "Send your changes? [y/N] ")?;
    if matches!(answer.to_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(stop(s, io, &format!("edit {key}"), Error::Cancelled))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_from_flags() {
        let f = FieldArgs {
            summary: Some("S".into()),
            components: vec!["A".into(), "B".into()],
            labels: vec!["x".into()],
            set: vec!["severity=Major".into()],
            description: Some("-".into()),
            ..Default::default()
        };
        let p = preset(&f, Some("Text\n\n")).unwrap();
        assert_eq!(p.description.as_deref(), Some("Text"));
        assert_eq!(p.values[0], ("summary".into(), FlagValue::Text("S".into())));
        assert_eq!(p.values[1], ("components".into(), FlagValue::List(vec!["A".into(), "B".into()])));
        assert_eq!(p.values.last().unwrap(), &("severity".to_string(), FlagValue::Text("Major".into())));
        assert!(preset(&f, None).is_err(), "-d - with no stdin");
    }
}
