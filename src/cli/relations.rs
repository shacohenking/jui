//! `jui subtask`, `jui link`, and `jui worklog`. See ticket 24 "Build: Sub-tasks, Links,
//! and Worklogs".
//!
//! The rules:
//! - `link add A <relation> B`: the relation matches as `core::link::find` tells. More
//!   matches ask on a terminal and stop in a script. A Link that exists is not sent again.
//! - `link delete` and `worklog delete` ask on a terminal. `--yes` skips the question. A
//!   script without `--yes` stops.
//! - `worklog add <key> <time>` sends at once. With no time (or `-e`), jui opens the Template.
//! - `worklog edit <key> <id>` with no time and no flags opens the Template. A change of an
//!   existing comment shows the diff and asks.
//! - In a script, jui never opens the editor or asks.

use super::args::{AdjustArg, WorklogFlags};
use super::prompt;
use super::write::{Io, read_file};
use crate::core::format::human_duration;
use crate::core::link::{self as lk, Found, LinkPlan};
use crate::core::model::{Link, Related, Worklog};
use crate::core::template as tpl;
use crate::core::worklog::{self as wl, Clock, Quick, WorklogPlan, WorklogSession, WorklogStep};
use crate::core::{Error, Jira, Result};

// ------------------------------------------------------------------ Sub-tasks

/// The lines of `jui subtask list`.
pub fn render_subtasks(subs: &[Related]) -> String {
    let kw = subs.iter().map(|s| s.key.len()).max().unwrap_or(0);
    let sw = subs.iter().map(|s| s.status.chars().count()).max().unwrap_or(0);
    subs.iter().map(|s| format!("{:<kw$}  {:<sw$}  {}", s.key, s.status, s.summary)).collect::<Vec<_>>().join("\n")
}

// ------------------------------------------------------------------ Links

