//! Templates for create and edit. See ticket 08 "Template format for create and edit".
//!
//! A Template is a Markdown file:
//! 1. A TOML header between two `+++` lines (the fields).
//! 2. A Markdown body (the description).
//! 3. An optional reference part below the scissors line. jui ignores it.
//!
//! All functions here are pure. A `Session` keeps the state of one create or edit:
//! the text that jui wrote, the errors, and the text for a Draft. The CLI and the TUI
//! drive the same `Session`; they only start the editor and send the requests.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::error::Error;
use super::markup;
use super::model::{Choice, CreateMeta, EditData, FieldKind, ScreenField, looks_like_key, normalize_key};
use super::transition::snake;

/// jui ignores this line and all lines below it.
pub const SCISSORS: &str = "# ------------------------ >8 ------------------------";
/// The body text of a new Template. jui does not send it.
pub const PLACEHOLDER: &str = "Write the description here in Markdown.";
/// Lines that start with this text are error lines. jui removes them before it reads the Template.
pub const ERROR_PREFIX: &str = "# ERROR:";
/// A field with more permitted values than this shows only the first values in the header.
pub const MAX_SHOWN: usize = 10;
/// jui deletes Drafts that are older than this number of days.
pub const DRAFT_DAYS: u64 = 30;

/// Standard fields have a fixed key.
const FIXED: &[(&str, &str)] = &[
    ("project", "project"),
    ("type", "issuetype"),
    ("summary", "summary"),
    ("components", "components"),
    ("labels", "labels"),
    ("priority", "priority"),
    ("assignee", "assignee"),
    ("reporter", "reporter"),
    ("epic", "customfield_10016"),
    ("epic_name", "customfield_10018"),
    ("parent", "parent"),
    ("fix_versions", "fixVersions"),
    ("affects_versions", "versions"),
    ("story_points", "customfield_10021"),
    ("due_date", "duedate"),
];

/// The frequent optional fields. The create Template shows them as comments.
const FREQUENT: &[&str] = &["assignee", "priority", "labels", "customfield_10016", "fixVersions", "customfield_10021", "duedate"];

/// Fields that the header never shows. The body is the description.
const NOT_IN_HEADER: &[&str] = &["project", "issuetype", "description", "attachment", "issuelinks", "comment"];

/// One problem in a Template. `key` is the field key, or `None` for the full Template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    pub key: Option<String>,
    pub message: String,
}

impl TemplateError {
    fn new(key: &str, message: impl Into<String>) -> Self {
        TemplateError { key: Some(key.to_string()), message: message.into() }
    }

    fn general(message: impl Into<String>) -> Self {
        TemplateError { key: None, message: message.into() }
    }

    /// The text of the error line (without the `# ERROR:` prefix).
    pub fn text(&self) -> String {
        match &self.key {
            Some(k) => format!("{k}: {}", self.message),
            None => self.message.clone(),
        }
    }
}

// ------------------------------------------------------------------ keys

/// The Template key of each field, in the same order as `fields`.
/// Standard fields have a fixed key. Other fields use the snake-case name.
/// When two fields get the same key, both use their field ID.
pub fn keys(fields: &[ScreenField]) -> Vec<String> {
    let first: Vec<String> = fields
        .iter()
        .map(|f| match FIXED.iter().find(|(_, id)| *id == f.id) {
            Some((k, _)) => k.to_string(),
            None => {
                let s = snake(&f.name);
                if s.is_empty() { f.id.clone() } else { s }
            }
        })
        .collect();
    first
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let fixed = FIXED.iter().any(|(_, id)| *id == fields[i].id);
            let dup = first.iter().enumerate().any(|(j, o)| j != i && o == k);
            if dup && !fixed { fields[i].id.clone() } else { k.clone() }
        })
        .collect()
}

/// Find the index of a field by its key, its ID, its name, or its snake-case name.
fn find(fields: &[ScreenField], keys: &[String], key: &str) -> Option<usize> {
    let k = key.trim();
    keys.iter()
        .position(|x| x.eq_ignore_ascii_case(k))
        .or_else(|| fields.iter().position(|f| f.id.eq_ignore_ascii_case(k)))
        .or_else(|| fields.iter().position(|f| f.name.eq_ignore_ascii_case(k) || snake(&f.name) == snake(k)))
}

fn is_custom(id: &str) -> bool {
    id.starts_with("customfield_")
}

// ------------------------------------------------------------------ values

/// The result of the conversion of one header value.
#[derive(Debug, Clone, PartialEq)]
enum Conv {
    /// The JSON value for Jira.
    Json(Value),
    /// No value: `""`, `[]`, or `none` for a user.
    Empty,
}

