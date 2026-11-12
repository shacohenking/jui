use super::*;
use crate::core::model::User;

/// A fixed time zone of -07:00 (Los Angeles in October).
fn la(n: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
    FixedOffset::west_opt(7 * 3600).unwrap().from_local_datetime(&n).single()
}

fn clock() -> Clock {
    Clock { now: NaiveDateTime::parse_from_str("2026-10-20 14:37:21", "%Y-%m-%d %H:%M:%S").unwrap(), local: la }
}

fn dt(s: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z").unwrap()
}

fn worklog(id: &str, secs: u64, started: &str, comment: Option<&str>) -> Worklog {
    Worklog {
        id: id.into(),
        author: Some(User { name: "jdoe".into(), display_name: "Jane Doe".into() }),
        started: Some(dt(started)),
        updated: Some(dt("2026-10-19T12:00:00.000-0700")),
        time_spent: human_duration(secs),
        time_spent_seconds: secs,
        comment: comment.map(str::to_string),
    }
}

fn ctx() -> WorklogContext {
    WorklogContext {
        key: "BCE-1".into(),
        summary: "Fix the upload".into(),
        original_estimate: Some("1d".into()),
        remaining_estimate: Some("6h".into()),
        time_spent: Some("2h".into()),
        worklogs: vec![
            worklog("300", 3600, "2026-10-18T10:00:00.000-0700", Some("Investigation of *upload*")),
            worklog("301", 3600, "2026-10-19T10:00:00.000-0700", None),
        ],
    }
}

#[test]
fn durations() {
    assert_eq!(parse_duration("1h 30m"), Ok(5400));
    assert_eq!(parse_duration("1h30m"), Ok(5400));
    assert_eq!(parse_duration("1.5h"), Ok(5400));
    assert_eq!(parse_duration("90m"), Ok(5400));
    assert_eq!(parse_duration("90"), Ok(5400), "a number is minutes");
    assert_eq!(parse_duration("2d"), Ok(2 * 8 * 3600), "a day is 8 hours");
    assert_eq!(parse_duration("1w 1d"), Ok(6 * 8 * 3600));
    assert_eq!(parse_duration(" 1H "), Ok(3600));
    for bad in ["", "0", "0m", "1x", "h", "1 h", "abc", "-1h"] {
        assert!(parse_duration(bad).is_err(), "{bad}");
    }
}

#[test]
fn start_times_are_local() {
    let c = clock();
    let s = |t: &str| parse_started(t, &c).map(|d| jira_started(&d));
    assert_eq!(s("").unwrap(), "2026-10-20T14:37:00.000-0700", "now, with no seconds");
    assert_eq!(s("now").unwrap(), "2026-10-20T14:37:00.000-0700");
    assert_eq!(s("yesterday").unwrap(), "2026-10-19T14:37:00.000-0700");
    assert_eq!(s("10:00").unwrap(), "2026-10-20T10:00:00.000-0700");
    assert_eq!(s("yesterday 9:15").unwrap(), "2026-10-19T09:15:00.000-0700");
    assert_eq!(s("2026-10-12").unwrap(), "2026-10-12T09:00:00.000-0700");
    assert_eq!(s("2026-10-12 17:30").unwrap(), "2026-10-12T17:30:00.000-0700");
    assert_eq!(s("2026-10-12T17:30").unwrap(), "2026-10-12T17:30:00.000-0700");
    assert!(s("tomorrow").is_err());
    assert!(s("25:00").is_err());
}

#[test]
fn adjust_options_and_queries() {
    assert_eq!(Adjust::parse("", None, None, "reduce_by"), Ok(Adjust::Auto));
    assert_eq!(Adjust::parse("new", Some("90m"), None, "reduce_by"), Ok(Adjust::New("1h 30m".into())));
    assert_eq!(Adjust::parse("new", Some("0"), None, "reduce_by"), Ok(Adjust::New("0m".into())));
    assert_eq!(Adjust::parse("new", None, None, "reduce_by").unwrap_err().0, "new_estimate");
    assert_eq!(Adjust::parse("manual", None, None, "reduce_by").unwrap_err().0, "reduce_by");
    assert_eq!(Adjust::parse("x", None, None, "reduce_by").unwrap_err().0, "adjust_estimate");
    let m = Adjust::Manual("1h".into());
    assert_eq!(m.query(), vec![("adjustEstimate", "manual".to_string()), ("reduceBy", "1h".to_string())]);
    assert_eq!(m.delete_query(), vec![("adjustEstimate", "manual".to_string()), ("increaseBy", "1h".to_string())]);
    assert_eq!(Adjust::Leave.delete_query(), vec![("adjustEstimate", "leave".to_string())]);
}