/// The lines of `jui link list`: the ID, the relation, the other key, the status, the summary.
pub fn render_links(links: &[Link]) -> String {
    let iw = links.iter().map(|l| l.id.len()).max().unwrap_or(0);
    let rw = links.iter().map(|l| l.relation.chars().count()).max().unwrap_or(0);
    let kw = links.iter().map(|l| l.ticket.key.len()).max().unwrap_or(0);
    let sw = links.iter().map(|l| l.ticket.status.chars().count()).max().unwrap_or(0);
    links
        .iter()
        .map(|l| format!("{:<iw$}  {:<rw$}  {:<kw$}  {:<sw$}  {}", l.id, l.relation, l.ticket.key, l.ticket.status, l.ticket.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The lines of `jui link types`.
pub fn render_types(types: &[crate::core::model::LinkType]) -> String {
    let nw = types.iter().map(|t| t.name.chars().count()).max().unwrap_or(0);
    let ow = types.iter().map(|t| t.outward.chars().count()).max().unwrap_or(0);
    types.iter().map(|t| format!("{:<nw$}  {:<ow$}  {}", t.name, t.outward, t.inward)).collect::<Vec<_>>().join("\n")
}

/// The result of `link add`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOutcome {
    Added(String),
    /// The Link exists. Nothing was sent.
    Exists(String),
}

/// `jui link add`.
pub async fn add_link(j: &Jira, from: &str, relation: &str, to: &str, comment: Option<&str>, io: &mut Io<'_>) -> Result<LinkOutcome> {
    let types = j.link_types().await?;
    let rel = match lk::find(&types, relation) {
        Found::One(r) => r,
        Found::None => return Err(lk::not_found(relation, &types, &[])),
        Found::Many(many) => {
            let Some(input) = io.input.as_deref_mut() else { return Err(lk::not_found(relation, &types, &many)) };
            let items: Vec<String> = many.iter().map(lk::Relation::label).collect();
            let i = prompt::select(input, io.out, &format!("More than one Link relation matches `{relation}`:"), &items)?;
            many[i].clone()
        }
    };
    let wiki = comment.map(|c| crate::core::markup::markdown_to_wiki(c).text);
    let plan = LinkPlan::new(from, rel, to, wiki)?;
    // A Link that exists is not sent again (Jira would make a second one).
    let now = j.links(from).await?;
    if plan.exists_in(&now.links) {
        return Ok(LinkOutcome::Exists(plan.describe()));
    }
    j.add_link(&plan).await?;
    Ok(LinkOutcome::Added(plan.describe()))
}

/// `jui link delete`: the Link by ID or by the other key. Returns the description, or `None`
/// when the user says no.
pub async fn delete_link(j: &Jira, key: &str, arg: &str, yes: bool, io: &mut Io<'_>) -> Result<Option<String>> {
    let tl = j.links(key).await?;
    let found = lk::select(&tl.links, arg);
    let link = match found.as_slice() {
        [] => {
            return Err(Error::input_hint(
                format!("{key} has no Link `{arg}`."),
                format!("`jui link list {key}` shows the Link IDs."),
            ));
        }
        [one] => (*one).clone(),
        many => {
            let Some(input) = io.input.as_deref_mut() else {
                return Err(Error::input_hint(
                    format!("{key} has {} Links to {arg}.", many.len()),
                    format!("Give the Link ID. `jui link list {key}` shows it."),
                ));
            };
            let items: Vec<String> = many.iter().map(|l| format!("{} ({})", lk::describe(key, l), l.id)).collect();
            let i = prompt::select(input, io.out, "Select the Link to delete:", &items)?;
            many[i].clone()
        }
    };
    let what = lk::describe(key, &link);
    if !yes {
        let Some(input) = io.input.as_deref_mut() else {
            return Err(Error::input_hint(format!("jui must ask before it deletes the Link \"{what}\"."), "Use --yes to delete it. A delete cannot be reversed."));
        };
        let answer = prompt::ask(input, io.out, &format!("Delete the Link \"{what}\" ({})? [y/N] ", link.id))?;
        if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
            return Ok(None);
        }
    }
    j.delete_link(&link.id).await?;
    Ok(Some(what))
}

// ------------------------------------------------------------------ Worklogs

/// The lines of `jui worklog list`, and the total.
pub fn render_worklogs(ws: &[Worklog]) -> String {
    let iw = ws.iter().map(|w| w.id.len()).max().unwrap_or(0);
    let tw = ws.iter().map(|w| human_duration(w.time_spent_seconds).len()).max().unwrap_or(0);
    let mut out: Vec<String> = ws
        .iter()
        .map(|w| {
            let day = w.started.map(|d| d.format(wl::STARTED_FORMAT).to_string()).unwrap_or_default();
            let who = w.author.as_ref().map(|u| u.name.as_str()).unwrap_or("?");
            let c = wl::first_line(w, 60);
            format!("{:<iw$}  {day}  {:<tw$}  @{who}  {c}", w.id, human_duration(w.time_spent_seconds)).trim_end().to_string()
        })
        .collect();
    let total: u64 = ws.iter().map(|w| w.time_spent_seconds).sum();
    out.push(format!("Total: {} in {} Worklog{}", human_duration(total), ws.len(), if ws.len() == 1 { "" } else { "s" }));
    out.join("\n")
}

/// The result of `worklog add` or `worklog edit`.
#[derive(Debug, Clone)]
pub enum WorklogOutcome {
    Added(Worklog, WorklogPlan),
    Edited(Worklog, WorklogPlan),
    Cancelled(String),
}

/// The values of the flags. `-m -` reads stdin.
fn quick(time: Option<&str>, f: &WorklogFlags, stdin: Option<&str>) -> Result<Quick> {
    let comment = match f.comment.as_deref() {
        Some("-") => stdin.map(|s| s.trim_end().to_string()).ok_or_else(|| Error::input("`-m -` reads stdin, but stdin is empty."))?,
        Some(c) => c.to_string(),
        None => String::new(),
    };
    Ok(Quick {
        time_spent: time.unwrap_or("").to_string(),
        started: f.started.clone().unwrap_or_default(),
        comment,
        adjust: f.adjust.map(AdjustArg::name).map(str::to_string),
        new_estimate: f.new_estimate.clone(),
        reduce_by: f.reduce_by.clone(),
    })
}

/// An error of a flag, with the flag name in the hint.
fn flag_error((key, msg): (String, String)) -> Error {
    let flag = match key.as_str() {
        "time_spent" => "the time",
        "started" => "--started",
        "adjust_estimate" => "--adjust",
        "new_estimate" => "--new-estimate",
        "reduce_by" => "--reduce-by",
        k => k,
    };
    Error::input(format!("{msg} ({flag})"))
}

fn stop(s: &WorklogSession, io: &mut Io<'_>, e: Error) -> Error {
    let message = e.to_string();
    match tpl::write_draft(&io.drafts, &s.label(), s.draft_text()) {
        Ok(path) => Error::input_hint(message, tpl::draft_hint(&path, &s.command())),
        Err(d) => Error::input_hint(message, format!("jui could not write a Draft: {d}")),
    }
}

/// Run the Template loop until jui can send a request, or it stops.
fn run_loop(s: &mut WorklogSession, io: &mut Io<'_>, first: Option<String>) -> Result<Option<WorklogPlan>> {
    let mut next = first;
    loop {
        let step = match next.take() {
            Some(t) => s.submit(&t),
            None => {
                let Some(ed) = io.editor.as_deref_mut() else { unreachable!("the caller gives a first text in a script") };
                match ed(s.text(), &s.label()) {
                    Ok(t) => s.submit(&t),
                    Err(e) => return Err(stop(s, io, e)),
                }
            }
        };
        match step {
            WorklogStep::Send(p) => return Ok(Some(p)),
            WorklogStep::Cancel(m) => {
                let _ = writeln!(io.out, "{m}");
                return Ok(None);
            }
            WorklogStep::Reopen if io.editor.is_some() => continue,
            WorklogStep::Reopen => {
                let msg = format!("The Worklog is not valid:\n{}", tpl::errors_text(s.errors()));
                return Err(stop(s, io, Error::input(msg)));
            }
            WorklogStep::Stop(m) => return Err(stop(s, io, Error::input(m))),
        }
    }
}

fn warn(p: &WorklogPlan, io: &mut Io<'_>) {
    for w in &p.warnings {
        let _ = writeln!(io.out, "warning: {w}");
    }
}

/// `jui worklog add`.
pub async fn add_worklog(j: &Jira, key: &str, time: Option<&str>, f: &WorklogFlags, clock: Clock, io: &mut Io<'_>) -> Result<WorklogOutcome> {
    let q = quick(time, f, io.stdin.as_deref())?;
    // A time with no `-e` and no `--from` goes at once, with no Template.
    if time.is_some() && !f.editor && f.from.is_none() {
        let p = wl::quick_plan(&q, &clock).map_err(flag_error)?;
        warn(&p, io);
        let w = j.add_worklog(key, &p).await?;
        return Ok(WorklogOutcome::Added(w, p));
    }
    let first = match &f.from {
        Some(path) => Some(read_file(path)?),
        None if io.editor.is_some() => None,
        None => {
            return Err(Error::input_hint(
                "jui needs a terminal to open the Worklog Template.",
                format!("Give the time: `jui worklog add {key} 1h30m`, or use --from <file>."),
            ));
        }
    };
    let ctx = j.worklog_context(key).await?;
    let mut s = WorklogSession::add(&ctx, &q, clock);
    let mut first = first;
    loop {
        let Some(p) = run_loop(&mut s, io, first.take())? else {
            return Ok(WorklogOutcome::Cancelled("Cancelled. No Worklog added.".into()));
        };
        warn(&p, io);
        match j.add_worklog(key, &p).await {
            Ok(w) => return Ok(WorklogOutcome::Added(w, p)),
            Err(e) => match s.rejected(e) {
                Ok(_) if io.editor.is_some() => continue,
                Ok(_) => return Err(stop(&s, io, Error::input(format!("Jira did not accept the Worklog:\n{}", tpl::errors_text(s.errors()))))),
                Err(e) => return Err(stop(&s, io, e)),
            },
        }
    }
}

/// Show the diff of a comment change, and ask. False when the user says no.
fn confirm(p: &WorklogPlan, yes: bool, io: &mut Io<'_>) -> Result<bool> {
    if !p.needs_diff() || yes {
        return Ok(true);
    }
    let _ = writeln!(io.out, "The Worklog comment changes:");
    for l in tpl::diff(p.old_comment.as_deref().unwrap_or(""), &p.comment) {
        let line = format!("{}{}", l.tag, l.text);
        let line = match (io.color, l.tag) {
            (true, '-') => format!("\x1b[31m{line}\x1b[0m"),
            (true, '+') => format!("\x1b[32m{line}\x1b[0m"),
            _ => line,
        };
        let _ = writeln!(io.out, "{line}");
    }
    let Some(input) = io.input.as_deref_mut() else {
        return Err(Error::input_hint("jui must ask before it changes the Worklog comment.", "Use --yes to send the change."));
    };
    let answer = prompt::ask(input, io.out, "Send this change? [y/N] ")?;
    Ok(matches!(answer.to_lowercase().as_str(), "y" | "yes"))
}

/// The arguments of `jui worklog edit`.
pub struct EditCmd<'a> {
    pub key: &'a str,
    pub id: &'a str,
    pub time: Option<&'a str>,
    pub flags: &'a WorklogFlags,
    pub yes: bool,
}