fn is_empty(v: &toml::Value) -> bool {
    match v {
        toml::Value::String(s) => s.trim().is_empty(),
        toml::Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// The text values of a TOML value. A string gives one value.
fn texts(v: &toml::Value) -> Result<Vec<String>, String> {
    match v {
        toml::Value::String(s) => Ok(vec![s.trim().to_string()]),
        toml::Value::Array(a) => a
            .iter()
            .map(|x| match x {
                toml::Value::String(s) => Ok(s.trim().to_string()),
                toml::Value::Integer(n) => Ok(n.to_string()),
                toml::Value::Float(n) => Ok(n.to_string()),
                _ => Err("use a list of text values, for example [\"a\", \"b\"].".to_string()),
            })
            .collect(),
        toml::Value::Integer(n) => Ok(vec![n.to_string()]),
        toml::Value::Float(n) => Ok(vec![n.to_string()]),
        _ => Err("use text in quotes.".to_string()),
    }
}

fn one_text(v: &toml::Value) -> Result<String, String> {
    let t = texts(v)?;
    match t.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err("use one value, not a list.".to_string()),
    }
}

fn permitted_list(cs: &[Choice]) -> String {
    let mut shown: Vec<&str> = cs.iter().take(MAX_SHOWN).map(|c| c.name.as_str()).collect();
    if cs.len() > MAX_SHOWN {
        shown.push("…");
    }
    shown.join(", ")
}

/// Find a permitted value: exact name or ID first, then a unique prefix (ignores case).
fn pick<'a>(f: &'a ScreenField, text: &str) -> Result<&'a Choice, String> {
    let t = text.trim().to_lowercase();
    if let Some(c) = f.allowed.iter().find(|c| c.name.to_lowercase() == t || c.id == t) {
        return Ok(c);
    }
    let found: Vec<&Choice> = f.allowed.iter().filter(|c| c.name.to_lowercase().starts_with(&t)).collect();
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("\"{}\" is not a permitted value. Permitted: {}", text.trim(), permitted_list(&f.allowed))),
        many => Err(format!(
            "\"{}\" matches more than one value: {}",
            text.trim(),
            many.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Convert a header value to the JSON value of a field.
fn to_json(f: &ScreenField, v: &toml::Value, me: &str, project: &str) -> Result<Conv, String> {
    if is_empty(v) {
        return Ok(Conv::Empty);
    }
    match &f.kind {
        FieldKind::Text => match v {
            toml::Value::String(s) => Ok(Conv::Json(json!(s))),
            _ => Err("use text in quotes.".into()),
        },
        FieldKind::Number => match v {
            toml::Value::Integer(n) => Ok(Conv::Json(json!(n))),
            toml::Value::Float(n) => Ok(Conv::Json(json!(n))),
            toml::Value::String(s) => {
                s.trim().parse::<f64>().map(|n| Conv::Json(json!(n))).map_err(|_| "use a number, for example 3.".into())
            }
            _ => Err("use a number, for example 3.".into()),
        },
        FieldKind::Date => {
            let t = one_text(v)?;
            chrono::NaiveDate::parse_from_str(&t, "%Y-%m-%d")
                .map(|_| Conv::Json(json!(t)))
                .map_err(|_| format!("\"{t}\" is not a date. Use \"YYYY-MM-DD\"."))
        }
        FieldKind::User => {
            let t = one_text(v)?;
            match t.to_lowercase().as_str() {
                "none" => Ok(Conv::Empty),
                "me" => Ok(Conv::Json(json!({ "name": me }))),
                _ => Ok(Conv::Json(json!({ "name": t }))),
            }
        }
        FieldKind::Labels => {
            let t = texts(v)?;
            if let Some(bad) = t.iter().find(|l| l.contains(char::is_whitespace)) {
                return Err(format!("the label \"{bad}\" contains a space. Jira labels cannot contain spaces."));
            }
            Ok(Conv::Json(Value::Array(t.into_iter().filter(|s| !s.is_empty()).map(Value::String).collect())))
        }
        FieldKind::Choice => Ok(Conv::Json(json!({ "id": pick(f, &one_text(v)?)?.id }))),
        FieldKind::MultiChoice => {
            let ids: Result<Vec<Value>, String> =
                texts(v)?.iter().filter(|s| !s.is_empty()).map(|s| pick(f, s).map(|c| json!({ "id": c.id }))).collect();
            Ok(Conv::Json(Value::Array(ids?)))
        }
        FieldKind::Key => {
            let raw = one_text(v)?;
            let key = normalize_key(&raw, project);
            if !looks_like_key(&key) {
                return Err(format!("\"{raw}\" is not a Ticket key. Use a key like {project}-123."));
            }
            if f.id == "parent" { Ok(Conv::Json(json!({ "key": key }))) } else { Ok(Conv::Json(json!(key))) }
        }
        FieldKind::Other(kind) => Err(format!("jui cannot set this field type (`{kind}`). Use the web UI: jui open <key>")),
    }
}

/// The empty JSON value that clears a field in an edit.
fn clear_value(f: &ScreenField) -> Value {
    match f.kind {
        FieldKind::Labels | FieldKind::MultiChoice => json!([]),
        _ => Value::Null,
    }
}

/// Convert the current Jira value of a field to a header value. `None` when the field
/// has no value, or jui cannot show its type.
pub fn from_json(f: &ScreenField, v: &Value) -> Option<toml::Value> {
    let name_of = |x: &Value| {
        x.get("name").or_else(|| x.get("value")).and_then(Value::as_str).map(|s| toml::Value::String(s.to_string()))
    };
    let out = match &f.kind {
        _ if v.is_null() => None,
        FieldKind::Text | FieldKind::Date => v.as_str().map(|s| toml::Value::String(s.to_string())),
        FieldKind::Number => v.as_f64().map(|n| {
            if n.fract() == 0.0 && n.abs() < 1e15 { toml::Value::Integer(n as i64) } else { toml::Value::Float(n) }
        }),
        FieldKind::User => v.get("name").and_then(Value::as_str).map(|s| toml::Value::String(s.to_string())),
        FieldKind::Labels => v
            .as_array()
            .map(|a| toml::Value::Array(a.iter().filter_map(Value::as_str).map(|s| toml::Value::String(s.into())).collect())),
        FieldKind::Choice => name_of(v),
        FieldKind::MultiChoice => v.as_array().map(|a| toml::Value::Array(a.iter().filter_map(name_of).collect())),
        FieldKind::Key => v
            .as_str()
            .or_else(|| v.get("key").and_then(Value::as_str))
            .map(|s| toml::Value::String(s.to_string())),
        FieldKind::Other(_) => None,
    };
    out.filter(|t| !is_empty(t))
}

/// The empty header value for a field: `[]` for lists, `""` for other fields.
fn empty_for(f: &ScreenField) -> toml::Value {
    match f.kind {
        FieldKind::Labels | FieldKind::MultiChoice => toml::Value::Array(vec![]),
        _ => toml::Value::String(String::new()),
    }
}

/// Write a TOML value on one line.
pub fn fmt_value(v: &toml::Value) -> String {
    match v {
        toml::Value::Array(a) => format!("[{}]", a.iter().map(fmt_value).collect::<Vec<_>>().join(", ")),
        // `Value::String` writes a TOML string with the correct escapes.
        other => other.to_string(),
    }
}

// ------------------------------------------------------------------ values from flags

/// A value from a command-line flag. jui changes it to a header value with the field type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagValue {
    /// One text. For a list field, jui splits it at commas.
    Text(String),
    /// A repeated flag (for example `-c A -c B`).
    List(Vec<String>),
}

/// Values from flags. They go into the Template, and the same checks apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preset {
    /// The field key (or ID, or name) and the value.
    pub values: Vec<(String, FlagValue)>,
    /// The description in Markdown (or raw wiki markup for a raw wiki edit).
    pub description: Option<String>,
}

