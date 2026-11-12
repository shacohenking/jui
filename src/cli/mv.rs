//! `jui mv <key> [target]`: move a Ticket with a Transition.
//! See ticket 05 "jui CLI command tree", part 4, and the common rule for terminals and scripts.

use std::io::{BufRead, Write};

use serde_json::{Map, Value};

use super::args::MoveArgs;
use super::prompt;
use crate::core::model::{FieldKind, ScreenField, Transition};
use crate::core::transition::{self as tr, Match};
use crate::core::{Error, Jira, Result};

/// Where the questions come from. `None` means a script: jui does not ask, it stops with an error.
pub struct Asker<'a, R: BufRead, W: Write> {
    pub input: Option<&'a mut R>,
    pub out: &'a mut W,
}

/// Select the Transition and collect the field values. No request is sent here.
pub fn plan<R: BufRead, W: Write>(
    key: &str,
    ts: &[Transition],
    a: &MoveArgs,
    ask: &mut Asker<'_, R, W>,
) -> Result<(usize, Map<String, Value>)> {
    if ts.is_empty() {
        return Err(Error::input(format!("{key} has no Transitions that you can do.")));
    }
    let labels: Vec<String> = ts.iter().map(tr::label).collect();
    let targets = || labels.join(", ");
    let found = match a.target.as_deref() {
        Some(t) => tr::match_target(ts, t),
        None => Match::Many((0..ts.len()).collect()),
    };
    let index = match found {
        Match::One(i) => i,
        Match::None => {
            return Err(Error::input_hint(
                format!("No Transition of {key} matches `{}`.", a.target.as_deref().unwrap_or("")),
                format!("Use one of: {}.", targets()),
            ));
        }
        Match::Many(candidates) => match ask.input.as_deref_mut() {
            Some(input) => {
                let shown: Vec<String> = candidates.iter().map(|&i| labels[i].clone()).collect();
                let title = format!("Move {key} to:");
                candidates[prompt::select(input, ask.out, &title, &shown)?]
            }
            None => {
                let what = match &a.target {
                    Some(t) => format!("`{t}` matches more than one Transition of {key}."),
                    None => format!("Give the target Status for {key}."),
                };
                let shown: Vec<String> = candidates.iter().map(|&i| labels[i].clone()).collect();
                return Err(Error::input_hint(what, format!("Use one of: {}.", shown.join(", "))));
            }
        },
    };
    let t = &ts[index];
    let values = field_values(t, a, ask)?;
    Ok((index, values))
}

fn field_values<R: BufRead, W: Write>(t: &Transition, a: &MoveArgs, ask: &mut Asker<'_, R, W>) -> Result<Map<String, Value>> {
    let mut values = Map::new();
    if let Some(r) = &a.resolution {
        let f = t.fields.iter().find(|f| f.id == "resolution").ok_or_else(|| {
            Error::input_hint(
                format!("The Transition to {} has no resolution field.", t.to),
                "Remove --resolution.",
            )
        })?;
        values.insert(f.id.clone(), tr::field_value(f, r)?);
    }
    for s in &a.set {
        let (name, value) = s.split_once('=').ok_or_else(|| {
            Error::input_hint(format!("`{s}` is not FIELD=VALUE."), "Example: --set \"Story Points=3\"")
        })?;
        let f = tr::find_field(&t.fields, name).ok_or_else(|| {
            let names: Vec<&str> = t.fields.iter().map(|f| f.name.as_str()).collect();
            let hint = if names.is_empty() {
                "This Transition has no fields.".to_string()
            } else {
                format!("Fields of this Transition: {}.", names.join(", "))
            };
            Error::input_hint(format!("The Transition to {} has no field `{}`.", t.to, name.trim()), hint)
        })?;
        values.insert(f.id.clone(), tr::field_value(f, value)?);
    }
    let missing: Vec<ScreenField> = tr::missing_required(t, &values).into_iter().cloned().collect();
    for f in missing {
        match ask.input.as_deref_mut() {
            Some(input) => {
                let v = ask_field(input, ask.out, &f)?;
                values.insert(f.id.clone(), v);
            }
            None => {
                let flag = if f.id == "resolution" { "--resolution <name>".to_string() } else { format!("--set \"{}=<value>\"", f.name) };
                let mut hint = format!("Use {flag}.");
                if !f.allowed.is_empty() {
                    let names: Vec<&str> = f.allowed.iter().map(|c| c.name.as_str()).collect();
                    hint.push_str(&format!(" Values: {}.", names.join(", ")));
                }
                return Err(Error::input_hint(format!("The Transition to {} needs {}.", t.to, f.name), hint));
            }
        }
    }
    Ok(values)
}

