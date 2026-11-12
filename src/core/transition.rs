//! Transition logic that the CLI and the TUI share: the target match, the field values,
//! and the field names in errors. Pure functions. See ticket 05 "jui CLI command tree", part 4.

use serde_json::{Map, Value, json};

use super::error::{Error, Result};
use super::model::{Choice, FieldKind, ScreenField, Transition};

/// The result of a target match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    One(usize),
    /// More than one Transition matches. The indexes of the candidates.
    Many(Vec<usize>),
    None,
}

/// True when `name` starts with `target`, or a word of `name` starts with `target`.
/// Both must be lower case. Example: `pull` matches `in pull request`.
fn word_prefix(name: &str, target: &str) -> bool {
    name.starts_with(target)
        || name.match_indices(target).any(|(i, _)| !name[..i].ends_with(|c: char| c.is_alphanumeric()))
}

/// Find the Transition for a target. The order:
/// 1. the destination Status name (exact), 2. the Transition name (exact),
/// 3. a word prefix of the Status name, 4. a word prefix of the Transition name.
///
/// The match ignores case. The first step that finds Transitions gives the result.
pub fn match_target(ts: &[Transition], target: &str) -> Match {
    let t = target.trim().to_lowercase();
    if t.is_empty() {
        return Match::None;
    }
    let steps: [&dyn Fn(&Transition) -> bool; 4] = [
        &|x| x.to.to_lowercase() == t,
        &|x| x.name.to_lowercase() == t,
        &|x| word_prefix(&x.to.to_lowercase(), &t),
        &|x| word_prefix(&x.name.to_lowercase(), &t),
    ];
    for step in steps {
        let found: Vec<usize> = ts.iter().enumerate().filter(|(_, x)| step(x)).map(|(i, _)| i).collect();
        match found.len() {
            0 => continue,
            1 => return Match::One(found[0]),
            _ => return Match::Many(found),
        }
    }
    Match::None
}

/// The text that shows a Transition: the destination Status, and the Transition name
/// when it is different.
pub fn label(t: &Transition) -> String {
    if t.name.eq_ignore_ascii_case(&t.to) { t.to.clone() } else { format!("{} (via {})", t.to, t.name) }
}

/// True when the filter text is in the Status name or the Transition name (ignores case).
pub fn filter_matches(t: &Transition, filter: &str) -> bool {
    let f = filter.to_lowercase();
    t.to.to_lowercase().contains(&f) || t.name.to_lowercase().contains(&f)
}

/// A field key in snake case, as the Template uses it: "Fix Version/s" becomes "fix_version_s".
pub fn snake(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

/// Find a screen field by its ID, its name, or its snake-case name (ignores case).
pub fn find_field<'a>(fields: &'a [ScreenField], key: &str) -> Option<&'a ScreenField> {
    let k = key.trim();
    fields
        .iter()
        .find(|f| f.id.eq_ignore_ascii_case(k) || f.name.eq_ignore_ascii_case(k) || snake(&f.name) == snake(k))
}

/// The required fields that have no value and no default value.
pub fn missing_required<'a>(t: &'a Transition, values: &Map<String, Value>) -> Vec<&'a ScreenField> {
    t.fields.iter().filter(|f| f.required && !f.has_default && !values.contains_key(&f.id)).collect()
}

/// Find a permitted value by its name or ID: exact first, then a unique prefix (ignores case).
pub fn pick<'a>(field: &'a ScreenField, text: &str) -> Result<&'a Choice> {
    let t = text.trim().to_lowercase();
    if let Some(c) = field.allowed.iter().find(|c| c.name.to_lowercase() == t || c.id == t) {
        return Ok(c);
    }
    let found: Vec<&Choice> = field.allowed.iter().filter(|c| c.name.to_lowercase().starts_with(&t)).collect();
    match found.as_slice() {
        [one] => Ok(one),
        _ => Err(Error::input_hint(
            format!("`{}` is not a permitted value of {}.", text.trim(), field.name),
            format!("Use one of: {}.", names(&field.allowed)),
        )),
    }
}