impl Preset {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.description.is_none()
    }

    /// Add `FIELD=VALUE` from `--set`.
    pub fn set(&mut self, raw: &str) -> Result<(), Error> {
        let (k, v) = raw.split_once('=').ok_or_else(|| {
            Error::input_hint(format!("`{raw}` is not FIELD=VALUE."), "Example: --set \"severity=Major\"")
        })?;
        self.values.push((k.trim().to_string(), FlagValue::Text(v.trim().to_string())));
        Ok(())
    }
}

fn flag_to_toml(f: Option<&ScreenField>, v: &FlagValue) -> toml::Value {
    let list = |items: Vec<String>| toml::Value::Array(items.into_iter().map(toml::Value::String).collect());
    let kind = f.map(|f| &f.kind);
    match (kind, v) {
        (Some(FieldKind::Labels | FieldKind::MultiChoice), FlagValue::Text(t)) => {
            list(t.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
        }
        (Some(FieldKind::Labels | FieldKind::MultiChoice), FlagValue::List(l)) => list(l.clone()),
        (Some(FieldKind::Number), FlagValue::Text(t)) => match t.parse::<i64>() {
            Ok(n) => toml::Value::Integer(n),
            Err(_) => t.parse::<f64>().map(toml::Value::Float).unwrap_or_else(|_| toml::Value::String(t.clone())),
        },
        (_, FlagValue::Text(t)) => toml::Value::String(t.clone()),
        (_, FlagValue::List(l)) if l.len() == 1 => toml::Value::String(l[0].clone()),
        (_, FlagValue::List(l)) => list(l.clone()),
    }
}

// ------------------------------------------------------------------ parse

/// A Template after the split: the TOML header and the body.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub header: toml::Table,
    pub body: String,
}

/// Normalize text for a compare: LF line ends, no spaces at the ends of the lines,
/// and no blank lines at the start or the end.
pub fn norm(text: &str) -> String {
    let t = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = t.lines().map(str::trim_end).collect();
    let start = lines.iter().position(|l| !l.is_empty()).unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| !l.is_empty()).map(|i| i + 1).unwrap_or(start);
    lines[start..end].join("\n")
}

/// Remove the error lines and the reference part. Returns the lines that remain.
fn content_lines(text: &str) -> Vec<String> {
    let t = text.replace("\r\n", "\n");
    let mut out = Vec::new();
    for l in t.lines() {
        if l.trim_end() == SCISSORS || (l.starts_with("# ---") && l.contains(">8")) {
            break;
        }
        if l.starts_with(ERROR_PREFIX) {
            continue;
        }
        out.push(l.to_string());
    }
    out
}

/// The text with no error lines and no reference part, normalized.
pub fn content(text: &str) -> String {
    norm(&content_lines(text).join("\n"))
}

/// Split a Template into the header and the body, and parse the header.
/// A Template with no header gives an empty header (comments and Worklogs can have no header).
pub fn parse(text: &str) -> Result<Parsed, TemplateError> {
    let lines = content_lines(text);
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let Some(first) = first.filter(|&i| lines[i].trim() == "+++") else {
        return Ok(Parsed { header: toml::Table::new(), body: norm(&lines.join("\n")) });
    };
    let close = lines[first + 1..].iter().position(|l| l.trim() == "+++").map(|i| i + first + 1);
    let Some(close) = close else {
        return Err(TemplateError::general("The header has no closing +++ line."));
    };
    let header_text = lines[first + 1..close].join("\n");
    let header: toml::Table = header_text.parse().map_err(|e: toml::de::Error| {
        let line = e.span().map(|s| header_text[..s.start.min(header_text.len())].lines().count().max(1));
        // The line number in the file: the lines before the header, and the `+++` line.
        let msg = e.message().trim().to_string();
        match line {
            Some(n) => TemplateError::general(format!("TOML syntax error in the header, line {}: {msg}", n + first + 1)),
            None => TemplateError::general(format!("TOML syntax error in the header: {msg}")),
        }
    })?;
    let body = norm(&lines[close + 1..].join("\n"));
    Ok(Parsed { header, body })
}

/// Add error lines at the top of the header. jui removes the old error lines first.
pub fn with_errors(text: &str, errors: &[TemplateError]) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with(ERROR_PREFIX)).collect();
    let errs: Vec<String> = errors.iter().map(|e| format!("{ERROR_PREFIX} {}", e.text())).collect();
    let at = lines.iter().position(|l| l.trim() == "+++").map(|i| i + 1).unwrap_or(0);
    let mut out: Vec<String> = lines[..at].iter().map(|s| s.to_string()).collect();
    out.extend(errs);
    out.extend(lines[at..].iter().map(|s| s.to_string()));
    let mut s = out.join("\n");
    s.push('\n');
    s
}

// ------------------------------------------------------------------ build

fn pad(line: String, comment: &str) -> String {
    if comment.is_empty() {
        return line;
    }
    let width = 30usize.max(line.chars().count() + 1);
    format!("{line:<width$}# {comment}")
}