#[test]
fn quick_plan_from_the_popup() {
    let q = Quick { time_spent: "1h 30m".into(), started: "10:00".into(), comment: "Review of **PR** @jdoe".into(), ..Default::default() };
    let p = quick_plan(&q, &clock()).unwrap();
    assert_eq!(p.body, json!({"timeSpentSeconds": 5400, "started": "2026-10-20T10:00:00.000-0700", "comment": "Review of *PR* [~jdoe]"}));
    assert_eq!(p.adjust, Adjust::Auto);
    assert_eq!(p.describe(), "1h 30m on 2026-10-20 10:00");
    // No comment: no `comment` key.
    let p = quick_plan(&Quick { time_spent: "15m".into(), ..Default::default() }, &clock()).unwrap();
    assert!(p.body.get("comment").is_none());
    assert_eq!(quick_plan(&Quick::default(), &clock()).unwrap_err().0, "time_spent");
    let bad = Quick { time_spent: "1h".into(), started: "soon".into(), ..Default::default() };
    assert_eq!(quick_plan(&bad, &clock()).unwrap_err().0, "started");
}

#[test]
fn edit_plan_from_flags_keeps_the_other_fields() {
    let w = worklog("300", 3600, "2026-10-18T10:00:00.000-0700", Some("Old text"));
    let p = edit_plan(&w, &Quick { time_spent: "2h".into(), ..Default::default() }, &clock()).unwrap().unwrap();
    assert_eq!(p.body, json!({"timeSpentSeconds": 7200, "started": "2026-10-18T10:00:00.000-0700", "comment": "Old text"}));
    assert!(!p.needs_diff(), "the comment did not change");
    let p = edit_plan(&w, &Quick { comment: "New text".into(), ..Default::default() }, &clock()).unwrap().unwrap();
    assert!(p.needs_diff());
    assert_eq!(edit_plan(&w, &Quick { time_spent: "1h".into(), ..Default::default() }, &clock()).unwrap(), None, "the same values");
    let manual = Quick { time_spent: "2h".into(), adjust: Some("manual".into()), reduce_by: Some("1h".into()), ..Default::default() };
    assert_eq!(edit_plan(&w, &manual, &clock()).unwrap_err().0, "adjust_estimate");
}

#[test]
fn add_template_and_submit() {
    let mut s = WorklogSession::add(&ctx(), &Quick::default(), clock());
    let text = s.text().to_string();
    assert!(text.starts_with("+++\ntime_spent = \"\""), "{text}");
    assert!(text.contains("started = \"2026-10-20 14:37\""), "{text}");
    assert!(text.contains("# Time tracking: estimate 1d, remaining 6h, spent 2h"), "{text}");
    assert!(text.contains("# The last Worklogs (2 of 2):"), "{text}");
    assert!(text.contains("2026-10-18 10:00  1h  @jdoe  Investigation of upload"), "{text}");
    assert_eq!(s.label(), "BCE-1-worklog");
    assert_eq!(s.command(), "worklog add BCE-1");
    // Saved with no time: a cancel.
    assert_eq!(s.submit(&text), WorklogStep::Cancel("Cancelled. No Worklog added.".into()));
    // A time and a comment.
    let saved = text.replacen("time_spent = \"\"", "time_spent = \"45m\"", 1).replacen("+++\n\n", "+++\nLooked at **logs**\n", 1);
    let WorklogStep::Send(p) = s.submit(&saved) else { panic!("{saved}") };
    assert_eq!(p.seconds, 2700);
    assert_eq!(p.comment, "Looked at *logs*");
    assert_eq!(jira_started(&p.started), "2026-10-20T14:37:00.000-0700");
}