fn ask_field(input: &mut impl BufRead, out: &mut impl Write, f: &ScreenField) -> Result<Value> {
    if matches!(f.kind, FieldKind::Choice) {
        let names: Vec<String> = f.allowed.iter().map(|c| c.name.clone()).collect();
        let i = prompt::select(input, out, &format!("{} (required):", f.name), &names)?;
        return Ok(serde_json::json!({ "id": f.allowed[i].id }));
    }
    loop {
        let text = prompt::ask(input, out, &format!("{} (required): ", f.name))?;
        if text.is_empty() {
            return Err(Error::Cancelled);
        }
        match tr::field_value(f, &text) {
            Ok(v) => return Ok(v),
            Err(e) => {
                let _ = writeln!(out, "{e}");
                if let Some(h) = e.hint() {
                    let _ = writeln!(out, "{h}");
                }
            }
        }
    }
}

/// Run `jui mv`. Returns the Transition that jui did.
pub async fn run<R: BufRead, W: Write>(j: &Jira, key: &str, a: &MoveArgs, ask: &mut Asker<'_, R, W>) -> Result<Transition> {
    let ts = j.transitions(key).await?;
    let (i, values) = plan(key, &ts, a, ask)?;
    let t = ts[i].clone();
    j.transition(key, &t, &values, a.comment.as_deref()).await?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Choice, StatusCategory};

    fn ts() -> Vec<Transition> {
        let resolution = ScreenField {
            id: "resolution".into(),
            name: "Resolution".into(),
            required: true,
            has_default: false,
            kind: FieldKind::Choice,
            allowed: vec![Choice { id: "10001".into(), name: "Won't Do".into() }, Choice { id: "10401".into(), name: "Done".into() }],
        };
        let points = ScreenField {
            id: "customfield_10021".into(),
            name: "Story Points".into(),
            required: false,
            has_default: false,
            kind: FieldKind::Number,
            allowed: vec![],
        };
        vec![
            Transition { id: "11".into(), name: "To Do".into(), to: "To Do".into(), to_category: StatusCategory::ToDo, fields: vec![] },
            Transition { id: "31".into(), name: "Done".into(), to: "Done".into(), to_category: StatusCategory::Done, fields: vec![resolution, points] },
            Transition { id: "41".into(), name: "In Pull Request".into(), to: "In Pull Request".into(), to_category: StatusCategory::InProgress, fields: vec![] },
        ]
    }

    fn args(target: Option<&str>) -> MoveArgs {
        MoveArgs { key: "BCE-1".into(), target: target.map(Into::into), resolution: None, set: vec![], comment: None, json: false }
    }

    fn script() -> (Vec<u8>,) {
        (Vec::new(),)
    }

    #[test]
    fn script_with_target_and_resolution() {
        let (mut out,) = script();
        let mut ask: Asker<'_, &[u8], _> = Asker { input: None, out: &mut out };
        let mut a = args(Some("done"));
        a.resolution = Some("done".into());
        a.set = vec!["story_points=2".into()];
        let (i, v) = plan("BCE-1", &ts(), &a, &mut ask).unwrap();
        assert_eq!(i, 1);
        assert_eq!(serde_json::Value::Object(v), serde_json::json!({"resolution": {"id": "10401"}, "customfield_10021": 2.0}));
    }

    #[test]
    fn script_errors_tell_the_flag() {
        let (mut out,) = script();
        let mut ask: Asker<'_, &[u8], _> = Asker { input: None, out: &mut out };
        let e = plan("BCE-1", &ts(), &args(Some("done")), &mut ask).unwrap_err();
        assert_eq!(e.to_string(), "The Transition to Done needs Resolution.");
        assert_eq!(e.hint().unwrap(), "Use --resolution <name>. Values: Won't Do, Done.");

        let e = plan("BCE-1", &ts(), &args(None), &mut ask).unwrap_err();
        assert_eq!(e.hint().unwrap(), "Use one of: To Do, Done, In Pull Request.");

        let e = plan("BCE-1", &ts(), &args(Some("xyz")), &mut ask).unwrap_err();
        assert_eq!(e.to_string(), "No Transition of BCE-1 matches `xyz`.");

        let mut a = args(Some("pull"));
        a.set = vec!["Points=1".into()];
        let e = plan("BCE-1", &ts(), &a, &mut ask).unwrap_err();
        assert_eq!(e.hint().unwrap(), "This Transition has no fields.");
    }

    #[test]
    fn terminal_asks_for_the_target_and_the_field() {
        let mut out = Vec::new();
        let mut input = "2\n2\n".as_bytes();
        let mut ask = Asker { input: Some(&mut input), out: &mut out };
        let (i, v) = plan("BCE-1", &ts(), &args(None), &mut ask).unwrap();
        assert_eq!(i, 1);
        assert_eq!(v["resolution"], serde_json::json!({"id": "10401"}));
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Move BCE-1 to:"));
        assert!(text.contains("Resolution (required):"));
    }
}