/// The help line above a field with permitted values.
fn permitted_comment(f: &ScreenField) -> Option<String> {
    if f.allowed.is_empty() {
        return None;
    }
    if f.allowed.len() <= MAX_SHOWN {
        Some(format!("# Permitted: {}", permitted_list(&f.allowed)))
    } else {
        let first: Vec<&str> = f.allowed.iter().take(MAX_SHOWN).map(|c| c.name.as_str()).collect();
        Some(format!(
            "# Permitted: {}, ... ({} values, see the list below the scissors line)",
            first.join(", "),
            f.allowed.len()
        ))
    }
}

fn kind_text(k: &FieldKind) -> String {
    match k {
        FieldKind::Text => "text".into(),
        FieldKind::Number => "number".into(),
        FieldKind::User => "username".into(),
        FieldKind::Labels => "list of labels".into(),
        FieldKind::Choice => "one permitted value".into(),
        FieldKind::MultiChoice => "list of permitted values".into(),
        FieldKind::Date => "date YYYY-MM-DD".into(),
        FieldKind::Key => "Ticket key".into(),
        FieldKind::Other(o) => format!("not supported: {o}"),
    }
}

/// The reference part: the full lists of permitted values, and the other fields.
fn reference(fields: &[ScreenField], keys: &[String], shown: &[usize]) -> Vec<String> {
    let mut out = vec![SCISSORS.to_string(), "# jui ignores this line and all lines below it.".to_string()];
    for (i, f) in fields.iter().enumerate() {
        if f.allowed.len() > MAX_SHOWN {
            out.push("#".into());
            out.push(format!("# {} (all {} values):", keys[i], f.allowed.len()));
            let mut line = String::from("#  ");
            for (n, c) in f.allowed.iter().enumerate() {
                let item = if n + 1 < f.allowed.len() { format!(" {},", c.name) } else { format!(" {}", c.name) };
                if line.len() + item.len() > 90 && line.len() > 3 {
                    out.push(line);
                    line = String::from("#  ");
                }
                line.push_str(&item);
            }
            out.push(line);
        }
    }
    let others: Vec<String> = fields
        .iter()
        .enumerate()
        .filter(|(i, f)| !shown.contains(i) && !NOT_IN_HEADER.contains(&f.id.as_str()) && !matches!(f.kind, FieldKind::Other(_)))
        .map(|(i, f)| format!("#   {} = ...  ({}, {})", keys[i], f.name, kind_text(&f.kind)))
        .collect();
    if !others.is_empty() {
        out.push("#".into());
        out.push("# Other fields that you can add to the header:".into());
        out.extend(others);
    }
    out
}

/// Find the value of a field in the preset.
fn preset_value(p: &Preset, fields: &[ScreenField], keys: &[String], i: usize) -> Option<toml::Value> {
    p.values.iter().rev().find(|(k, _)| find(fields, keys, k) == Some(i)).map(|(_, v)| flag_to_toml(Some(&fields[i]), v))
}

/// Preset keys that match no field. They go into the header, and the check reports them.
fn unknown_preset(p: &Preset, fields: &[ScreenField], keys: &[String]) -> Vec<String> {
    p.values
        .iter()
        .filter(|(k, _)| find(fields, keys, k).is_none() && !matches!(k.as_str(), "project" | "type"))
        .map(|(k, v)| format!("{} = {}", snake(k), fmt_value(&flag_to_toml(None, v))))
        .collect()
}

fn field_line(key: &str, f: &ScreenField, v: &toml::Value) -> String {
    pad(format!("{key} = {}", fmt_value(v)), if is_custom(&f.id) && key != f.id { &f.id } else { "" })
}

/// The fields of the create form in the TUI, as indexes in `meta.fields`: the summary, the
/// other required fields, the frequent fields, and the fields of the preset. The form does not
/// show the description (it has its own box), the reporter (jui sets it to `me`), or fields that
/// jui cannot set. Ctrl-E in the form opens the full Template for the other fields.
pub fn form_fields(meta: &CreateMeta, preset: &Preset) -> Vec<usize> {
    let fields = &meta.fields;
    let keys = keys(fields);
    let usable = |f: &ScreenField| {
        !NOT_IN_HEADER.contains(&f.id.as_str()) && f.id != "reporter" && !matches!(f.kind, FieldKind::Other(_))
    };
    let mut out: Vec<usize> = fields.iter().position(|f| f.id == "summary").into_iter().collect();
    for (i, f) in fields.iter().enumerate() {
        if f.required && usable(f) && !out.contains(&i) {
            out.push(i);
        }
    }
    for id in FREQUENT {
        if let Some(i) = fields.iter().position(|f| f.id == *id)
            && usable(&fields[i])
            && !out.contains(&i)
        {
            out.push(i);
        }
    }
    for i in 0..fields.len() {
        if usable(&fields[i]) && !out.contains(&i) && preset_value(preset, fields, &keys, i).is_some() {
            out.push(i);
        }
    }
    out
}