#[test]
fn template_errors_reopen_and_stop() {
    let mut s = WorklogSession::add(&ctx(), &Quick { time_spent: "1h".into(), ..Default::default() }, clock());
    let bad = s.text().replacen("time_spent = \"1h\"", "time_spent = \"1 hour\"", 1).replacen("adjust_estimate = \"auto\"", "adjust_estimate = \"new\"", 1).replacen("+++\n", "+++\nsize = \"x\"\n", 1);
    assert_eq!(s.submit(&bad), WorklogStep::Reopen);
    let keys: Vec<_> = s.errors().iter().map(|e| e.key.clone().unwrap_or_default()).collect();
    assert_eq!(keys, ["size", "time_spent", "new_estimate"]);
    assert!(s.text().contains("# ERROR: time_spent:"), "{}", s.text());
    // The same text again: stop with a Draft.
    let again = s.text().to_string();
    assert!(matches!(s.submit(&again), WorklogStep::Stop(_)));
    assert!(s.draft_text().contains("1 hour"));
    // A TOML error reopens.
    let mut s = WorklogSession::add(&ctx(), &Quick::default(), clock());
    assert_eq!(s.submit("+++\ntime_spent = \n+++\n"), WorklogStep::Reopen);
}

#[test]
fn edit_template_no_change_and_change() {
    let mut s = WorklogSession::edit(&ctx(), "300", clock()).unwrap();
    let text = s.text().to_string();
    assert!(text.contains("time_spent = \"1h\""), "{text}");
    assert!(text.contains("started = \"2026-10-18 10:00\""), "{text}");
    assert!(text.contains("+++\nInvestigation of **upload**\n"), "the comment as Markdown: {text}");
    assert!(!text.contains("reduce_by"), "an edit has no manual option: {text}");
    assert!(text.contains("# The last Worklogs (1 of 1):"), "the edited Worklog is not in the list: {text}");
    assert_eq!(s.command(), "worklog edit BCE-1 300");
    assert_eq!(s.submit(&text), WorklogStep::Cancel("No changes.".into()));
    let WorklogStep::Send(p) = s.submit(&text.replacen("\"1h\"", "\"2h\"", 1)) else { panic!() };
    assert_eq!(p.id.as_deref(), Some("300"));
    assert_eq!(p.comment, "Investigation of *upload*", "an unchanged body keeps the old wiki text");
    assert!(!p.needs_diff());
    let WorklogStep::Send(p) = s.submit(&text.replacen("Investigation", "Review", 1)) else { panic!() };
    assert!(p.needs_diff());
    let manual = text.replacen("adjust_estimate = \"auto\"", "adjust_estimate = \"manual\"", 1);
    assert_eq!(s.submit(&manual), WorklogStep::Reopen);
    assert!(WorklogSession::edit(&ctx(), "999", clock()).is_err());
}

#[test]
fn rejected_maps_jira_fields_to_template_keys() {
    let mut s = WorklogSession::add(&ctx(), &Quick { time_spent: "1h".into(), ..Default::default() }, clock());
    let text = s.text().to_string();
    assert!(matches!(s.submit(&text), WorklogStep::Send(_)));
    let e = Error::Validation { fields: [("timeLogged".to_string(), "Too much.".to_string())].into(), messages: vec![] };
    assert_eq!(s.rejected(e).unwrap(), WorklogStep::Reopen);
    assert!(s.text().contains("# ERROR: time_spent: Too much."), "{}", s.text());
    assert!(s.rejected(Error::Timeout(30)).is_err());
}

#[test]
fn changed_in_jira_and_one_line() {
    let a = worklog("300", 3600, "2026-10-18T10:00:00.000-0700", Some("x"));
    let mut b = a.clone();
    assert_eq!(changed_in_jira("BCE-1", &a, &b), None);
    b.time_spent_seconds = 7200;
    assert_eq!(changed_in_jira("BCE-1", &a, &b).as_deref(), Some("Worklog 300 on BCE-1 changed in Jira while you edited it."));
    assert_eq!(one_line(&a, 40), "2026-10-18 10:00  1h  @jdoe  x");
}