fn names(cs: &[Choice]) -> String {
    const MAX: usize = 15;
    let mut shown: Vec<&str> = cs.iter().take(MAX).map(|c| c.name.as_str()).collect();
    if cs.len() > MAX {
        shown.push("…");
    }
    shown.join(", ")
}

/// Make the JSON value of a field from the text of the user.
pub fn field_value(field: &ScreenField, text: &str) -> Result<Value> {
    let t = text.trim();
    let bad = |what: &str| Error::input(format!("{}: {what}", field.name));
    match &field.kind {
        FieldKind::Text | FieldKind::Key => Ok(json!(t)),
        FieldKind::Date => chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
            .map(|_| json!(t))
            .map_err(|_| bad("use a date, YYYY-MM-DD.")),
        FieldKind::Number => t.parse::<f64>().map(|n| json!(n)).map_err(|_| bad("use a number.")),
        FieldKind::User => Ok(json!({ "name": t })),
        FieldKind::Labels => Ok(Value::Array(split(t).map(|s| json!(s)).collect())),
        FieldKind::Choice => Ok(json!({ "id": pick(field, t)?.id })),
        FieldKind::MultiChoice => {
            let ids: Result<Vec<Value>> = split(t).map(|s| pick(field, s).map(|c| json!({ "id": c.id }))).collect();
            Ok(Value::Array(ids?))
        }
        FieldKind::Other(kind) => Err(Error::input_hint(
            format!("jui cannot set the field {} (type `{kind}`).", field.name),
            "Use the Jira web page for this Transition.",
        )),
    }
}

fn split(t: &str) -> impl Iterator<Item = &str> {
    t.split(',').map(str::trim).filter(|s| !s.is_empty())
}

/// The body of `POST /rest/api/2/issue/{key}/transitions`.
pub fn request_body(id: &str, fields: &Map<String, Value>, comment_wiki: Option<&str>) -> Value {
    let mut body = json!({ "transition": { "id": id } });
    if !fields.is_empty() {
        body["fields"] = Value::Object(fields.clone());
    }
    if let Some(c) = comment_wiki.filter(|c| !c.trim().is_empty()) {
        body["update"] = json!({ "comment": [{ "add": { "body": c } }] });
    }
    body
}

