//! Unit tests for Templates. The fixtures come from real GET responses (createmeta and
//! editmeta of BCE), with fewer permitted values. No test sends a request.

use super::*;
use crate::core::api;
use crate::core::model::IssueType;

const BUG: &str = include_str!("../../../tests/fixtures/createmeta_bce_bug.json");
const TASK: &str = include_str!("../../../tests/fixtures/createmeta_bce_task.json");
const SUBTASK: &str = include_str!("../../../tests/fixtures/createmeta_bce_subtask.json");
const EDIT: &str = include_str!("../../../tests/fixtures/edit_bce_bug.json");

fn meta(text: &str, id: &str, name: &str, subtask: bool) -> CreateMeta {
    let page: api::MetaPage<api::FieldMeta> = serde_json::from_str(text).unwrap();
    let fields = page
        .values
        .into_iter()
        .map(|m| {
            let id = m.field_id.clone().unwrap();
            ScreenField::from_api(id, m)
        })
        .collect();
    CreateMeta { project: "BCE".into(), issue_type: IssueType { id: id.into(), name: name.into(), subtask }, fields }
}

fn bug() -> CreateMeta {
    meta(BUG, "10102", "Bug", false)
}

fn edit_data() -> EditData {
    let i: api::EditIssue = serde_json::from_str(EDIT).unwrap();
    i.into()
}