/// `jui worklog edit`.
pub async fn edit_worklog(j: &Jira, c: EditCmd<'_>, clock: Clock, io: &mut Io<'_>) -> Result<WorklogOutcome> {
    let EditCmd { key, id, time, flags: f, yes } = c;
    let q = quick(time, f, io.stdin.as_deref())?;
    let has_flags = time.is_some() || f.started.is_some() || f.comment.is_some();
    // Flags with no `-e` and no `--from`: change only the given fields, with no Template.
    if has_flags && !f.editor && f.from.is_none() {
        let before = j.worklog(key, id).await?;
        let Some(p) = wl::edit_plan(&before, &q, &clock).map_err(flag_error)? else {
            let _ = writeln!(io.out, "No changes.");
            return Ok(WorklogOutcome::Cancelled("No changes.".into()));
        };
        warn(&p, io);
        if !confirm(&p, yes, io)? {
            let _ = writeln!(io.out, "Cancelled. Worklog {id} did not change.");
            return Ok(WorklogOutcome::Cancelled("Cancelled.".into()));
        }
        let w = j.edit_worklog(key, id, &p).await?;
        return Ok(WorklogOutcome::Edited(w, p));
    }
    let ctx = j.worklog_context(key).await?;
    let mut s = WorklogSession::edit(&ctx, id, clock)?;
    let first = match &f.from {
        Some(path) => Some(read_file(path)?),
        None if io.editor.is_some() => None,
        None => {
            return Err(Error::input_hint(
                "jui needs a terminal to open the Worklog Template.",
                format!("Give the changes: `jui worklog edit {key} {id} 2h`, -s, -m, or --from <file>."),
            ));
        }
    };
    let mut first = first;
    loop {
        let Some(p) = run_loop(&mut s, io, first.take())? else {
            return Ok(WorklogOutcome::Cancelled("No changes.".into()));
        };
        warn(&p, io);
        let ok = match confirm(&p, yes, io) {
            Ok(ok) => ok,
            Err(e) => {
                let hint = e.hint().unwrap_or_default();
                let e = stop(&s, io, e);
                return Err(Error::input_hint(e.to_string(), format!("{hint} {}", e.hint().unwrap_or_default()).trim().to_string()));
            }
        };
        if !ok {
            if io.editor.is_some() {
                s.reopen_note("You did not send the change. Edit the Template, or save it with no changes to cancel.");
                continue;
            }
            return Err(stop(&s, io, Error::Cancelled));
        }
        // The Worklog can have changed in Jira while the editor was open.
        let now = j.worklog(key, id).await?;
        if let Some(msg) = s.old().and_then(|before| wl::changed_in_jira(key, before, &now)) {
            let _ = writeln!(io.out, "{msg}");
            let Some(input) = io.input.as_deref_mut() else { return Err(stop(&s, io, Error::input(msg))) };
            let answer = prompt::ask(input, io.out, "Send your change? [y/N] ")?;
            if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
                return Err(stop(&s, io, Error::Cancelled));
            }
        }
        match j.edit_worklog(key, id, &p).await {
            Ok(w) => return Ok(WorklogOutcome::Edited(w, p)),
            Err(e) => match s.rejected(e) {
                Ok(_) if io.editor.is_some() => continue,
                Ok(_) => return Err(stop(&s, io, Error::input(format!("Jira did not accept the change:\n{}", tpl::errors_text(s.errors()))))),
                Err(e) => return Err(stop(&s, io, e)),
            },
        }
    }
}