/// Write the create Template.
pub fn create_template(meta: &CreateMeta, preset: &Preset) -> String {
    let fields = &meta.fields;
    let keys = keys(fields);
    let mut shown = Vec::new();
    let mut h = vec![
        "+++".to_string(),
        format!("# Create a Ticket of type {} in {}. Save and close the editor to create it.", meta.issue_type.name, meta.project),
        "# Close with no changes to cancel. Lines that start with # are help.".to_string(),
        format!("project = {}", fmt_value(&toml::Value::String(meta.project.clone()))),
        format!("type = {}", fmt_value(&toml::Value::String(meta.issue_type.name.clone()))),
        String::new(),
        "# --- Required ---".to_string(),
    ];
    for (i, f) in fields.iter().enumerate() {
        if !f.required || NOT_IN_HEADER.contains(&f.id.as_str()) || f.id == "reporter" {
            continue;
        }
        shown.push(i);
        if let FieldKind::Other(_) = f.kind {
            h.push(format!("# {}: required, but jui cannot edit this field type. Use the Jira web page.", keys[i]));
            continue;
        }
        if let Some(c) = permitted_comment(f) {
            h.push(c);
        }
        let v = preset_value(preset, fields, &keys, i).unwrap_or_else(|| empty_for(f));
        h.push(field_line(&keys[i], f, &v));
    }
    h.push(String::new());
    h.push("# --- Optional (remove the # to use) ---".to_string());
    let mut optional: Vec<usize> = FREQUENT
        .iter()
        .filter_map(|id| fields.iter().position(|f| f.id == *id))
        .chain(fields.iter().position(|f| f.id == "reporter"))
        .filter(|&i| !fields[i].required || fields[i].id == "reporter")
        .collect();
    // Fields from flags that are not in a group.
    for i in 0..fields.len() {
        if !shown.contains(&i) && !optional.contains(&i) && preset_value(preset, fields, &keys, i).is_some() {
            optional.push(i);
        }
    }
    for i in optional {
        let f = &fields[i];
        shown.push(i);
        match preset_value(preset, fields, &keys, i) {
            Some(v) => h.push(field_line(&keys[i], f, &v)),
            None => {
                let example = match (&f.kind, f.id.as_str()) {
                    (_, "assignee" | "reporter") => toml::Value::String("me".into()),
                    (FieldKind::Number, _) => toml::Value::Integer(1),
                    _ => empty_for(f),
                };
                let comment = match f.allowed.len() {
                    0 => String::new(),
                    n if n <= MAX_SHOWN => format!("Permitted: {}", permitted_list(&f.allowed)),
                    n => format!("{n} values, see the list below the scissors line"),
                };
                let id = if is_custom(&f.id) { f.id.clone() } else { String::new() };
                let comment = [id, comment].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                h.push(pad(format!("# {} = {}", keys[i], fmt_value(&example)), &comment));
            }
        }
    }
    h.extend(unknown_preset(preset, fields, &keys));
    h.push("+++".to_string());
    h.push(String::new());
    h.push(preset.description.clone().unwrap_or_else(|| PLACEHOLDER.to_string()));
    h.push(String::new());
    h.extend(reference(fields, &keys, &shown));
    let mut s = h.join("\n");
    s.push('\n');
    s
}

/// The fields that the edit Template shows: the fields with a value now, and the frequent fields.
fn edit_fields(data: &EditData) -> Vec<usize> {
    let fields = &data.fields;
    let mut out: Vec<usize> = fields.iter().position(|f| f.id == "summary").into_iter().collect();
    for (i, f) in fields.iter().enumerate() {
        if out.contains(&i) || NOT_IN_HEADER.contains(&f.id.as_str()) || matches!(f.kind, FieldKind::Other(_)) {
            continue;
        }
        let has_value = data.values.get(&f.id).and_then(|v| from_json(f, v)).is_some();
        if has_value || FREQUENT.contains(&f.id.as_str()) {
            out.push(i);
        }
    }
    // The standard fields first (in the order of the fixed keys), then the custom fields.
    let rank = |i: &usize| {
        let f = &fields[*i];
        let fixed = FIXED.iter().position(|(_, id)| *id == f.id).unwrap_or(FIXED.len());
        (fixed, !f.required, f.name.to_lowercase())
    };
    out.sort_by_key(rank);
    out
}

/// Write the edit Template. `body` is the Markdown (edit mode) or the raw wiki markup.
pub fn edit_template(data: &EditData, body: &str, wiki: bool, preset: &Preset) -> String {
    let fields = &data.fields;
    let keys = keys(fields);
    let shown = edit_fields(data);
    let mut h = vec![
        "+++".to_string(),
        format!("# Edit {}. Save and close the editor to send the changes.", data.key),
        "# Close with no changes to cancel. jui sends only the fields that you change.".to_string(),
    ];
    if wiki {
        h.push("# The body is raw Jira wiki markup. jui sends it with no conversion.".into());
    }
    h.push(format!("# project = {}   (jui cannot change the project or the type)", fmt_value(&toml::Value::String(data.project.clone()))));
    h.push(format!("# type = {}", fmt_value(&toml::Value::String(data.issue_type.clone()))));
    for &i in &shown {
        let f = &fields[i];
        if let Some(c) = permitted_comment(f) {
            h.push(c);
        }
        let current = data.values.get(&f.id).and_then(|v| from_json(f, v));
        match preset_value(preset, fields, &keys, i).or(current) {
            Some(v) => h.push(field_line(&keys[i], f, &v)),
            // A frequent field with no value: a comment, as in the create Template.
            None => {
                let example = match (&f.kind, f.id.as_str()) {
                    (_, "assignee") => toml::Value::String("me".into()),
                    (FieldKind::Number, _) => toml::Value::Integer(1),
                    _ => empty_for(f),
                };
                h.push(format!("# {}", field_line(&keys[i], f, &example)));
            }
        }
    }
    // Fields from flags that the Template does not show.
    for i in 0..fields.len() {
        if !shown.contains(&i)
            && let Some(v) = preset_value(preset, fields, &keys, i)
        {
            h.push(field_line(&keys[i], &fields[i], &v));
        }
    }
    h.extend(unknown_preset(preset, fields, &keys));
    h.push("+++".to_string());
    h.push(String::new());
    h.push(preset.description.clone().unwrap_or_else(|| body.to_string()));
    h.push(String::new());
    h.extend(reference(fields, &keys, &shown));
    let mut s = h.join("\n");
    s.push('\n');
    s
}

// ------------------------------------------------------------------ session

