//! Worklogs: the time text, the start time, the estimate options, the Worklog Template, and
//! the add and edit logic. See ticket 08 "Template format for create and edit", part 5.
//!
//! A Worklog Template has a TOML header (`time_spent`, `started`, `adjust_estimate`,
//! `new_estimate`, `reduce_by`) and a body: the Worklog comment in Markdown. The reference
//! part below the scissors line shows the Ticket, its time tracking, and the last Worklogs.
//!
//! Server facts (GET check, 2026-10-20): time tracking is on, with 8 hours in a day, 5 days
//! in a week, and the default unit "minute". A Worklog with no comment has no `comment` key.
//!
//! All functions are pure. The time comes from a `Clock`, so that tests are stable.

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde_json::{Value, json};

use super::error::Error;
use super::format::human_duration;
use super::markup;
use super::model::{Worklog, WorklogContext};
use super::template::{self as tpl, SCISSORS, TemplateError, content, norm};

/// Seconds in one Jira day (server setting: 8 hours).
pub const DAY_SECS: u64 = 8 * 3600;
/// Seconds in one Jira week (server setting: 5 days).
pub const WEEK_SECS: u64 = 5 * DAY_SECS;
/// The number of earlier Worklogs that the reference part shows.
pub const CONTEXT_WORKLOGS: usize = 3;
/// A date with no time starts at this time.
pub const DEFAULT_START: (u32, u32) = (9, 0);
/// The format of `started` in the Template.
pub const STARTED_FORMAT: &str = "%Y-%m-%d %H:%M";
/// The format of `started` that Jira needs.
pub const JIRA_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3f%z";

// ------------------------------------------------------------------ time

/// Change a local time to a time with an offset. `None` when the local time does not exist.
pub type LocalFn = fn(NaiveDateTime) -> Option<DateTime<FixedOffset>>;

/// The time zone of the system (the user's time zone).
pub fn system_local(n: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
    Local.from_local_datetime(&n).earliest().map(|d| d.fixed_offset())
}

/// The current local time and the time zone. Tests give a fixed clock.
#[derive(Clone, Copy)]
pub struct Clock {
    pub now: NaiveDateTime,
    pub local: LocalFn,
}

impl Clock {
    pub fn system() -> Self {
        Clock { now: Local::now().naive_local(), local: system_local }
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clock").field("now", &self.now).finish_non_exhaustive()
    }
}

impl PartialEq for Clock {
    fn eq(&self, other: &Self) -> bool {
        self.now == other.now
    }
}

impl Eq for Clock {}

/// Parse a time text to seconds: `1h 30m`, `1h30m`, `1.5h`, `90m`, `2d`, `1w`, or a number of
/// minutes (`45`). A day is 8 hours and a week is 5 days (the server settings).
pub fn parse_duration(text: &str) -> Result<u64, String> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Err("The time is empty. Use for example `1h 30m`.".into());
    }
    let bad = || format!("`{}` is not a time. Use for example `1h 30m`, `90m`, `1.5h`, or `2d`.", text.trim());
    if let Ok(n) = t.parse::<f64>() {
        return to_secs(n * 60.0).ok_or_else(bad);
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut any = false;
    for c in t.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            ' ' | ',' => {
                if !num.is_empty() {
                    return Err(bad());
                }
            }
            'w' | 'd' | 'h' | 'm' => {
                let n: f64 = num.parse().map_err(|_| bad())?;
                num.clear();
                let unit = match c {
                    'w' => WEEK_SECS,
                    'd' => DAY_SECS,
                    'h' => 3600,
                    _ => 60,
                } as f64;
                total += n * unit;
                any = true;
            }
            _ => return Err(bad()),
        }
    }
    if !num.is_empty() || !any {
        return Err(bad());
    }
    to_secs(total).ok_or_else(bad)
}

/// Round to whole minutes. `None` for zero or less.
fn to_secs(s: f64) -> Option<u64> {
    let m = (s / 60.0).round();
    (m >= 1.0 && m.is_finite()).then_some(m as u64 * 60)
}