/// `jui worklog delete`. Returns false when the user says no.
pub async fn delete_worklog(j: &Jira, key: &str, id: &str, adjust: &wl::Adjust, yes: bool, io: &mut Io<'_>) -> Result<bool> {
    if !yes {
        let Some(input) = io.input.as_deref_mut() else {
            return Err(Error::input_hint(format!("jui must ask before it deletes Worklog {id} on {key}."), "Use --yes to delete it. A delete cannot be reversed."));
        };
        let w = j.worklog(key, id).await?;
        let _ = writeln!(io.out, "{}", wl::one_line(&w, 70));
        let answer = prompt::ask(input, io.out, &format!("Delete Worklog {id} on {key}? [y/N] "))?;
        if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
            return Ok(false);
        }
    }
    j.delete_worklog(key, id, adjust).await?;
    Ok(true)
}

/// The estimate option of `worklog delete`.
pub fn delete_adjust(adjust: Option<AdjustArg>, new_estimate: Option<&str>, increase_by: Option<&str>) -> Result<wl::Adjust> {
    wl::Adjust::parse(adjust.map(AdjustArg::name).unwrap_or("auto"), new_estimate, increase_by, "increase_by").map_err(|(k, m)| {
        let flag = match k.as_str() {
            "new_estimate" => "--new-estimate",
            "increase_by" => "--increase-by",
            _ => "--adjust",
        };
        Error::input(format!("{} ({flag})", m.replace("`increase_by`", "--increase-by").replace("`new_estimate`", "--new-estimate")))
    })
}