/// A change of the description, with the old and the new wiki text for the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescChange {
    /// The old wiki text. Empty when the Ticket had no description.
    pub old: String,
    pub new: String,
    /// Losses that the Markdown-to-wiki conversion reports.
    pub warnings: Vec<String>,
}

impl DescChange {
    /// True when jui must show a diff: the change replaces existing text.
    pub fn needs_diff(&self) -> bool {
        !self.old.trim().is_empty()
    }
}

/// The fields that an edit sends.
#[derive(Debug, Clone, PartialEq)]
pub struct EditPlan {
    /// The `fields` object of `PUT /rest/api/2/issue/{key}`.
    pub fields: Map<String, Value>,
    /// The keys of the changed fields, for the messages.
    pub changed: Vec<String>,
    pub description: Option<DescChange>,
}

impl EditPlan {
    /// The full `fields` object, with the description.
    pub fn body(&self) -> Value {
        let mut f = self.fields.clone();
        if let Some(d) = &self.description {
            f.insert("description".into(), json!(d.new));
        }
        json!({ "fields": f })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// The body of `POST /rest/api/2/issue`.
    Create(Value),
    Edit(EditPlan),
}

/// What the caller must do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Stop with no request. The text is the message, for example "No changes.".
    Cancel(String),
    /// Open the editor again with `Session::text()`. It has the error lines.
    Reopen,
    /// Stop and write a Draft. The text is the reason.
    Stop(String),
    /// Send the request.
    Send(Request),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Create(CreateMeta),
    Edit { data: EditData, wiki: bool },
}

/// The state of one create or edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub mode: Mode,
    me: String,
    keys: Vec<String>,
    /// The first text that jui wrote.
    written: String,
    /// The text for the next editor start.
    shown: String,
    /// The errors in `shown`.
    errors: Vec<TemplateError>,
    /// The last text that the user saved.
    saved: Option<String>,
    /// Edit: the body that jui wrote (normalized Markdown, or wiki markup in raw wiki mode).
    base_body: String,
}

impl Session {
    /// Start a create. `me` is the username of the current user.
    pub fn create(meta: CreateMeta, me: &str, preset: &Preset) -> Session {
        let written = create_template(&meta, preset);
        let keys = keys(&meta.fields);
        Session::new(Mode::Create(meta), me, keys, written, String::new())
    }

    /// Start an edit. In raw wiki mode, the body is the wiki markup with no conversion.
    pub fn edit(data: EditData, me: &str, wiki: bool, preset: &Preset) -> Session {
        let wiki_text = data.description.clone().unwrap_or_default();
        let body = if wiki { norm(&wiki_text) } else { norm(&markup::wiki_to_markdown(&wiki_text).text) };
        let written = edit_template(&data, &body, wiki, preset);
        let keys = keys(&data.fields);
        Session::new(Mode::Edit { data, wiki }, me, keys, written, body)
    }

    fn new(mode: Mode, me: &str, keys: Vec<String>, written: String, base_body: String) -> Session {
        Session { mode, me: me.to_string(), keys, shown: written.clone(), written, errors: vec![], saved: None, base_body }
    }

    /// The username of the current user (`me`).
    pub fn me(&self) -> &str {
        &self.me
    }

    /// The text for the editor.
    pub fn text(&self) -> &str {
        &self.shown
    }

    /// The errors of the last check (for a script, which cannot open the editor).
    pub fn errors(&self) -> &[TemplateError] {
        &self.errors
    }

    /// The text for a Draft: the last text that the user saved, or the Template.
    pub fn draft_text(&self) -> &str {
        self.saved.as_deref().unwrap_or(&self.shown)
    }

    /// The Ticket key (edit) or the project (create), for file names and messages.
    pub fn label(&self) -> &str {
        match &self.mode {
            Mode::Create(m) => &m.project,
            Mode::Edit { data, .. } => &data.key,
        }
    }

    fn fields(&self) -> &[ScreenField] {
        match &self.mode {
            Mode::Create(m) => &m.fields,
            Mode::Edit { data, .. } => &data.fields,
        }
    }

    fn no_change_message(&self) -> String {
        match &self.mode {
            Mode::Create(_) => "Cancelled. No Ticket created.".into(),
            Mode::Edit { .. } => "No changes.".into(),
        }
    }

    /// Check the text that the user saved in the editor.
    pub fn submit(&mut self, saved: &str) -> Step {
        self.check(saved, true)
    }

    /// Check a text that did not come from the editor (`--from`, or the Template from flags).
    /// For a create, an unchanged text is not a cancel.
    pub fn submit_given(&mut self, text: &str) -> Step {
        self.check(text, false)
    }

    /// Check the values of the create form (field keys and texts, as from flags). Returns the
    /// body of the create request, or the errors with the field keys.
    pub fn submit_form(&mut self, preset: &Preset) -> Result<Value, Vec<TemplateError>> {
        let Mode::Create(meta) = &self.mode else {
            return Err(vec![TemplateError::general("The form can only create a Ticket.")]);
        };
        let text = create_template(meta, preset);
        let result = self.create_request(&text).map(|r| r.unwrap_or_else(|| json!({ "fields": {} })));
        self.saved = Some(text.clone());
        match result {
            Ok(body) => {
                self.errors.clear();
                self.shown = text;
                Ok(body)
            }
            Err(errs) => {
                self.shown = with_errors(&text, &errs);
                self.errors = errs.clone();
                Err(errs)
            }
        }
    }

    /// Put the values of the create form into the Template, for the editor (Ctrl-E) or for a
    /// Draft. The old errors go away.
    pub fn use_form(&mut self, preset: &Preset) {
        if let Mode::Create(meta) = &self.mode {
            self.shown = create_template(meta, preset);
            // A save with no change in the editor is a cancel, as for a new Template.
            self.written = self.shown.clone();
            self.errors.clear();
            self.saved = None;
        }
    }