/// Parse a start time. Empty, `now`, and `today` are now. Also: `yesterday`, `HH:MM`
/// (today), `today HH:MM`, `yesterday HH:MM`, `YYYY-MM-DD` (at 09:00), and
/// `YYYY-MM-DD HH:MM`. The time is local time.
pub fn parse_started(text: &str, clock: &Clock) -> Result<DateTime<FixedOffset>, String> {
    let t = text.trim().to_lowercase();
    let now = clock.now.with_second(0);
    let bad = || format!("`{}` is not a start time. Use `now`, `yesterday`, `10:00`, or `2026-10-19 10:00`.", text.trim());
    let time = |s: &str| NaiveTime::parse_from_str(s.trim(), "%H:%M").map_err(|_| bad());
    let naive = match t.as_str() {
        "" | "now" | "today" => now,
        "yesterday" => now - Duration::days(1),
        _ => {
            if let Some(rest) = t.strip_prefix("today ") {
                now.date().and_time(time(rest)?)
            } else if let Some(rest) = t.strip_prefix("yesterday ") {
                (now.date() - Duration::days(1)).and_time(time(rest)?)
            } else if let Ok(tm) = time(&t) {
                now.date().and_time(tm)
            } else if let Ok(d) = NaiveDate::parse_from_str(&t, "%Y-%m-%d") {
                d.and_hms_opt(DEFAULT_START.0, DEFAULT_START.1, 0).ok_or_else(bad)?
            } else {
                let t2 = t.replace('t', " ");
                NaiveDateTime::parse_from_str(&t2, STARTED_FORMAT).map_err(|_| bad())?
            }
        }
    };
    (clock.local)(naive).ok_or_else(|| format!("{} does not exist in your time zone.", naive.format(STARTED_FORMAT)))
}

trait WithSecond {
    fn with_second(self, s: u32) -> Self;
}

impl WithSecond for NaiveDateTime {
    fn with_second(self, s: u32) -> Self {
        chrono::Timelike::with_second(&self, s).and_then(|d| chrono::Timelike::with_nanosecond(&d, 0)).unwrap_or(self)
    }
}

/// A start time in the Jira format, for example `2026-10-19T10:00:00.000-0700`.
pub fn jira_started(d: &DateTime<FixedOffset>) -> String {
    d.format(JIRA_FORMAT).to_string()
}

// ------------------------------------------------------------------ estimate

/// What Jira does with the remaining estimate (`adjustEstimate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adjust {
    /// Jira reduces the remaining estimate by the time spent (or adds it back for a delete).
    Auto,
    /// The remaining estimate does not change.
    Leave,
    /// Set the remaining estimate to this time (Jira text, for example `2h`).
    New(String),
    /// Add: reduce the remaining estimate by this time. Delete: increase it by this time.
    Manual(String),
}

impl Adjust {
    /// The query of an add or an edit.
    pub fn query(&self) -> Vec<(&'static str, String)> {
        match self {
            Adjust::Auto => vec![("adjustEstimate", "auto".into())],
            Adjust::Leave => vec![("adjustEstimate", "leave".into())],
            Adjust::New(e) => vec![("adjustEstimate", "new".into()), ("newEstimate", e.clone())],
            Adjust::Manual(r) => vec![("adjustEstimate", "manual".into()), ("reduceBy", r.clone())],
        }
    }