/// Replace field IDs with field names in a validation error.
pub fn name_fields(e: Error, fields: &[ScreenField]) -> Error {
    match e {
        Error::Validation { fields: errs, messages } => {
            let errs = errs
                .into_iter()
                .map(|(id, m)| (fields.iter().find(|f| f.id == id).map(|f| f.name.clone()).unwrap_or(id), m))
                .collect();
            Error::Validation { fields: errs, messages }
        }
        e => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::StatusCategory;

    fn tr(id: &str, name: &str, to: &str) -> Transition {
        Transition { id: id.into(), name: name.into(), to: to.into(), to_category: StatusCategory::InProgress, fields: vec![] }
    }

    fn all() -> Vec<Transition> {
        vec![
            tr("11", "To Do", "To Do"),
            tr("21", "In Progress", "In Progress"),
            tr("31", "Done", "Done"),
            tr("41", "In Pull Request", "In Pull Request"),
            tr("71", "Close", "Closed"),
        ]
    }

    #[test]
    fn status_name_first_then_transition_name() {
        let ts = all();
        assert_eq!(match_target(&ts, "done"), Match::One(2));
        assert_eq!(match_target(&ts, "closed"), Match::One(4));
        assert_eq!(match_target(&ts, "close"), Match::One(4), "the Transition name");
    }

    #[test]
    fn word_prefixes() {
        let ts = all();
        assert_eq!(match_target(&ts, "pull"), Match::One(3));
        assert_eq!(match_target(&ts, "prog"), Match::One(1));
        assert_eq!(match_target(&ts, "in"), Match::Many(vec![1, 3]));
        assert_eq!(match_target(&ts, "ull"), Match::None, "not the start of a word");
        assert_eq!(match_target(&ts, "x"), Match::None);
    }

    #[test]
    fn labels_show_the_transition_name_when_different() {
        assert_eq!(label(&tr("1", "Done", "Done")), "Done");
        assert_eq!(label(&tr("1", "Close", "Closed")), "Closed (via Close)");
    }

    fn field(kind: FieldKind) -> ScreenField {
        ScreenField {
            id: "resolution".into(),
            name: "Resolution".into(),
            required: true,
            has_default: false,
            kind,
            allowed: vec![
                Choice { id: "10001".into(), name: "Won't Do".into() },
                Choice { id: "10002".into(), name: "Duplicate".into() },
                Choice { id: "10401".into(), name: "Done".into() },
            ],
        }
    }

    #[test]
    fn choice_values() {
        let f = field(FieldKind::Choice);
        assert_eq!(field_value(&f, "done").unwrap(), json!({"id": "10401"}));
        assert_eq!(field_value(&f, "dup").unwrap(), json!({"id": "10002"}));
        assert_eq!(field_value(&f, "10001").unwrap(), json!({"id": "10001"}));
        let e = field_value(&f, "d").unwrap_err();
        assert_eq!(e.hint().unwrap(), "Use one of: Won't Do, Duplicate, Done.");
        let m = field(FieldKind::MultiChoice);
        assert_eq!(field_value(&m, "done, dup").unwrap(), json!([{"id": "10401"}, {"id": "10002"}]));
    }

    #[test]
    fn other_values() {
        assert_eq!(field_value(&field(FieldKind::Number), "3").unwrap(), json!(3.0));
        assert!(field_value(&field(FieldKind::Number), "x").is_err());
        assert_eq!(field_value(&field(FieldKind::User), "jdoe").unwrap(), json!({"name": "jdoe"}));
        assert_eq!(field_value(&field(FieldKind::Labels), "a, b").unwrap(), json!(["a", "b"]));
        assert!(field_value(&field(FieldKind::Other("any".into())), "x").is_err());
    }

    #[test]
    fn fields_by_id_name_or_snake() {
        let mut f = field(FieldKind::Choice);
        f.id = "customfield_10021".into();
        f.name = "Story Points".into();
        let fs = vec![f];
        assert!(find_field(&fs, "customfield_10021").is_some());
        assert!(find_field(&fs, "story points").is_some());
        assert!(find_field(&fs, "story_points").is_some());
        assert!(find_field(&fs, "points").is_none());
        assert_eq!(snake("Fix Version/s"), "fix_version_s");
    }

    #[test]
    fn body_has_fields_and_comment_only_when_given() {
        assert_eq!(request_body("41", &Map::new(), None), json!({"transition": {"id": "41"}}));
        let mut m = Map::new();
        m.insert("resolution".into(), json!({"id": "10401"}));
        assert_eq!(
            request_body("31", &m, Some("*Done*")),
            json!({"transition": {"id": "31"}, "fields": {"resolution": {"id": "10401"}},
                   "update": {"comment": [{"add": {"body": "*Done*"}}]}})
        );
    }

    #[test]
    fn validation_errors_get_field_names() {
        let mut errs = std::collections::BTreeMap::new();
        errs.insert("resolution".to_string(), "Resolution is required.".to_string());
        errs.insert("customfield_1".to_string(), "x".to_string());
        let e = name_fields(Error::Validation { fields: errs, messages: vec![] }, &[field(FieldKind::Choice)]);
        let Error::Validation { fields, .. } = e else { panic!() };
        assert_eq!(fields.keys().collect::<Vec<_>>(), ["Resolution", "customfield_1"]);
    }
}