    fn check(&mut self, saved: &str, from_editor: bool) -> Step {
        if saved.trim().is_empty() {
            return Step::Cancel(self.no_change_message());
        }
        if !self.errors.is_empty() && content(saved) == content(&self.shown) && norm(saved) == norm(&self.shown) {
            self.saved = Some(saved.to_string());
            return Step::Stop("The Template is not valid, and you did not change it.".into());
        }
        if from_editor && matches!(self.mode, Mode::Create(_)) && content(saved) == content(&self.written) {
            return Step::Cancel(self.no_change_message());
        }
        self.saved = Some(saved.to_string());
        let result = match &self.mode {
            Mode::Create(_) => self.create_request(saved).map(|r| r.map(Request::Create)),
            Mode::Edit { .. } => self.edit_request(saved).map(|r| r.map(Request::Edit)),
        };
        match result {
            Ok(Some(r)) => {
                self.errors.clear();
                Step::Send(r)
            }
            Ok(None) => Step::Cancel(self.no_change_message()),
            Err(errs) => self.reopen(saved, errs),
        }
    }

    fn reopen(&mut self, saved: &str, errs: Vec<TemplateError>) -> Step {
        self.shown = with_errors(saved, &errs);
        self.errors = errs;
        Step::Reopen
    }

    /// Open the editor again with a note at the top, for example after `n` in the diff
    /// question. The note is not an error: an unchanged save gives the same request again.
    pub fn reopen_note(&mut self, note: &str) {
        let saved = self.saved.clone().unwrap_or_else(|| self.shown.clone());
        self.shown = with_errors(&saved, &[TemplateError::general(note)]);
        self.errors.clear();
    }

    /// Jira did not accept the request. A validation error opens the Template again with
    /// error lines (field IDs become keys). Other errors come back unchanged.
    pub fn rejected(&mut self, e: Error) -> Result<Step, Error> {
        let Error::Validation { fields, messages } = e else { return Err(e) };
        let mut errs: Vec<TemplateError> = messages.iter().map(|m| TemplateError::general(m.clone())).collect();
        for (id, m) in &fields {
            let key = self
                .fields()
                .iter()
                .position(|f| &f.id == id || &f.name == id)
                .map(|i| self.keys[i].clone())
                .unwrap_or_else(|| id.clone());
            errs.push(TemplateError::new(&key, m.clone()));
        }
        if errs.is_empty() {
            errs.push(TemplateError::general("Jira did not accept the request."));
        }
        let saved = self.saved.clone().unwrap_or_else(|| self.shown.clone());
        Ok(self.reopen(&saved, errs))
    }

    /// The header keys that are not fields of the screen.
    fn unknown_keys(&self, header: &toml::Table, special: &[&str]) -> Vec<TemplateError> {
        let fields = self.fields();
        header
            .keys()
            .filter(|k| !special.contains(&k.as_str()) && find(fields, &self.keys, k).is_none())
            .map(|k| TemplateError::new(k, "is not a field of this screen. See the list below the scissors line."))
            .collect()
    }