    /// The query of a delete: `manual` uses `increaseBy`.
    pub fn delete_query(&self) -> Vec<(&'static str, String)> {
        match self {
            Adjust::Manual(r) => vec![("adjustEstimate", "manual".into()), ("increaseBy", r.clone())],
            a => a.query(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Adjust::Auto => "auto",
            Adjust::Leave => "leave",
            Adjust::New(_) => "new",
            Adjust::Manual(_) => "manual",
        }
    }

    /// Make the option from its name and the times. `extra` is `new_estimate` for `new`,
    /// and `reduce_by` (or `increase_by`) for `manual`. The error is (the key, the message).
    pub fn parse(name: &str, new_estimate: Option<&str>, manual: Option<&str>, manual_key: &str) -> Result<Adjust, (String, String)> {
        let time = |key: &str, v: Option<&str>| -> Result<String, (String, String)> {
            let v = v.map(str::trim).filter(|v| !v.is_empty()).ok_or_else(|| (key.to_string(), format!("Give `{key}` with adjust_estimate = \"{name}\".")))?;
            if v == "0" || v == "0m" {
                return Ok("0m".into());
            }
            parse_duration(v).map(human_duration).map_err(|e| (key.to_string(), e))
        };
        match name.trim().to_lowercase().as_str() {
            "" | "auto" => Ok(Adjust::Auto),
            "leave" => Ok(Adjust::Leave),
            "new" => Ok(Adjust::New(time("new_estimate", new_estimate)?)),
            "manual" => Ok(Adjust::Manual(time(manual_key, manual)?)),
            other => Err(("adjust_estimate".into(), format!("`{other}` is not permitted. Use auto, leave, new, or manual."))),
        }
    }
}

// ------------------------------------------------------------------ plan

/// The request that jui sends for an add or an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorklogPlan {
    /// `None` for a new Worklog.
    pub id: Option<String>,
    pub seconds: u64,
    pub started: DateTime<FixedOffset>,
    /// The comment in wiki markup (empty: no comment).
    pub comment: String,
    /// Edit: the old comment in wiki markup, for the diff.
    pub old_comment: Option<String>,
    pub adjust: Adjust,
    /// Losses that the Markdown-to-wiki conversion reports.
    pub warnings: Vec<String>,
    /// The JSON body.
    pub body: Value,
}

impl WorklogPlan {
    fn new(id: Option<String>, seconds: u64, started: DateTime<FixedOffset>, comment: String, old_comment: Option<String>, adjust: Adjust, warnings: Vec<String>) -> Self {
        let mut body = json!({ "timeSpentSeconds": seconds, "started": jira_started(&started) });
        // An edit sends an empty comment to remove the old one.
        if !comment.is_empty() || id.is_some() {
            body["comment"] = json!(comment);
        }
        WorklogPlan { id, seconds, started, comment, old_comment, adjust, warnings, body }
    }

    /// True when jui must show a diff: the edit changes an existing comment.
    pub fn needs_diff(&self) -> bool {
        self.old_comment.as_deref().is_some_and(|o| !o.trim().is_empty() && norm(o) != norm(&self.comment))
    }

    /// "1h 30m on 2026-10-19 10:00".
    pub fn describe(&self) -> String {
        format!("{} on {}", human_duration(self.seconds), self.started.format(STARTED_FORMAT))
    }
}

/// The comment text as wiki markup, and the warnings. With `wiki`, no conversion.
fn comment_wiki(text: &str, wiki: bool) -> (String, Vec<String>) {
    let t = norm(text);
    if t.is_empty() || wiki {
        return (t, vec![]);
    }
    let c = markup::markdown_to_wiki(&t);
    (c.text, c.warnings)
}

/// The fields of the `L` popup or of the command line. The error is (the field, the message):
/// the field is `time_spent`, `started`, or a key of the estimate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Quick {
    pub time_spent: String,
    pub started: String,
    /// Markdown.
    pub comment: String,
    pub adjust: Option<String>,
    pub new_estimate: Option<String>,
    pub reduce_by: Option<String>,
}

/// A new Worklog from the popup or from the command line, with no Template.
pub fn quick_plan(q: &Quick, clock: &Clock) -> Result<WorklogPlan, (String, String)> {
    let seconds = parse_duration(&q.time_spent).map_err(|e| ("time_spent".to_string(), e))?;
    let started = parse_started(&q.started, clock).map_err(|e| ("started".to_string(), e))?;
    let adjust = Adjust::parse(q.adjust.as_deref().unwrap_or("auto"), q.new_estimate.as_deref(), q.reduce_by.as_deref(), "reduce_by")?;
    let (comment, warnings) = comment_wiki(&q.comment, false);
    Ok(WorklogPlan::new(None, seconds, started, comment, None, adjust, warnings))
}