/// Replace one header line that starts with `start`.
fn set_line(text: &str, start: &str, line: &str) -> String {
    let mut done = false;
    let out: Vec<String> = text
        .lines()
        .map(|l| {
            if !done && l.starts_with(start) {
                done = true;
                line.to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    assert!(done, "no line starts with {start}");
    out.join("\n") + "\n"
}

fn fill_bug(text: &str) -> String {
    let t = set_line(text, "summary =", "summary = \"Scanner stops\"");
    let t = set_line(&t, "components =", "components = [\"aspm-app\"]");
    let t = set_line(&t, "severity =", "severity = \"maj\"");
    let t = set_line(&t, "detected_in_production =", "detected_in_production = \"No\"");
    let t = set_line(&t, "regression =", "regression = \"yes\"");
    set_line(&t, "affects_versions =", "affects_versions = [\"unified-pc-february\"]")
}

// ------------------------------------------------------------------ keys and the create Template

#[test]
fn keys_use_fixed_names_and_snake_case() {
    let m = bug();
    let k = key_map(&m.fields);
    assert_eq!(k["customfield_11908"], "severity");
    assert_eq!(k["customfield_26910"], "detected_in_production");
    assert_eq!(k["customfield_26903"], "regression");
    assert_eq!(k["versions"], "affects_versions");
    assert_eq!(k["customfield_10016"], "epic");
    assert_eq!(k["customfield_10021"], "story_points");
    assert_eq!(k["customfield_19678"], "cortex_pm");
}

#[test]
fn two_fields_with_the_same_key_use_their_ids() {
    let f = |id: &str, name: &str| ScreenField {
        id: id.into(),
        name: name.into(),
        required: false,
        has_default: false,
        kind: FieldKind::Text,
        allowed: vec![],
    };
    let k = keys(&[f("customfield_1", "Team"), f("customfield_2", "Team?"), f("summary", "Summary")]);
    assert_eq!(k, ["customfield_1", "customfield_2", "summary"]);
}

#[test]
fn bug_template_has_the_four_extra_required_fields() {
    let t = create_template(&bug(), &Preset::default());
    let header = t.split(SCISSORS).next().unwrap();
    let required = header.split("# --- Required ---").nth(1).unwrap().split("# --- Optional").next().unwrap();
    for line in [
        "summary = \"\"",
        "components = []",
        "severity = \"\"",
        "detected_in_production = \"\"",
        "regression = \"\"",
        "affects_versions = []",
    ] {
        assert!(required.contains(line), "{line} is not in the Required group:\n{required}");
    }
    assert!(required.contains("# customfield_11908"), "the field ID is a comment");
    assert!(required.contains("# Permitted: Critical, Major, Moderate, Minor, Normal"));
    assert!(required.contains("(12 values, see the list below the scissors line)"), "12 components > 10");
    assert!(!required.contains("reporter"), "jui sets the reporter");
    assert!(header.contains("project = \"BCE\"\ntype = \"Bug\""));
    assert!(header.contains("# assignee = \"me\""));
    assert!(header.contains("# priority = \"\""));
    assert!(!header.contains("archived"));
    let reference = t.split(SCISSORS).nth(1).unwrap();
    assert!(reference.contains("# components (all 12 values):"));
    assert!(reference.contains("steps_to_reproduce = ..."), "other fields are in the reference part");
    assert!(!reference.contains("sprint = ..."), "jui cannot set the Sprint field");
}

#[test]
fn bug_template_snapshot() {
    insta::assert_snapshot!(create_template(&bug(), &Preset::default()));
}

#[test]
fn subtask_needs_parent_and_flags_fill_the_template() {
    let m = meta(SUBTASK, "10101", "Sub-task", true);
    let mut p = Preset::default();
    p.values.push(("parent".into(), FlagValue::Text("62886".into())));
    p.values.push(("summary".into(), FlagValue::Text("Child".into())));
    p.values.push(("components".into(), FlagValue::List(vec!["DCO-Flow".into()])));
    let t = create_template(&m, &p);
    assert!(t.contains("parent = \"62886\""));
    assert!(t.contains("summary = \"Child\""));
    let mut s = Session::create(m, "shacohen", &p);
    let Step::Send(Request::Create(body)) = s.submit_given(&t) else { panic!("{:?}", s.errors()) };
    assert_eq!(body["fields"]["parent"], json!({"key": "BCE-62886"}));
    assert_eq!(body["fields"]["issuetype"], json!({"id": "10101"}));
    assert_eq!(body["fields"]["reporter"], json!({"name": "shacohen"}));
    assert!(body["fields"].get("description").is_none(), "the placeholder is not sent");
}

// ------------------------------------------------------------------ create checks

#[test]
fn a_filled_bug_gives_the_create_body() {
    let mut s = Session::create(bug(), "shacohen", &Preset::default());
    let text = fill_bug(s.text()).replace(PLACEHOLDER, "The scanner **stops**. cc @jdoe");
    let text = set_line(&text, "# assignee =", "assignee = \"me\"");
    let Step::Send(Request::Create(body)) = s.submit(&text) else { panic!("{:?}", s.errors()) };
    let f = &body["fields"];
    assert_eq!(f["project"], json!({"key": "BCE"}));
    assert_eq!(f["issuetype"], json!({"id": "10102"}));
    assert_eq!(f["summary"], json!("Scanner stops"));
    assert_eq!(f["customfield_11908"], json!({"id": "44420"}), "a unique prefix, any case");
    assert_eq!(f["customfield_26903"]["id"], json!("44423"));
    assert_eq!(f["assignee"], json!({"name": "shacohen"}), "me is the current user");
    assert_eq!(f["description"], json!("The scanner *stops*. cc [~jdoe]"));
    assert!(f["components"].as_array().unwrap().len() == 1);
    assert_eq!(f["versions"].as_array().unwrap().len(), 1);
}

#[test]
fn unchanged_or_empty_create_cancels() {
    let mut s = Session::create(bug(), "me1", &Preset::default());
    let t = s.text().to_string();
    assert_eq!(s.submit(&t), Step::Cancel("Cancelled. No Ticket created.".into()));
    // Only spaces at the ends of lines: still no change.
    assert_eq!(s.submit(&t.replace('\n', "  \n")), Step::Cancel("Cancelled. No Ticket created.".into()));
    assert_eq!(s.submit(""), Step::Cancel("Cancelled. No Ticket created.".into()));
}

#[test]
fn a_template_that_is_not_valid_opens_again_with_error_lines() {
    let mut s = Session::create(bug(), "me1", &Preset::default());
    let text = fill_bug(s.text());
    let text = set_line(&text, "severity =", "severity = \"S5\"");
    let text = set_line(&text, "regression =", "regression = \"\"");
    let text = text.replace("summary = \"Scanner stops\"", "summary = \"Scanner stops\"\nsevrity = \"x\"");
    assert_eq!(s.submit(&text), Step::Reopen);
    let shown = s.text().to_string();
    let errs: Vec<&str> = shown.lines().filter(|l| l.starts_with(ERROR_PREFIX)).collect();
    assert!(shown.starts_with("+++\n# ERROR:"), "the errors are at the top of the header");
    assert!(errs.contains(&"# ERROR: severity: \"S5\" is not a permitted value. Permitted: Critical, Major, Moderate, Minor, Normal"), "{errs:?}");
    assert!(errs.contains(&"# ERROR: regression: a value is required."), "{errs:?}");
    assert!(errs.iter().any(|e| e.starts_with("# ERROR: sevrity: is not a field")), "{errs:?}");

    // The user corrects one error: the old error lines go, and the new check has fewer errors.
    let fixed = set_line(&shown, "severity =", "severity = \"Minor\"");
    assert_eq!(s.submit(&fixed), Step::Reopen);
    assert_eq!(s.text().lines().filter(|l| l.starts_with(ERROR_PREFIX)).count(), 2);

    // The user saves with no change: jui stops, and the Draft has the text of the user.
    let again = s.text().to_string();
    assert!(matches!(s.submit(&again), Step::Stop(_)));
    assert!(s.draft_text().contains("severity = \"Minor\""));
}

#[test]
fn toml_syntax_error_gives_the_line() {
    let mut s = Session::create(bug(), "me1", &Preset::default());
    let text = set_line(s.text(), "summary =", "summary = \"no end");
    assert_eq!(s.submit(&text), Step::Reopen);
    assert!(s.errors()[0].message.starts_with("TOML syntax error in the header, line "), "{:?}", s.errors());
}

#[test]
fn jira_validation_errors_use_the_template_keys() {
    let mut s = Session::create(bug(), "me1", &Preset::default());
    let text = fill_bug(s.text());
    assert!(matches!(s.submit(&text), Step::Send(_)));
    let mut fields = BTreeMap::new();
    fields.insert("customfield_26910".to_string(), "Detected in Production? is required.".to_string());
    let step = s.rejected(Error::Validation { fields, messages: vec!["Bad.".into()] }).unwrap();
    assert_eq!(step, Step::Reopen);
    assert!(s.text().contains("# ERROR: detected_in_production: Detected in Production? is required."));
    assert!(s.text().contains("# ERROR: Bad."));
    assert!(s.text().contains("summary = \"Scanner stops\""), "the text of the user stays");
    assert!(s.rejected(Error::Timeout(3)).is_err(), "other errors come back");
}

#[test]
fn project_and_type_cannot_change() {
    let mut s = Session::create(meta(TASK, "10100", "Task", false), "me1", &Preset::default());
    let text = set_line(s.text(), "type =", "type = \"Bug\"");
    assert_eq!(s.submit(&text), Step::Reopen);
    assert!(s.errors().iter().any(|e| e.key.as_deref() == Some("type")));
}

#[test]
fn values_by_type() {
    let m = meta(TASK, "10100", "Task", false);
    let k = keys(&m.fields);
    let f = |id: &str| m.fields.iter().find(|f| f.id == id).unwrap();
    let conv = |id: &str, v: toml::Value| to_json(f(id), &v, "me1", "BCE");
    let s = |x: &str| toml::Value::String(x.into());
    assert_eq!(conv("customfield_10021", toml::Value::Integer(3)), Ok(Conv::Json(json!(3))));
    assert!(conv("customfield_10021", s("x")).is_err());
    assert_eq!(conv("customfield_10016", s("47921")), Ok(Conv::Json(json!("BCE-47921"))));
    assert_eq!(conv("assignee", s("none")), Ok(Conv::Empty));
    assert!(conv("labels", toml::Value::Array(vec![s("a b")])).is_err());
    assert!(conv("customfield_10020", s("x")).is_err(), "Sprint is not supported");
    assert!(k.contains(&"epic".to_string()));
}

// ------------------------------------------------------------------ edit

#[test]
fn edit_template_shows_current_values() {
    let s = Session::edit(edit_data(), "me1", false, &Preset::default());
    let t = s.text();
    assert!(t.contains("summary = \"Scanner stops on an empty file\""));
    assert!(t.contains("severity = \"Major\""));
    assert!(t.contains("components = [\"ASPM-Application\"]"));
    assert!(t.contains("labels = [\"scanner\"]"));
    assert!(t.contains("fix_versions = [\"v0.1.0-old\"]"), "an archived current value stays visible");
    assert!(t.contains("# project = \"BCE\""));
    assert!(t.contains("`jira:{color:#de350b}Failed{color}`"), "the body is Markdown with Raw wiki pieces");
    insta::assert_snapshot!("edit_template", t);
}

#[test]
fn edit_with_no_change_sends_nothing() {
    let mut s = Session::edit(edit_data(), "me1", false, &Preset::default());
    let t = s.text().to_string();
    assert_eq!(s.submit(&t), Step::Cancel("No changes.".into()));
    // The reference part and spaces at the ends of lines do not count.
    let t2 = t.replace("jui ignores this line", "changed help") + "\n\n";
    assert_eq!(s.submit(&t2.replace('\n', " \n")), Step::Cancel("No changes.".into()));
}

#[test]
fn edit_sends_only_the_changed_fields() {
    let mut s = Session::edit(edit_data(), "me1", false, &Preset::default());
    let t = set_line(s.text(), "severity =", "severity = \"crit\"");
    let t = set_line(&t, "labels =", "labels = []");
    let Step::Send(Request::Edit(p)) = s.submit(&t) else { panic!("{:?}", s.errors()) };
    assert_eq!(p.changed, ["labels", "severity"]);
    assert_eq!(p.body(), json!({"fields": {"customfield_11908": {"id": "44419"}, "labels": []}}));
    assert!(p.description.is_none(), "the description did not change");
}

#[test]
fn an_archived_value_that_stays_is_not_an_error() {
    let mut s = Session::edit(edit_data(), "me1", false, &Preset::default());
    let t = set_line(s.text(), "summary =", "summary = \"New\"");
    let Step::Send(Request::Edit(p)) = s.submit(&t) else { panic!("{:?}", s.errors()) };
    assert_eq!(p.changed, ["summary"]);
}

#[test]
fn edit_of_the_description_gives_a_diff() {
    let mut s = Session::edit(edit_data(), "me1", false, &Preset::default());
    let t = s.text().replace("See the error.", "See the error message.");
    let Step::Send(Request::Edit(p)) = s.submit(&t) else { panic!("{:?}", s.errors()) };
    let d = p.description.unwrap();
    assert!(d.needs_diff());
    assert!(d.new.contains("{color:#de350b}Failed{color}"), "the Raw wiki piece goes back unchanged");
    let lines = diff(&d.old, &d.new);
    assert!(lines.contains(&DiffLine { tag: '-', text: "# See the error.".into() }), "{lines:?}");
    assert!(lines.contains(&DiffLine { tag: '+', text: "# See the error message.".into() }), "{lines:?}");
}

#[test]
fn raw_wiki_edit_is_not_converted() {
    let mut s = Session::edit(edit_data(), "me1", true, &Preset::default());
    assert!(s.text().contains("h3. Steps"));
    let t = s.text().replace("h3. Steps", "h2. Steps");
    let Step::Send(Request::Edit(p)) = s.submit(&t) else { panic!() };
    assert!(p.description.unwrap().new.starts_with("h2. Steps\n# Run"));
}

#[test]
fn edit_rejects_project_and_required_empty() {
    let mut s = Session::edit(edit_data(), "me1", false, &Preset::default());
    let t = s.text().replace("# project = ", "project = ").replace("   (jui cannot change the project or the type)", "");
    let t = set_line(&t, "summary =", "summary = \"\"");
    assert_eq!(s.submit(&t), Step::Reopen);
    let keys: Vec<_> = s.errors().iter().filter_map(|e| e.key.clone()).collect();
    assert!(keys.contains(&"project".to_string()) && keys.contains(&"summary".to_string()), "{keys:?}");
}

#[test]
fn preset_sets_values_in_the_edit_template() {
    let mut p = Preset::default();
    p.set("severity=Minor").unwrap();
    p.set("story_points=5").unwrap();
    assert!(p.set("nothing").is_err());
    let mut s = Session::edit(edit_data(), "me1", false, &p);
    let t = s.text().to_string();
    assert!(t.contains("severity = \"Minor\""));
    assert!(t.contains("story_points = 5"));
    let Step::Send(Request::Edit(plan)) = s.submit_given(&t) else { panic!("{:?}", s.errors()) };
    assert_eq!(plan.changed, ["severity", "story_points"]);
}

#[test]
fn changes_in_jira_are_found() {
    let a = edit_data();
    let mut b = a.clone();
    b.summary = "Other".into();
    b.values.insert("customfield_11908".into(), json!({"value": "Minor"}));
    assert_eq!(changed_in_jira(&a, &b), ["Summary", "Severity"]);
    assert!(changed_in_jira(&a, &a).is_empty());
}

// ------------------------------------------------------------------ parser and Drafts

#[test]
fn parser_handles_no_header_and_scissors() {
    let p = parse("Only text.\n\n# ------------------------ >8 ------------------------\nignored").unwrap();
    assert!(p.header.is_empty());
    assert_eq!(p.body, "Only text.");
    let p = parse("+++\n# ERROR: old\na = 1\n+++\n\nBody\n").unwrap();
    assert_eq!(p.header["a"], toml::Value::Integer(1));
    assert_eq!(p.body, "Body");
    assert!(parse("+++\na = 1\nBody").is_err());
}

#[test]
fn drafts_are_written_and_old_drafts_are_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.md");
    std::fs::write(&old, "x").unwrap();
    let f = std::fs::File::options().write(true).open(&old).unwrap();
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(31 * 24 * 3600)).unwrap();
    let p1 = write_draft(dir.path(), "BCE-1", "text").unwrap();
    let p2 = write_draft(dir.path(), "BCE-1", "text 2").unwrap();
    assert_ne!(p1, p2);
    assert!(!old.exists(), "a Draft of 31 days is deleted");
    assert_eq!(std::fs::read_to_string(&p2).unwrap(), "text 2");
    assert!(p1.file_name().unwrap().to_str().unwrap().ends_with("-BCE-1.md"));
}

#[test]
fn a_draft_from_a_file_is_checked_again() {
    // `--from`: an unchanged create Template is not a cancel, it is checked.
    let mut s = Session::create(bug(), "me1", &Preset::default());
    let t = s.text().to_string();
    assert_eq!(s.submit_given(&t), Step::Reopen);
    let filled = fill_bug(&t);
    assert!(matches!(s.submit_given(&filled), Step::Send(_)));
}

#[test]
fn diff_ignores_crlf() {
    let d = diff("a\r\nb\r\nc", "a\nB\nc");
    let changed: Vec<String> = d.iter().filter(|l| l.tag != ' ').map(|l| format!("{}{}", l.tag, l.text)).collect();
    assert_eq!(changed, ["-b", "+B"]);
}

// ------------------------------------------------------------------ create form

fn preset(values: &[(&str, &str)], description: Option<&str>) -> Preset {
    Preset {
        values: values.iter().map(|(k, v)| (k.to_string(), FlagValue::Text(v.to_string()))).collect(),
        description: description.map(str::to_string),
    }
}

#[test]
fn form_fields_are_the_summary_the_required_and_the_frequent_fields() {
    let m = bug();
    let ids: Vec<&str> = form_fields(&m, &Preset::default()).iter().map(|&i| m.fields[i].id.as_str()).collect();
    assert_eq!(ids[0], "summary");
    for id in ["components", "versions", "assignee", "priority", "labels", "fixVersions"] {
        assert!(ids.contains(&id), "{id} is in the form: {ids:?}");
    }
    for id in ["reporter", "description", "project", "issuetype", "attachment", "issuelinks"] {
        assert!(!ids.contains(&id), "{id} is not in the form: {ids:?}");
    }
    let sub = meta(SUBTASK, "10101", "Sub-task", true);
    let ids: Vec<&str> = form_fields(&sub, &Preset::default()).iter().map(|&i| sub.fields[i].id.as_str()).collect();
    assert!(ids.contains(&"parent"), "the parent of a Sub-task is required: {ids:?}");
}

#[test]
fn submit_form_gives_the_create_body_or_the_errors_with_keys() {
    let mut s = Session::create(bug(), "shacohen", &Preset::default());
    let errs = s.submit_form(&preset(&[("summary", "Scanner stops")], None)).unwrap_err();
    let keys: Vec<&str> = errs.iter().filter_map(|e| e.key.as_deref()).collect();
    assert!(keys.contains(&"components") && keys.contains(&"severity"), "{keys:?}");
    assert!(!keys.contains(&"summary"));

    let p = preset(
        &[
            ("summary", "Scanner stops"),
            ("components", "aspm-app"),
            ("severity", "maj"),
            ("detected_in_production", "No"),
            ("regression", "yes"),
            ("affects_versions", "unified-pc-february"),
            ("labels", "a, b"),
            ("assignee", "me"),
        ],
        Some("It **stops**."),
    );
    let body = s.submit_form(&p).unwrap();
    let f = &body["fields"];
    assert_eq!(f["summary"], "Scanner stops");
    assert_eq!(f["labels"], json!(["a", "b"]));
    assert_eq!(f["assignee"], json!({ "name": "shacohen" }));
    assert_eq!(f["reporter"], json!({ "name": "shacohen" }));
    assert_eq!(f["description"], "It *stops*.");
    assert!(s.errors().is_empty());
    // A Draft of the form has the values in a Template.
    assert!(s.draft_text().contains("summary = \"Scanner stops\""));
}

#[test]
fn use_form_writes_the_template_for_the_editor() {
    let mut s = Session::create(bug(), "shacohen", &Preset::default());
    s.use_form(&preset(&[("summary", "From the form")], Some("Body text")));
    assert!(s.text().contains("summary = \"From the form\""));
    assert!(s.text().contains("Body text"));
    // The editor closes with no change: a cancel.
    let text = s.text().to_string();
    assert!(matches!(s.submit(&text), Step::Cancel(_)));
}