    fn create_request(&self, text: &str) -> Result<Option<Value>, Vec<TemplateError>> {
        let Mode::Create(meta) = &self.mode else { unreachable!() };
        let p = parse(text).map_err(|e| vec![e])?;
        let mut errs = self.unknown_keys(&p.header, &["project", "type"]);
        let same = |k: &str, want: &str| match p.header.get(k) {
            None => true,
            Some(toml::Value::String(s)) => s.trim().eq_ignore_ascii_case(want),
            Some(_) => false,
        };
        if !same("project", &meta.project) {
            errs.push(TemplateError::new("project", format!("jui cannot change the project here. Start again with -p. This Template is for {}.", meta.project)));
        }
        if !same("type", &meta.issue_type.name) {
            errs.push(TemplateError::new("type", format!("jui cannot change the type here. Start again with -t. This Template is for {}.", meta.issue_type.name)));
        }
        let mut fields = Map::new();
        fields.insert("project".into(), json!({ "key": meta.project }));
        fields.insert("issuetype".into(), json!({ "id": meta.issue_type.id }));
        for (k, v) in &p.header {
            if matches!(k.as_str(), "project" | "type") {
                continue;
            }
            let Some(i) = find(&meta.fields, &self.keys, k) else { continue };
            let f = &meta.fields[i];
            match to_json(f, v, &self.me, &meta.project) {
                Ok(Conv::Json(j)) => {
                    fields.insert(f.id.clone(), j);
                }
                Ok(Conv::Empty) => {}
                Err(m) => errs.push(TemplateError::new(&self.keys[i], m)),
            }
        }
        if meta.fields.iter().any(|f| f.id == "reporter") && !fields.contains_key("reporter") {
            fields.insert("reporter".into(), json!({ "name": self.me }));
        }
        for (i, f) in meta.fields.iter().enumerate() {
            if !f.required || f.has_default || fields.contains_key(&f.id) || matches!(f.id.as_str(), "description") {
                continue;
            }
            if errs.iter().any(|e| e.key.as_deref() == Some(self.keys[i].as_str())) {
                continue;
            }
            let msg = match f.kind {
                FieldKind::Other(_) => "is required, but jui cannot edit this field type. Use the Jira web page.".to_string(),
                _ => "a value is required.".to_string(),
            };
            errs.push(TemplateError::new(&self.keys[i], msg));
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let body = p.body.trim();
        if !body.is_empty() && body != PLACEHOLDER {
            let wiki = markup::markdown_to_wiki(body).text;
            fields.insert("description".into(), json!(wiki));
        }
        Ok(Some(json!({ "fields": fields })))
    }

    fn edit_request(&self, text: &str) -> Result<Option<EditPlan>, Vec<TemplateError>> {
        let Mode::Edit { data, wiki } = &self.mode else { unreachable!() };
        let p = parse(text).map_err(|e| vec![e])?;
        let mut errs = self.unknown_keys(&p.header, &[]);
        for k in ["project", "type"] {
            if p.header.contains_key(k) {
                errs.push(TemplateError::new(k, "a change of project or type is not in the MVP. Remove this line."));
            }
        }
        let mut plan = EditPlan { fields: Map::new(), changed: vec![], description: None };
        for (k, v) in &p.header {
            let Some(i) = find(&data.fields, &self.keys, k) else { continue };
            let f = &data.fields[i];
            let current = data.values.get(&f.id).and_then(|x| from_json(f, x));
            let unchanged = match &current {
                Some(c) => c == v,
                None => is_empty(v),
            };
            if unchanged {
                continue;
            }
            match to_json(f, v, &self.me, &data.project) {
                Ok(Conv::Json(j)) => {
                    plan.fields.insert(f.id.clone(), j);
                    plan.changed.push(self.keys[i].clone());
                }
                Ok(Conv::Empty) if f.required => errs.push(TemplateError::new(&self.keys[i], "a value is required.")),
                Ok(Conv::Empty) => {
                    plan.fields.insert(f.id.clone(), clear_value(f));
                    plan.changed.push(self.keys[i].clone());
                }
                Err(m) => errs.push(TemplateError::new(&self.keys[i], m)),
            }
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        if norm(&p.body) != self.base_body {
            let old = data.description.clone().unwrap_or_default();
            let (new, warnings) = if *wiki {
                (norm(&p.body), vec![])
            } else {
                let c = markup::markdown_to_wiki(&p.body);
                (c.text, c.warnings)
            };
            if norm(&new) != norm(&old) {
                plan.description = Some(DescChange { old, new, warnings });
            }
        }
        if plan.fields.is_empty() && plan.description.is_none() {
            return Ok(None);
        }
        Ok(Some(plan))
    }
}

/// The names of the fields that changed in Jira between two reads of a Ticket.
pub fn changed_in_jira(old: &EditData, new: &EditData) -> Vec<String> {
    let mut out = Vec::new();
    if old.summary != new.summary {
        out.push("Summary".to_string());
    }
    if old.description != new.description {
        out.push("Description".to_string());
    }
    for f in &new.fields {
        if matches!(f.id.as_str(), "summary" | "description" | "comment" | "attachment" | "issuelinks") {
            continue;
        }
        if old.values.get(&f.id) != new.values.get(&f.id) {
            out.push(f.name.clone());
        }
    }
    out
}

// ------------------------------------------------------------------ diff

/// One line of a diff: `-` (removed), `+` (added), or ` ` (the same).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub tag: char,
    pub text: String,
}

/// A line diff of two wiki texts, with 3 lines of context around each change.
/// `...` lines show where jui removed unchanged lines.
pub fn diff(old: &str, new: &str) -> Vec<DiffLine> {
    use similar::{ChangeTag, TextDiff};
    // Jira texts can have CRLF line ends. jui sends LF, and Jira shows both the same.
    let (old, new) = (old.replace("\r\n", "\n"), new.replace("\r\n", "\n"));
    let d = TextDiff::from_lines(&old, &new);
    let mut out = Vec::new();
    for (n, group) in d.grouped_ops(3).iter().enumerate() {
        if n > 0 {
            out.push(DiffLine { tag: ' ', text: "...".into() });
        }
        for op in group {
            for c in d.iter_changes(op) {
                let tag = match c.tag() {
                    ChangeTag::Delete => '-',
                    ChangeTag::Insert => '+',
                    ChangeTag::Equal => ' ',
                };
                out.push(DiffLine { tag, text: c.value().trim_end_matches('\n').to_string() });
            }
        }
    }
    out
}

// ------------------------------------------------------------------ Drafts

/// The Draft directory: `$XDG_STATE_HOME/jui/drafts` or `~/.local/state/jui/drafts`.
pub fn draft_dir() -> PathBuf {
    use etcetera::BaseStrategy;
    match etcetera::choose_base_strategy() {
        Ok(s) => s.state_dir().unwrap_or_else(|| s.home_dir().join(".local/state")).join("jui").join("drafts"),
        Err(_) => PathBuf::from(".local/state/jui/drafts"),
    }
}

/// Write a Draft. Old Drafts (more than 30 days) are deleted first. Returns the path.
pub fn write_draft(dir: &Path, label: &str, text: &str) -> Result<PathBuf, Error> {
    let io = |e: std::io::Error| Error::input(format!("Cannot write the Draft in {}: {e}", dir.display()));
    std::fs::create_dir_all(dir).map_err(io)?;
    prune_drafts(dir, std::time::Duration::from_secs(DRAFT_DAYS * 24 * 3600));
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let mut path = dir.join(format!("{stamp}-{safe}.md"));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("{stamp}-{safe}-{n}.md"));
    }
    std::fs::write(&path, text).map_err(io)?;
    Ok(path)
}

/// Delete the Drafts that are older than `age`.
pub fn prune_drafts(dir: &Path, age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        let p = e.path();
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|d| d > age);
        if old && p.extension().is_some_and(|x| x == "md") {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// A message that tells where the Draft is.
pub fn draft_hint(path: &Path, key_or_new: &str) -> String {
    format!("Your text is in the Draft {}. Use it again with `jui {key_or_new} --from {}`.", path.display(), path.display())
}

/// The errors as text lines (for a script and for `Error` hints).
pub fn errors_text(errs: &[TemplateError]) -> String {
    errs.iter().map(TemplateError::text).collect::<Vec<_>>().join("\n")
}

/// Field keys by ID, for tests and help.
pub fn key_map(fields: &[ScreenField]) -> BTreeMap<String, String> {
    fields.iter().zip(keys(fields)).map(|(f, k)| (f.id.clone(), k)).collect()
}

#[cfg(test)]
mod tests;