/// A changed Worklog from the command-line flags, with no Template. Fields with no flag
/// keep their values. `None` when nothing changes.
pub fn edit_plan(w: &Worklog, q: &Quick, clock: &Clock) -> Result<Option<WorklogPlan>, (String, String)> {
    let seconds = if q.time_spent.trim().is_empty() { w.time_spent_seconds } else { parse_duration(&q.time_spent).map_err(|e| ("time_spent".to_string(), e))? };
    let started = match (q.started.trim().is_empty(), w.started) {
        (true, Some(s)) => s,
        _ => parse_started(&q.started, clock).map_err(|e| ("started".to_string(), e))?,
    };
    let old = w.comment.clone().unwrap_or_default();
    let (comment, warnings) = if q.comment.is_empty() { (old.clone(), vec![]) } else { comment_wiki(&q.comment, false) };
    let adjust = edit_adjust(q)?;
    let same = seconds == w.time_spent_seconds && Some(started) == w.started && norm(&comment) == norm(&old);
    if same {
        return Ok(None);
    }
    Ok(Some(WorklogPlan::new(Some(w.id.clone()), seconds, started, comment, Some(old), adjust, warnings)))
}

/// The estimate option of an edit. Jira accepts only auto, leave, and new for an edit.
fn edit_adjust(q: &Quick) -> Result<Adjust, (String, String)> {
    let a = Adjust::parse(q.adjust.as_deref().unwrap_or("auto"), q.new_estimate.as_deref(), q.reduce_by.as_deref(), "reduce_by")?;
    if matches!(a, Adjust::Manual(_)) {
        return Err(("adjust_estimate".into(), "An edit permits only auto, leave, or new.".into()));
    }
    Ok(a)
}

// ------------------------------------------------------------------ session

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorklogMode {
    Add,
    /// Edit Worklog `id`.
    Edit { id: String },
}

/// What the caller must do next.
#[derive(Debug, Clone, PartialEq)]
pub enum WorklogStep {
    /// Stop with no request. The text is the message.
    Cancel(String),
    /// Open the editor again with `WorklogSession::text()`. It has the error lines.
    Reopen,
    /// Stop and write a Draft. The text is the reason.
    Stop(String),
    Send(WorklogPlan),
}

/// The state of one Worklog add or edit in the editor.
#[derive(Debug, Clone, PartialEq)]
pub struct WorklogSession {
    pub key: String,
    pub mode: WorklogMode,
    clock: Clock,
    shown: String,
    errors: Vec<TemplateError>,
    saved: Option<String>,
    /// Edit: the Worklog when the editor opened.
    old: Option<Worklog>,
}

impl WorklogSession {
    /// Start a new Worklog. `q` has the first values (from the popup or the flags).
    pub fn add(ctx: &WorklogContext, q: &Quick, clock: Clock) -> Self {
        let started = if q.started.trim().is_empty() { clock.now.format(STARTED_FORMAT).to_string() } else { q.started.trim().to_string() };
        let header = Header {
            time_spent: q.time_spent.trim().to_string(),
            started,
            adjust: q.adjust.clone().unwrap_or_else(|| "auto".into()),
            new_estimate: q.new_estimate.clone(),
            reduce_by: q.reduce_by.clone(),
        };
        let shown = template(ctx, None, &header, &q.comment);
        WorklogSession { key: ctx.key.clone(), mode: WorklogMode::Add, clock, shown, errors: vec![], saved: None, old: None }
    }

    /// Start the edit of Worklog `id`.
    pub fn edit(ctx: &WorklogContext, id: &str, clock: Clock) -> Result<Self, Error> {
        let w = ctx.find(id).ok_or_else(|| Error::NotFound(format!("Worklog {id} on {}", ctx.key)))?;
        let header = Header {
            time_spent: human_duration(w.time_spent_seconds),
            started: w.started.map(|d| d.format(STARTED_FORMAT).to_string()).unwrap_or_default(),
            adjust: "auto".into(),
            new_estimate: None,
            reduce_by: None,
        };
        let body = w.comment.as_deref().map(|c| norm(&markup::wiki_to_markdown(c).text)).unwrap_or_default();
        let shown = template(ctx, Some(w), &header, &body);
        Ok(WorklogSession { key: ctx.key.clone(), mode: WorklogMode::Edit { id: id.into() }, clock, shown, errors: vec![], saved: None, old: Some(w.clone()) })
    }

    pub fn text(&self) -> &str {
        &self.shown
    }

    pub fn errors(&self) -> &[TemplateError] {
        &self.errors
    }

    pub fn draft_text(&self) -> &str {
        self.saved.as_deref().unwrap_or(&self.shown)
    }

    pub fn label(&self) -> String {
        match &self.mode {
            WorklogMode::Add => format!("{}-worklog", self.key),
            WorklogMode::Edit { id } => format!("{}-worklog-{id}", self.key),
        }
    }

    /// The `jui` command that uses a Draft again (without `--from`).
    pub fn command(&self) -> String {
        match &self.mode {
            WorklogMode::Add => format!("worklog add {}", self.key),
            WorklogMode::Edit { id } => format!("worklog edit {} {id}", self.key),
        }
    }

    pub fn id(&self) -> Option<&str> {
        match &self.mode {
            WorklogMode::Add => None,
            WorklogMode::Edit { id } => Some(id),
        }
    }

    /// The Worklog when the editor opened (edit only).
    pub fn old(&self) -> Option<&Worklog> {
        self.old.as_ref()
    }

    /// Check the text that the user saved, or a text from `--from`.
    pub fn submit(&mut self, saved: &str) -> WorklogStep {
        if !self.errors.is_empty() && content(saved) == content(&self.shown) {
            self.saved = Some(saved.to_string());
            return WorklogStep::Stop("The Template is not valid, and you did not change it.".into());
        }
        if content(saved).is_empty() {
            return WorklogStep::Cancel(self.cancel_text());
        }
        let parsed = match tpl::parse(saved) {
            Ok(p) => p,
            Err(e) => return self.reopen(saved, vec![e]),
        };
        self.saved = Some(saved.to_string());
        let h = &parsed.header;
        let mut errs = Vec::new();
        for k in h.keys() {
            if !["time_spent", "started", "adjust_estimate", "new_estimate", "reduce_by"].contains(&k.as_str()) {
                errs.push(TemplateError { key: Some(k.clone()), message: "Unknown key. Use time_spent, started, adjust_estimate, new_estimate, or reduce_by.".into() });
            }
        }
        let get = |k: &str| -> Result<Option<String>, TemplateError> {
            match h.get(k) {
                None => Ok(None),
                Some(toml::Value::String(s)) => Ok(Some(s.clone())),
                Some(toml::Value::Integer(n)) => Ok(Some(n.to_string())),
                Some(toml::Value::Float(n)) => Ok(Some(n.to_string())),
                Some(_) => Err(TemplateError { key: Some(k.into()), message: "Use a text in quotes.".into() }),
            }
        };
        let mut val = |k: &str| match get(k) {
            Ok(v) => v,
            Err(e) => {
                errs.push(e);
                None
            }
        };
        let time = val("time_spent").unwrap_or_default();
        let started = val("started").unwrap_or_default();
        let adjust = val("adjust_estimate");
        let new_estimate = val("new_estimate");
        let reduce_by = val("reduce_by");
        // A new Worklog with no time is a cancel (the Template that jui wrote has no time).
        if self.mode == WorklogMode::Add && time.trim().is_empty() && errs.is_empty() {
            return WorklogStep::Cancel(self.cancel_text());
        }
        let seconds = parse_duration(&time).map_err(|m| errs.push(TemplateError { key: Some("time_spent".into()), message: m })).ok();
        let started = if started.trim().is_empty() {
            errs.push(TemplateError { key: Some("started".into()), message: "Give the start time, for example `now` or `2026-10-19 10:00`.".into() });
            None
        } else {
            parse_started(&started, &self.clock).map_err(|m| errs.push(TemplateError { key: Some("started".into()), message: m })).ok()
        };
        let adjust = match Adjust::parse(adjust.as_deref().unwrap_or("auto"), new_estimate.as_deref(), reduce_by.as_deref(), "reduce_by") {
            Ok(Adjust::Manual(_)) if self.old.is_some() => {
                errs.push(TemplateError { key: Some("adjust_estimate".into()), message: "An edit permits only auto, leave, or new.".into() });
                None
            }
            Ok(a) => Some(a),
            Err((k, m)) => {
                errs.push(TemplateError { key: Some(k), message: m });
                None
            }
        };
        if !errs.is_empty() {
            return self.reopen(saved, errs);
        }
        let (seconds, started, adjust) = (seconds.expect("checked"), started.expect("checked"), adjust.expect("checked"));
        let (comment, warnings) = comment_wiki(&parsed.body, false);
        self.errors.clear();
        match &self.old {
            None => WorklogStep::Send(WorklogPlan::new(None, seconds, started, comment, None, adjust, warnings)),
            Some(w) => {
                let old = w.comment.clone().unwrap_or_default();
                let old_md = norm(&markup::wiki_to_markdown(&old).text);
                // An unchanged body keeps the old wiki text (no round trip).
                let (comment, warnings) = if norm(&parsed.body) == old_md { (old.clone(), vec![]) } else { (comment, warnings) };
                let same = seconds == w.time_spent_seconds && Some(started) == w.started && norm(&comment) == norm(&old);
                if same {
                    return WorklogStep::Cancel("No changes.".into());
                }
                WorklogStep::Send(WorklogPlan::new(Some(w.id.clone()), seconds, started, comment, Some(old), adjust, warnings))
            }
        }
    }

    fn cancel_text(&self) -> String {
        match &self.mode {
            WorklogMode::Add => "Cancelled. No Worklog added.".into(),
            WorklogMode::Edit { id } => format!("Cancelled. Worklog {id} did not change."),
        }
    }

    fn reopen(&mut self, saved: &str, errs: Vec<TemplateError>) -> WorklogStep {
        self.saved = Some(saved.to_string());
        self.shown = tpl::with_errors(saved, &errs);
        self.errors = errs;
        WorklogStep::Reopen
    }

    /// Open the editor again with a note at the top (after `n` in the diff question).
    pub fn reopen_note(&mut self, note: &str) {
        let saved = self.saved.clone().unwrap_or_else(|| self.shown.clone());
        self.shown = tpl::with_errors(&saved, &[TemplateError { key: None, message: note.into() }]);
        self.errors.clear();
    }

    /// Jira did not accept the request. A validation error opens the Template again.
    pub fn rejected(&mut self, e: Error) -> Result<WorklogStep, Error> {
        let Error::Validation { fields, messages } = e else { return Err(e) };
        let mut errs: Vec<TemplateError> = messages.into_iter().map(|m| TemplateError { key: None, message: m }).collect();
        errs.extend(fields.into_iter().map(|(k, m)| TemplateError { key: Some(api_key(&k).to_string()), message: m }));
        if errs.is_empty() {
            errs.push(TemplateError { key: None, message: "Jira did not accept the Worklog.".into() });
        }
        let saved = self.saved.clone().unwrap_or_else(|| self.shown.clone());
        Ok(self.reopen(&saved, errs))
    }
}

/// The Template key of a Jira field name in an error.
fn api_key(k: &str) -> &str {
    match k {
        "timeLogged" | "timeSpent" | "timeSpentSeconds" => "time_spent",
        "started" | "startDate" => "started",
        "newEstimate" => "new_estimate",
        "reduceBy" => "reduce_by",
        k => k,
    }
}

/// The message when a Worklog changed in Jira during the edit. `None` when it did not change.
pub fn changed_in_jira(key: &str, before: &Worklog, now: &Worklog) -> Option<String> {
    let changed = before.updated != now.updated
        || before.time_spent_seconds != now.time_spent_seconds
        || before.started != now.started
        || norm(before.comment.as_deref().unwrap_or("")) != norm(now.comment.as_deref().unwrap_or(""));
    changed.then(|| format!("Worklog {} on {key} changed in Jira while you edited it.", before.id))
}

/// One line for a Worklog: the date, the time, the author, and the first line of the comment.
pub fn one_line(w: &Worklog, max: usize) -> String {
    let day = w.started.map(|d| d.format(STARTED_FORMAT).to_string()).unwrap_or_default();
    let who = w.author.as_ref().map(|u| u.name.as_str()).unwrap_or("?");
    let c = first_line(w, max);
    format!("{day}  {}  @{who}{}", human_duration(w.time_spent_seconds), if c.is_empty() { String::new() } else { format!("  {c}") })
}

/// The first line of the comment in display mode, shorter than `max` characters.
pub fn first_line(w: &Worklog, max: usize) -> String {
    let Some(c) = &w.comment else { return String::new() };
    let line = markup::display(c).iter().map(|l| l.plain()).find(|l| !l.trim().is_empty()).unwrap_or_default();
    let line = line.trim();
    if line.chars().count() > max { format!("{}…", line.chars().take(max.saturating_sub(1)).collect::<String>()) } else { line.to_string() }
}

struct Header {
    time_spent: String,
    started: String,
    adjust: String,
    new_estimate: Option<String>,
    reduce_by: Option<String>,
}

fn q(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// The Template text.
fn template(ctx: &WorklogContext, editing: Option<&Worklog>, h: &Header, body: &str) -> String {
    let mut out: Vec<String> = vec!["+++".into()];
    out.push(format!("time_spent = {}        # for example \"1h 30m\", \"90m\", \"1.5h\", \"2d\" (a day is 8h)", q(&h.time_spent)));
    out.push(format!("started = {}   # local time; also \"now\", \"yesterday\", \"10:00\"", q(&h.started)));
    out.push(format!("adjust_estimate = {}   # remaining estimate: auto, leave, new, or manual", q(&h.adjust)));
    match &h.new_estimate {
        Some(v) => out.push(format!("new_estimate = {}", q(v))),
        None => out.push("# new_estimate = \"2h\"     # with adjust_estimate = \"new\"".into()),
    }
    if editing.is_none() {
        match &h.reduce_by {
            Some(v) => out.push(format!("reduce_by = {}", q(v))),
            None => out.push("# reduce_by = \"1h\"        # with adjust_estimate = \"manual\"".into()),
        }
    }
    out.push("+++".into());
    out.push(body.to_string());
    out.push(String::new());
    out.push(SCISSORS.into());
    out.push("# jui ignores this line and all lines below it.".into());
    match editing {
        None => {
            out.push(format!("# Add a Worklog to {}: {}", ctx.key, ctx.summary));
            out.push("# Write the comment above the scissors line, in Markdown. It is optional.".into());
            out.push("# Save with an empty time_spent to cancel.".into());
        }
        Some(w) => {
            let who = w.author.as_ref().map(|u| u.name.as_str()).unwrap_or("?");
            out.push(format!("# Edit Worklog {} by @{who} on {}: {}", w.id, ctx.key, ctx.summary));
            out.push("# The comment is Markdown. Save with no changes to cancel.".into());
        }
    }
    let tt = [("estimate", &ctx.original_estimate), ("remaining", &ctx.remaining_estimate), ("spent", &ctx.time_spent)];
    let parts: Vec<String> = tt.iter().filter_map(|(n, v)| v.as_ref().map(|v| format!("{n} {v}"))).collect();
    out.push(format!("# Time tracking: {}", if parts.is_empty() { "no estimate, no time spent".to_string() } else { parts.join(", ") }));
    let others: Vec<&Worklog> = ctx.worklogs.iter().filter(|w| editing.is_none_or(|e| e.id != w.id)).collect();
    let start = others.len().saturating_sub(CONTEXT_WORKLOGS);
    if !others.is_empty() {
        out.push("#".into());
        out.push(format!("# The last Worklogs ({} of {}):", others.len() - start, others.len()));
        for w in &others[start..] {
            out.push(format!("#   {}", one_line(w, 60)).trim_end().to_string());
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests;
