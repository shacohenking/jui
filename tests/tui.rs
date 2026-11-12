//! TUI tests: unit tests for the pure `update` function, and snapshots of the screen
//! with the ratatui `TestBackend`. They make no HTTP requests.

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jui::core::api;
use jui::core::model::{ListRow, Page, Ticket, TicketSummary};
use jui::core::settings::Settings;
use jui::core::{Error, model};
use jui::tui::state::{CACHE_SIZE, MY_TICKETS, Popup, State, TemplateKind};
use jui::core::model::Transition;
use jui::tui::update::{Action, Event, init, update};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const ISSUE: &str = include_str!("fixtures/issue_full.json");
const AGILE: &str = include_str!("fixtures/agile_issue.json");

/// The test Settings. The grouping by Sprint is off, so that the tests of other functions
/// see the rows in the order of the query. The tests in "My Tickets by Sprint" turn it on.
fn settings() -> Settings {
    Settings { base_url: "https://jira.example".into(), group_by_sprint: false, ..Settings::default() }
}

fn state(w: u16, h: u16) -> State {
    let mut st = State::new(&settings(), w, h);
    st.now = Some("2026-10-19T12:00:00Z".parse::<DateTime<Utc>>().unwrap());
    st
}

fn row(n: usize) -> ListRow {
    ListRow {
        ticket: TicketSummary {
            key: format!("BCE-{n}"),
            summary: format!("Summary of Ticket {n}"),
            kind: "Task".into(),
            status: if n.is_multiple_of(3) { "In Progress".into() } else { "To Do".into() },
            status_category: if n.is_multiple_of(3) { model::StatusCategory::InProgress } else { model::StatusCategory::ToDo },
            priority: Some("Major".into()),
            assignee: None,
            updated: model::parse_jira_date("2026-10-18T12:00:00.000+0000"),
            sprint: None,
        },
        description: Some(format!("h3. Goal {n}\nText of *Ticket* {n}.")),
    }
}

fn page(start: usize, count: usize, total: u64) -> Result<Page<ListRow>, Error> {
    Ok(Page { items: (start..start + count).map(row).collect(), start_at: start as u64, total })
}

fn full_ticket() -> Ticket {
    let issue: api::FullIssue = serde_json::from_str(ISSUE).unwrap();
    let agile: api::AgileIssue = serde_json::from_str(AGILE).unwrap();
    Ticket::from_api(issue, Some(agile), "https://jira.example/browse/BCE-62886".into())
}

fn ticket_with_key(key: &str) -> Ticket {
    let mut t = full_ticket();
    t.key = key.into();
    t
}

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn code(c: KeyCode) -> Event {
    Event::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

/// Start the TUI state and give it the first page.
fn loaded(rows: usize, total: u64) -> State {
    let mut st = state(120, 30);
    init(&mut st);
    update(&mut st, Event::Page { tab: MY_TICKETS, generation: 1, start_at: 0, result: page(0, rows, total) });
    st
}

fn list(st: &State) -> &jui::tui::state::TicketList {
    st.tabs[MY_TICKETS].list.as_ref().unwrap()
}

fn draw(st: &mut State, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| jui::tui::view::view(f, st)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

// ------------------------------------------------------------------ update

#[test]
fn init_gets_the_first_page_of_my_tickets() {
    let mut st = state(120, 30);
    let a = init(&mut st);
    assert_eq!(
        a,
        vec![Action::FetchPage { tab: MY_TICKETS, jql: settings().my_tickets_jql, start_at: 0, generation: 1 }]
    );
    assert!(list(&st).loading);
}

#[test]
fn first_page_gets_the_first_ticket_at_once() {
    let mut st = state(120, 30);
    init(&mut st);
    let a = update(&mut st, Event::Page { tab: MY_TICKETS, generation: 1, start_at: 0, result: page(0, 5, 5) });
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }]);
    assert_eq!(list(&st).count().as_deref(), Some("5"));
}

#[test]
fn moving_waits_for_the_preview_delay() {
    let mut st = loaded(5, 5);
    let a = update(&mut st, key('j'));
    assert_eq!(a, vec![Action::SchedulePreview { key: "BCE-1".into() }]);
    // The user moved again before the delay: the old key is not fetched.
    update(&mut st, key('j'));
    assert!(update(&mut st, Event::PreviewDue { key: "BCE-1".into() }).is_empty());
    assert_eq!(update(&mut st, Event::PreviewDue { key: "BCE-2".into() }), vec![Action::FetchTicket { key: "BCE-2".into() }]);
    // A second request for the same key is not sent while one runs.
    assert!(update(&mut st, Event::PreviewDue { key: "BCE-2".into() }).is_empty());
}

#[test]
fn infinite_scroll_gets_one_page_at_a_time() {
    let mut st = loaded(100, 495);
    assert_eq!(list(&st).count().as_deref(), Some("100 of 495"));
    // 79 rows from the end: no page yet.
    for _ in 0..79 {
        let a = update(&mut st, key('j'));
        assert!(!a.iter().any(|a| matches!(a, Action::FetchPage { .. })));
    }
    // Row 80 is 19 rows from the end: get the next page.
    let a = update(&mut st, key('j'));
    assert!(a.contains(&Action::FetchPage { tab: 0, jql: settings().my_tickets_jql, start_at: 100, generation: 1 }));
    // No second request while the first one runs.
    let a = update(&mut st, key('j'));
    assert!(!a.iter().any(|a| matches!(a, Action::FetchPage { .. })));
    update(&mut st, Event::Page { tab: 0, generation: 1, start_at: 100, result: page(100, 100, 495) });
    assert_eq!(list(&st).rows.len(), 200);
    assert_eq!(list(&st).count().as_deref(), Some("200 of 495"));
}

#[test]
fn last_row_key_gets_the_next_page_only() {
    let mut st = loaded(100, 495);
    let a = update(&mut st, key('G'));
    assert_eq!(list(&st).selected, 99);
    let pages: Vec<_> = a.iter().filter(|a| matches!(a, Action::FetchPage { .. })).collect();
    assert_eq!(pages.len(), 1);
}

#[test]
fn all_rows_loaded_shows_only_the_total() {
    let mut st = loaded(100, 120);
    update(&mut st, key('G'));
    update(&mut st, Event::Page { tab: 0, generation: 1, start_at: 100, result: page(100, 20, 120) });
    assert_eq!(list(&st).count().as_deref(), Some("120"));
    assert!(!list(&st).has_more());
}

#[test]
fn refresh_keeps_old_rows_dim_and_ignores_late_responses() {
    let mut st = loaded(10, 10);
    update(&mut st, key('j'));
    update(&mut st, key('j'));
    let a = update(&mut st, key('r'));
    assert!(a.contains(&Action::FetchPage { tab: 0, jql: settings().my_tickets_jql, start_at: 0, generation: 2 }));
    assert!(list(&st).stale);
    assert_eq!(list(&st).rows.len(), 10, "the old rows stay until the new page arrives");

    // A late response of generation 1 is ignored.
    update(&mut st, Event::Page { tab: 0, generation: 1, start_at: 0, result: page(50, 3, 3) });
    assert_eq!(list(&st).rows[0].ticket.key, "BCE-0");

    // The new page: the selection stays on BCE-2.
    let mut items: Vec<ListRow> = (0..10).map(row).collect();
    items.swap(0, 2);
    update(&mut st, Event::Page { tab: 0, generation: 2, start_at: 0, result: Ok(Page { items, start_at: 0, total: 10 }) });
    assert!(!list(&st).stale);
    assert_eq!(list(&st).selected_key(), Some("BCE-2"));
}

#[test]
fn a_tab_keeps_its_state() {
    let mut st = loaded(10, 10);
    update(&mut st, key('j'));
    update(&mut st, key('5'));
    assert_eq!(st.tab, SEARCH);
    let a = update(&mut st, key('1'));
    assert!(a.is_empty(), "no new request when the user comes back");
    assert_eq!(list(&st).selected, 1);
}

#[test]
fn zoom_and_breadcrumb_with_subtask_and_link() {
    let mut st = loaded(5, 5);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    let a = update(&mut st, code(KeyCode::Enter));
    assert!(a.is_empty(), "the Ticket is in the cache");
    assert_eq!(st.tabs[0].zoom.len(), 1);

    // Item 0 is the Sub-task, items 1 and 2 are the Links, item 3 is the attachment, item 4 is the
    // Worklog, items 5 and 6 are the comments.
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-62890".into() }]);
    assert_eq!(st.tabs[0].zoom.iter().map(|z| z.key.as_str()).collect::<Vec<_>>(), ["BCE-0", "BCE-62890"]);

    update(&mut st, code(KeyCode::Esc));
    update(&mut st, key('j'));
    update(&mut st, key('j'));
    assert_eq!(st.tabs[0].zoom[0].item, 2);
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-61200".into() }]);
    update(&mut st, code(KeyCode::Esc));
    for _ in 0..7 {
        update(&mut st, key('j'));
    }
    assert_eq!(st.tabs[0].zoom[0].item, 6, "stays on the last item (the last comment)");
    let a = update(&mut st, code(KeyCode::Enter));
    assert!(a.is_empty(), "Enter on a comment opens nothing");
    assert!(st.status.as_ref().unwrap().text.contains("e edits the comment"));

    // q in the zoom goes back; it does not quit.
    update(&mut st, key('q'));
    assert!(st.tabs[0].zoom.is_empty());
    assert!(!st.quit);
    let a = update(&mut st, key('q'));
    assert_eq!(a, vec![Action::Quit]);
}

#[test]
fn zoom_keys_work_only_in_the_zoom() {
    let mut st = loaded(5, 5);
    update(&mut st, key('l'));
    update(&mut st, key('u'));
    assert!(st.status.is_none() && st.popup.is_none(), "l and u are zoom keys; they do nothing in the list");
    assert!(jui::tui::keys::KEYS.iter().all(|b| b.built), "all keys are built");
}

#[test]
fn open_in_browser_uses_the_selected_key() {
    let mut st = loaded(5, 5);
    update(&mut st, key('j'));
    let a = update(&mut st, key('o'));
    assert_eq!(a, vec![Action::OpenBrowser { url: "https://jira.example/browse/BCE-1".into() }]);
}

#[test]
fn errors_go_to_the_status_line_and_the_error_popup() {
    let mut st = loaded(5, 5);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Err(Error::NotFound("Ticket BCE-0".into()))) });
    assert!(st.status.as_ref().unwrap().text.starts_with("Ticket BCE-0 not found."));
    assert_eq!(st.errors[0].context, "Get Ticket BCE-0");
    update(&mut st, key('!'));
    assert_eq!(st.popup, Some(Popup::Errors { scroll: 0 }));
    update(&mut st, key('x'));
    assert_eq!(st.popup, None);
}

#[test]
fn network_error_marks_the_list_offline() {
    let mut st = loaded(5, 5);
    update(&mut st, key('r'));
    let e = Error::Network { host: "jira.example".into(), cause: "refused".into() };
    update(&mut st, Event::Page { tab: 0, generation: 2, start_at: 0, result: Err(e) });
    assert!(list(&st).offline);
    assert!(!list(&st).stale);
    assert_eq!(list(&st).rows.len(), 5, "the previous data stays");
    update(&mut st, key('r'));
    update(&mut st, Event::Page { tab: 0, generation: 3, start_at: 0, result: page(0, 5, 5) });
    assert!(!list(&st).offline);
}

#[test]
fn unauthorized_shows_the_fatal_popup() {
    let mut st = state(120, 30);
    init(&mut st);
    update(&mut st, Event::Page { tab: 0, generation: 1, start_at: 0, result: Err(Error::Unauthorized) });
    assert!(matches!(st.popup, Some(Popup::Fatal { .. })));
    assert!(update(&mut st, key('j')).is_empty());
    assert_eq!(update(&mut st, key('q')), vec![Action::Quit]);
}

#[test]
fn cancelled_requests_show_no_message() {
    let mut st = loaded(5, 5);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Err(Error::Cancelled)) });
    assert!(st.status.is_none());
    assert!(st.errors.is_empty());
}

#[test]
fn status_messages_end_after_8_seconds() {
    let mut st = loaded(5, 5);
    update(&mut st, key('!'));
    assert_eq!(st.status.as_ref().unwrap().text, "No errors.");
    for _ in 0..31 {
        update(&mut st, Event::Tick);
    }
    assert!(st.status.is_some());
    update(&mut st, Event::Tick);
    assert!(st.status.is_none());
}

#[test]
fn cache_keeps_the_last_50_tickets() {
    let mut st = loaded(5, 5);
    for i in 0..CACHE_SIZE + 5 {
        let k = format!("X-{i}");
        update(&mut st, Event::Ticket { key: k.clone(), result: Box::new(Ok(ticket_with_key(&k))) });
    }
    assert_eq!(st.cache.len(), CACHE_SIZE);
    assert!(!st.cache.contains("X-0"));
    assert!(st.cache.contains(&format!("X-{}", CACHE_SIZE + 4)));
}

// ------------------------------------------------------------------ snapshots

#[test]
fn snapshot_my_tickets_with_preview() {
    let mut st = loaded(6, 6);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    insta::assert_snapshot!(draw(&mut st, 120, 30));
}

#[test]
fn snapshot_preview_before_the_full_ticket() {
    let mut st = loaded(6, 6);
    insta::assert_snapshot!(draw(&mut st, 120, 16));
}

#[test]
fn snapshot_wide_list_with_more_pages() {
    let mut st = loaded(100, 495);
    insta::assert_snapshot!(draw(&mut st, 170, 10));
}

#[test]
fn snapshot_narrow_hides_the_preview() {
    let mut st = loaded(6, 6);
    insta::assert_snapshot!(draw(&mut st, 80, 10));
}

#[test]
fn snapshot_zoom_with_breadcrumb() {
    let mut st = loaded(6, 6);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, key('j'));
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Ticket { key: "BCE-63001".into(), result: Box::new(Ok(ticket_with_key("BCE-63001"))) });
    insta::assert_snapshot!(draw(&mut st, 100, 50));
}

#[test]
fn snapshot_help_popup() {
    let mut st = loaded(6, 6);
    update(&mut st, key('?'));
    insta::assert_snapshot!(draw(&mut st, 120, 60));
}

#[test]
fn snapshot_settings_tab_small() {
    let mut st = loaded(6, 6);
    update(&mut st, key('6'));
    insta::assert_snapshot!(draw(&mut st, 100, 8));
}

#[test]
fn snapshot_no_token_popup() {
    let mut st = state(100, 14);
    jui::tui::update::fatal(&mut st, &Error::NoToken);
    insta::assert_snapshot!(draw(&mut st, 100, 14));
}

// ------------------------------------------------------------------ Transitions

const TR_TASK: &str = include_str!("fixtures/transitions_task.json");

fn transitions() -> Vec<Transition> {
    let r: api::TransitionsResponse = serde_json::from_str(TR_TASK).unwrap();
    r.transitions.into_iter().map(Into::into).collect()
}

fn typed(st: &mut State, text: &str) {
    for c in text.chars() {
        update(st, key(c));
    }
}

fn popup_open_with_list(st: &mut State) {
    let a = update(st, key('t'));
    assert_eq!(a, vec![Action::FetchTransitions { key: "BCE-0".into() }]);
    update(st, Event::Transitions { key: "BCE-0".into(), result: Ok(transitions()) });
}

#[test]
fn transition_popup_filters_and_moves() {
    let mut st = loaded(5, 5);
    popup_open_with_list(&mut st);
    typed(&mut st, "pull");
    let a = update(&mut st, code(KeyCode::Enter));
    let [Action::DoTransition { key, transition, fields }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(key, "BCE-0");
    assert_eq!(transition.id, "41");
    assert!(fields.is_empty());
    // Letters go to the filter, not to the key table: 'q' does not quit.
    assert!(!st.quit);

    // Success: the popup closes, the status line tells it, and jui gets the Ticket again.
    let a = update(&mut st, Event::Moved { key: "BCE-0".into(), to: "In Pull Request".into(), result: Ok(()) });
    assert_eq!(st.popup, None);
    assert_eq!(st.status.as_ref().unwrap().text, "BCE-0 moved to In Pull Request.");
    assert!(a.contains(&Action::FetchTicket { key: "BCE-0".into() }));
    assert!(a.iter().any(|a| matches!(a, Action::CheckMatch { tab: 0, key, jql } if key == "BCE-0" && jql.starts_with("key = BCE-0 AND ("))));
}

#[test]
fn transition_with_required_field_opens_the_fields_popup() {
    let mut st = loaded(5, 5);
    popup_open_with_list(&mut st);
    typed(&mut st, "done");
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    let Some(Popup::Fields(p)) = &st.popup else { panic!("no fields popup") };
    assert_eq!(p.fields.len(), 1, "only the required field");
    assert_eq!(p.fields[0].name, "Resolution");

    // Filter the values, then accept.
    typed(&mut st, "dup");
    let a = update(&mut st, code(KeyCode::Enter));
    let [Action::DoTransition { transition, fields, .. }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(transition.id, "31");
    assert_eq!(fields["resolution"], serde_json::json!({"id": "10002"}));
}

#[test]
fn validation_error_keeps_the_popup_and_the_values() {
    let mut st = loaded(5, 5);
    popup_open_with_list(&mut st);
    typed(&mut st, "done");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, code(KeyCode::Enter)); // the first value: Won't Do
    let mut errs = std::collections::BTreeMap::new();
    errs.insert("Resolution".to_string(), "Resolution is not valid.".to_string());
    update(&mut st, Event::Moved { key: "BCE-0".into(), to: "Done".into(), result: Err(Error::Validation { fields: errs, messages: vec![] }) });
    let Some(Popup::Fields(p)) = &st.popup else { panic!("the popup must stay open") };
    assert!(!p.sending);
    assert_eq!(p.inputs[0], "Won't Do");
    assert_eq!(p.errors[0].as_deref(), Some("Resolution is not valid."));
    assert_eq!(st.errors[0].context, "Move BCE-0 to Done");
}

#[test]
fn other_errors_close_the_popup_and_show_in_the_status_line() {
    let mut st = loaded(5, 5);
    popup_open_with_list(&mut st);
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Moved { key: "BCE-0".into(), to: "To Do".into(), result: Err(Error::Forbidden("no".into())) });
    assert_eq!(st.popup, None);
    assert!(st.status.as_ref().unwrap().text.starts_with("You do not have permission"));

    // An error while jui gets the Transitions.
    update(&mut st, key('t'));
    update(&mut st, Event::Transitions { key: "BCE-0".into(), result: Err(Error::Timeout(30)) });
    assert_eq!(st.popup, None);
    assert_eq!(st.errors[0].context, "Get the Transitions of BCE-0");
}

#[test]
fn moved_row_updates_and_dims_when_it_leaves_the_query() {
    let mut st = loaded(5, 5);
    update(&mut st, Event::Moved { key: "BCE-0".into(), to: "Done".into(), result: Ok(()) });
    let mut t = ticket_with_key("BCE-0");
    t.status = "Done".into();
    t.status_category = model::StatusCategory::Done;
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(t)) });
    assert_eq!(list(&st).rows[0].ticket.status, "Done");
    update(&mut st, Event::Matched { tab: 0, key: "BCE-0".into(), result: Ok(false) });
    assert!(list(&st).gone.contains("BCE-0"));
    assert_eq!(list(&st).rows.len(), 5, "the row stays until the next refresh");
    update(&mut st, key('r'));
    update(&mut st, Event::Page { tab: 0, generation: 2, start_at: 0, result: page(1, 4, 4) });
    assert!(list(&st).gone.is_empty());
}

#[test]
fn esc_closes_the_transition_popup() {
    let mut st = loaded(5, 5);
    popup_open_with_list(&mut st);
    update(&mut st, code(KeyCode::Esc));
    assert_eq!(st.popup, None);
    // A late answer for a closed popup does nothing.
    update(&mut st, Event::Transitions { key: "BCE-0".into(), result: Ok(transitions()) });
    assert_eq!(st.popup, None);
}

#[test]
fn snapshot_transition_popup() {
    let mut st = loaded(6, 6);
    popup_open_with_list(&mut st);
    typed(&mut st, "o");
    insta::assert_snapshot!(draw(&mut st, 100, 24));
}

#[test]
fn snapshot_fields_popup_with_error() {
    let mut st = loaded(6, 6);
    popup_open_with_list(&mut st);
    typed(&mut st, "done");
    update(&mut st, code(KeyCode::Enter));
    let mut errs = std::collections::BTreeMap::new();
    errs.insert("Resolution".to_string(), "Resolution is required.".to_string());
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Moved { key: "BCE-0".into(), to: "Done".into(), result: Err(Error::Validation { fields: errs, messages: vec![] }) });
    insta::assert_snapshot!(draw(&mut st, 100, 22));
}

// ------------------------------------------------------------------ create and edit

const EDIT_BUG: &str = include_str!("fixtures/edit_bce_bug.json");
const META_TASK: &str = include_str!("fixtures/createmeta_bce_task.json");

fn edit_data() -> model::EditData {
    let i: api::EditIssue = serde_json::from_str(EDIT_BUG).unwrap();
    i.into()
}

fn task_meta() -> model::CreateMeta {
    let page: api::MetaPage<api::FieldMeta> = serde_json::from_str(META_TASK).unwrap();
    let fields = page.values.into_iter().map(|m| model::ScreenField::from_api(m.field_id.clone().unwrap(), m)).collect();
    model::CreateMeta {
        project: "BCE".into(),
        issue_type: model::IssueType { id: "10100".into(), name: "Task".into(), subtask: false },
        fields,
    }
}

fn editor_text(a: &[Action]) -> String {
    match a {
        [Action::OpenEditor { text, .. }] => text.clone(),
        other => panic!("no OpenEditor: {other:?}"),
    }
}

/// Start an edit with `e` on the first row, and give the edit data. Returns the Template.
fn start_edit(st: &mut State) -> String {
    let a = update(st, key('e'));
    let [Action::PrepareEdit { kind, key }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(key, "BCE-0");
    let mut d = edit_data();
    d.key = "BCE-0".into();
    let a = update(st, Event::EditReady { kind: kind.clone(), result: Box::new(Ok((d, "shacohen".into()))) });
    editor_text(&a)
}

#[test]
fn e_opens_the_editor_and_sends_only_the_changes() {
    let mut st = loaded(5, 5);
    let text = start_edit(&mut st);
    assert!(text.contains("severity = \"Major\""));
    let changed = text.replace("severity = \"Major\"", "severity = \"Minor\"");
    let a = update(&mut st, Event::Edited { result: Ok(changed) });
    assert_eq!(a, vec![Action::Recheck { key: "BCE-0".into() }], "jui reads the Ticket again first");
    let mut d = edit_data();
    d.key = "BCE-0".into();
    let a = update(&mut st, Event::Rechecked { key: "BCE-0".into(), result: Box::new(Ok(d)) });
    let [Action::SendEdit { key, body, changed }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(key, "BCE-0");
    assert_eq!(body, &serde_json::json!({"fields": {"customfield_11908": {"id": "44422"}}}));
    assert_eq!(changed, &vec!["severity".to_string()]);
    let a = update(&mut st, Event::Saved { key: "BCE-0".into(), changed: changed.clone(), result: Ok(()) });
    assert!(a.contains(&Action::FetchTicket { key: "BCE-0".into() }));
    assert!(st.job.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Saved BCE-0. Changed: severity.");
}

#[test]
fn no_change_in_the_editor_sends_nothing() {
    let mut st = loaded(5, 5);
    let text = start_edit(&mut st);
    let a = update(&mut st, Event::Edited { result: Ok(text) });
    assert!(a.is_empty());
    assert!(st.job.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "No changes.");
}

#[test]
fn a_template_that_is_not_valid_opens_the_editor_again() {
    let mut st = loaded(5, 5);
    let text = start_edit(&mut st);
    let bad = text.replace("severity = \"Major\"", "severity = \"S5\"");
    let again = editor_text(&update(&mut st, Event::Edited { result: Ok(bad.clone()) }));
    assert!(again.contains("# ERROR: severity: \"S5\" is not a permitted value."));
    // Saved with no change: jui stops and writes a Draft.
    let a = update(&mut st, Event::Edited { result: Ok(again) });
    let [Action::WriteDraft { label, text, .. }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(label, "BCE-0");
    assert!(text.contains("severity = \"S5\""));
    assert!(st.job.is_none());
}

#[test]
fn a_description_change_shows_the_diff_popup() {
    let mut st = loaded(5, 5);
    let text = start_edit(&mut st);
    let changed = text.replace("See the error.", "See the error message.");
    update(&mut st, Event::Edited { result: Ok(changed) });
    // The Ticket changed in Jira during the edit: the popup also shows that.
    let mut d = edit_data();
    d.key = "BCE-0".into();
    d.updated = Some("2026-10-19T11:00:00.000-0700".into());
    d.summary = "Changed".into();
    let a = update(&mut st, Event::Rechecked { key: "BCE-0".into(), result: Box::new(Ok(d)) });
    assert!(a.is_empty());
    let Some(Popup::Diff(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(p.conflict.as_deref(), Some("BCE-0 changed in Jira while you edited it. Changed: Summary."));
    insta::assert_snapshot!("snapshot_diff_popup", draw(&mut st, 100, 30));

    // `n` opens the editor again with the text of the user.
    let a = update(&mut st, key('n'));
    let again = editor_text(&a);
    assert!(again.contains("See the error message."));
    assert!(again.contains("# ERROR: You did not send the change."));
    // Save again, and `y` sends it.
    update(&mut st, Event::Edited { result: Ok(again) });
    let mut d = edit_data();
    d.key = "BCE-0".into();
    update(&mut st, Event::Rechecked { key: "BCE-0".into(), result: Box::new(Ok(d)) });
    let a = update(&mut st, key('y'));
    let [Action::SendEdit { body, .. }] = a.as_slice() else { panic!("{a:?}") };
    assert!(body["fields"]["description"].as_str().unwrap().contains("# See the error message."));
}

#[test]
fn a_conflict_with_only_field_changes_asks_and_n_keeps_a_draft() {
    let mut st = loaded(5, 5);
    let text = start_edit(&mut st);
    update(&mut st, Event::Edited { result: Ok(text.replace("labels = [\"scanner\"]", "labels = []")) });
    let mut d = edit_data();
    d.key = "BCE-0".into();
    d.updated = Some("2026-10-19T12:00:00.000-0700".into());
    update(&mut st, Event::Rechecked { key: "BCE-0".into(), result: Box::new(Ok(d)) });
    assert!(matches!(st.popup, Some(Popup::Conflict { .. })));
    let a = update(&mut st, key('n'));
    assert!(matches!(a.as_slice(), [Action::WriteDraft { .. }]));
    assert!(st.popup.is_none() && st.job.is_none());
}

/// The create form after `n` and the create screen.
fn create_form(st: &mut State) -> jui::tui::state::CreatePopup {
    match &st.popup {
        Some(Popup::Create(p)) => p.clone(),
        other => panic!("no create form: {other:?}"),
    }
}

/// Press `n` and give the create screen of a Task.
fn start_create(st: &mut State) {
    let a = update(st, key('n'));
    let [Action::PrepareCreate { kind, project, issue_type }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!((project.as_str(), issue_type.as_str()), ("BCE", "Task"));
    let a = update(st, Event::CreateReady { kind: kind.clone(), result: Box::new(Ok((task_meta(), "shacohen".into()))) });
    assert!(a.is_empty(), "the form opens, not the editor: {a:?}");
}

#[test]
fn n_opens_the_create_form_with_the_required_and_the_frequent_fields() {
    let mut st = loaded(5, 5);
    start_create(&mut st);
    let p = create_form(&mut st);
    assert_eq!(p.title, "Create a Task in BCE");
    let ids: Vec<&str> = p.fields.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids[..2], ["summary", "components"], "the summary first, then the required fields");
    for id in ["assignee", "priority", "labels", "customfield_10016", "fixVersions", "customfield_10021"] {
        assert!(ids.contains(&id), "{id} is in the form: {ids:?}");
    }
    for id in ["reporter", "description", "project", "issuetype", "customfield_10020"] {
        assert!(!ids.contains(&id), "{id} is not in the form");
    }
    assert!(st.job.is_some());
    insta::assert_snapshot!("snapshot_create_form", draw(&mut st, 120, 34));
}

#[test]
fn the_create_form_creates_the_ticket_and_a_jira_error_stays_in_the_form() {
    let mut st = loaded(5, 5);
    start_create(&mut st);
    typed(&mut st, "New thing");
    update(&mut st, code(KeyCode::Tab));
    // Components: type to filter, Enter takes the match.
    typed(&mut st, "flow");
    assert_eq!(create_form(&mut st).choices().len(), 1);
    update(&mut st, code(KeyCode::Enter));
    assert_eq!(create_form(&mut st).inputs[1], "DCO-Flow, ");
    // The description: Tab to the end (Shift-Tab from the summary), two lines.
    update(&mut st, code(KeyCode::BackTab));
    update(&mut st, code(KeyCode::BackTab));
    assert!(create_form(&mut st).on_description());
    typed(&mut st, "Line *one*");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, code(KeyCode::Enter));
    typed(&mut st, "Line two");

    let a = update(&mut st, ctrl('s'));
    let [Action::SendCreate { body }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(body["fields"]["summary"], "New thing");
    assert_eq!(body["fields"]["components"].as_array().unwrap().len(), 1);
    assert_eq!(body["fields"]["reporter"], serde_json::json!({"name": "shacohen"}));
    assert_eq!(body["fields"]["description"], "Line _one_\n\nLine two");
    assert!(create_form(&mut st).sending);

    let mut fields = std::collections::BTreeMap::new();
    fields.insert("components".to_string(), "Component/s is not valid.".to_string());
    update(&mut st, Event::Created { result: Err(Error::Validation { fields, messages: vec![] }) });
    let p = create_form(&mut st);
    assert!(!p.sending);
    assert_eq!(p.focus, 1, "the focus goes to the field with the error");
    assert_eq!(p.errors[1].as_deref(), Some("Component/s is not valid."));
    assert_eq!(p.inputs[0], "New thing", "the form keeps the values");

    let a = update(&mut st, ctrl('s'));
    assert!(matches!(a.as_slice(), [Action::SendCreate { .. }]));
    let a = update(&mut st, Event::Created { result: Ok("BCE-9".into()) });
    assert!(a.iter().any(|a| matches!(a, Action::FetchPage { start_at: 0, .. })), "the lists get the new Ticket");
    assert!(st.popup.is_none() && st.job.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Created BCE-9.");
}

#[test]
fn the_create_form_shows_the_missing_fields_and_sends_nothing() {
    let mut st = loaded(5, 5);
    start_create(&mut st);
    typed(&mut st, "Only a summary");
    // Priority: a value that is not permitted.
    let pri = create_form(&mut st).fields.iter().position(|f| f.id == "priority").unwrap();
    for _ in 0..pri {
        update(&mut st, code(KeyCode::Tab));
    }
    typed(&mut st, "Urgent");
    let a = update(&mut st, ctrl('s'));
    assert!(a.is_empty(), "{a:?}");
    let p = create_form(&mut st);
    assert_eq!(p.errors[1].as_deref(), Some("a value is required."));
    assert!(p.errors[pri].as_deref().unwrap().contains("is not a permitted value"));
    assert_eq!(p.focus, 1);
    insta::assert_snapshot!("snapshot_create_form_with_errors", draw(&mut st, 120, 34));
}

#[test]
fn arrows_select_a_permitted_value_in_the_create_form() {
    let mut st = loaded(5, 5);
    start_create(&mut st);
    update(&mut st, code(KeyCode::Tab));
    update(&mut st, code(KeyCode::Down));
    update(&mut st, code(KeyCode::Down));
    update(&mut st, code(KeyCode::Enter));
    let p = create_form(&mut st);
    assert_eq!(p.inputs[1], format!("{}, ", p.fields[1].allowed[1].name));
    // A second value goes after the first; the first is not in the list again.
    update(&mut st, code(KeyCode::Down));
    update(&mut st, code(KeyCode::Enter));
    let p = create_form(&mut st);
    assert_eq!(p.inputs[1], format!("{}, {}, ", p.fields[1].allowed[1].name, p.fields[1].allowed[0].name));
}

#[test]
fn ctrl_e_in_the_create_form_opens_the_editor_with_the_values() {
    let mut st = loaded(5, 5);
    start_create(&mut st);
    typed(&mut st, "From the form");
    let text = editor_text(&update(&mut st, ctrl('e')));
    assert!(text.contains("summary = \"From the form\""), "{text}");
    assert!(st.popup.is_none());
    let filled = text.replace("components = []", "components = [\"DCO-Flow\"]");
    let a = update(&mut st, Event::Edited { result: Ok(filled) });
    let [Action::SendCreate { body }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(body["fields"]["summary"], "From the form");
    // A Jira error now opens the editor again (the form is closed).
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("components".to_string(), "Component/s is required.".to_string());
    let a = update(&mut st, Event::Created { result: Err(Error::Validation { fields, messages: vec![] }) });
    assert!(editor_text(&a).contains("# ERROR: components: Component/s is required."));
}

#[test]
fn ctrl_e_with_no_change_in_the_editor_keeps_the_form_text_in_a_draft() {
    let mut st = loaded(5, 5);
    start_create(&mut st);
    typed(&mut st, "Keep me");
    let text = editor_text(&update(&mut st, ctrl('e')));
    let a = update(&mut st, Event::Edited { result: Ok(text) });
    let [Action::WriteDraft { text, .. }] = a.as_slice() else { panic!("{a:?}") };
    assert!(text.contains("Keep me"));
    assert!(st.job.is_none());
}

#[test]
fn esc_closes_the_create_form_and_keeps_typed_text_in_a_draft() {
    let mut st = loaded(5, 5);
    start_create(&mut st);
    let a = update(&mut st, code(KeyCode::Esc));
    assert!(a.is_empty());
    assert!(st.popup.is_none() && st.job.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Cancelled. No Ticket created.");

    start_create(&mut st);
    typed(&mut st, "Half done");
    let a = update(&mut st, code(KeyCode::Esc));
    let [Action::WriteDraft { text, command, .. }] = a.as_slice() else { panic!("{a:?}") };
    assert!(text.contains("summary = \"Half done\""));
    assert_eq!(command, "new");
    assert!(st.popup.is_none() && st.job.is_none());
}

#[test]
fn q_and_digits_are_text_in_the_create_form() {
    let mut st = loaded(5, 5);
    start_create(&mut st);
    typed(&mut st, "q1 n?");
    assert_eq!(create_form(&mut st).inputs[0], "q1 n?");
    assert!(!st.quit);
}

#[test]
fn shift_n_in_the_zoom_creates_a_subtask_with_the_parent() {
    let mut st = loaded(5, 5);
    update(&mut st, code(KeyCode::Enter));
    assert!(update(&mut st, key('N')).iter().any(|a| matches!(a, Action::PrepareCreate { kind: TemplateKind::Subtask { parent }, project, issue_type } if parent == "BCE-0" && project == "BCE" && issue_type == "subtask")));
    // W only works in the zoom, and opens a raw wiki edit.
    let a = update(&mut st, key('W'));
    assert!(matches!(a.as_slice(), [Action::PrepareEdit { kind: TemplateKind::Edit { wiki: true, .. }, .. }]) || st.job.is_none());
}

#[test]
fn only_one_template_job_at_a_time_and_errors_show() {
    let mut st = loaded(5, 5);
    start_edit(&mut st);
    assert!(update(&mut st, key('n')).is_empty());
    assert!(st.status.as_ref().unwrap().text.contains("A create or edit is running"));
    let mut st = loaded(5, 5);
    let a = update(&mut st, key('e'));
    let [Action::PrepareEdit { kind, .. }] = a.as_slice() else { panic!() };
    update(&mut st, Event::EditReady { kind: kind.clone(), result: Box::new(Err(Error::NotFound("Ticket BCE-0".into()))) });
    assert_eq!(st.errors[0].context, "Edit BCE-0");
    assert!(st.job.is_none());
}

// ------------------------------------------------------------------ comments

use jui::tui::state::CommentStart;

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

/// The comment context of the fixture Ticket (comments 7001 and 7002).
fn comment_ctx(key: &str) -> model::CommentContext {
    let t = ticket_with_key(key);
    model::CommentContext { key: key.into(), summary: t.summary.clone(), comments: t.comments.clone() }
}

fn sent_comment(id: &str, body: &str) -> model::Comment {
    let mut c = full_ticket().comments[0].clone();
    c.id = id.into();
    c.body = model::Text::from_wiki(body.into());
    c
}

/// Open the zoom of BCE-0 and select the last comment (7002).
fn zoom_on_last_comment() -> State {
    let mut st = loaded(5, 5);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    for _ in 0..6 {
        update(&mut st, key('j'));
    }
    assert_eq!(st.selected_comment(), Some(("BCE-0".into(), "7002".into())));
    st
}

#[test]
fn c_popup_sends_one_line_with_mentions_and_refreshes() {
    let mut st = loaded(5, 5);
    update(&mut st, key('c'));
    assert!(matches!(&st.popup, Some(Popup::Comment(p)) if p.key == "BCE-0"));
    typed(&mut st, "Hi @jdoe, **ok**");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::SendComment { key: "BCE-0".into(), id: None, wiki: "Hi [~jdoe], *ok*".into() }]);
    // Keys wait while the request runs.
    update(&mut st, key('x'));
    assert!(matches!(&st.popup, Some(Popup::Comment(p)) if p.sending && p.input == "Hi @jdoe, **ok**"));
    let a = update(&mut st, Event::CommentSent { key: "BCE-0".into(), id: None, result: Box::new(Ok(sent_comment("7003", "x"))) });
    assert!(st.popup.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Added comment 7003 to BCE-0.");
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }], "the Ticket is read again");
}

#[test]
fn c_popup_with_an_empty_text_cancels_and_esc_closes() {
    let mut st = loaded(5, 5);
    update(&mut st, key('c'));
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    assert!(st.popup.is_none());
    update(&mut st, key('c'));
    typed(&mut st, "abc");
    update(&mut st, code(KeyCode::Esc));
    assert!(st.popup.is_none());
}

#[test]
fn c_popup_validation_error_keeps_the_text() {
    let mut st = loaded(5, 5);
    update(&mut st, key('c'));
    typed(&mut st, "Text");
    update(&mut st, code(KeyCode::Enter));
    let e = Error::Validation { fields: [("comment".to_string(), "Too long.".to_string())].into(), messages: vec![] };
    update(&mut st, Event::CommentSent { key: "BCE-0".into(), id: None, result: Box::new(Err(e)) });
    let Some(Popup::Comment(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(p.input, "Text");
    assert_eq!(p.error.as_deref(), Some("comment: Too long."));
    assert!(!p.sending);
}

#[test]
fn c_popup_network_error_writes_a_draft() {
    let mut st = loaded(5, 5);
    update(&mut st, key('c'));
    typed(&mut st, "Keep me");
    update(&mut st, code(KeyCode::Enter));
    let e = Error::Timeout(30);
    let a = update(&mut st, Event::CommentSent { key: "BCE-0".into(), id: None, result: Box::new(Err(e)) });
    assert!(st.popup.is_none());
    assert!(matches!(a.as_slice(), [Action::WriteDraft { text, command, .. }] if text == "Keep me" && command == "comment add BCE-0"), "{a:?}");
    assert!(st.status.as_ref().unwrap().text.contains("No response from Jira"));
}

#[test]
fn ctrl_e_moves_the_popup_text_to_the_editor() {
    let mut st = loaded(5, 5);
    update(&mut st, key('c'));
    typed(&mut st, "Start");
    let a = update(&mut st, ctrl('e'));
    assert_eq!(a, vec![Action::PrepareComment { key: "BCE-0".into(), start: CommentStart::Add { text: "Start".into() } }]);
    assert!(st.popup.is_none());
    let a = update(&mut st, Event::CommentReady { key: "BCE-0".into(), start: CommentStart::Add { text: "Start".into() }, result: Box::new(Ok(comment_ctx("BCE-0"))) });
    let text = editor_text(&a);
    assert!(text.starts_with("Start\n"), "{text}");
    assert!(text.contains("# The last comments (2 of 2):"), "{text}");
    // A new comment from the editor is sent with no diff.
    let saved = text.replacen("Start", "Start and @jdoe", 1);
    let a = update(&mut st, Event::Edited { result: Ok(saved) });
    assert_eq!(a, vec![Action::SendComment { key: "BCE-0".into(), id: None, wiki: "Start and [~jdoe]".into() }]);
    update(&mut st, Event::CommentSent { key: "BCE-0".into(), id: None, result: Box::new(Ok(sent_comment("7003", "x"))) });
    assert!(st.comment_job.is_none());
}

#[test]
fn e_on_a_comment_edits_it_with_a_diff() {
    let mut st = zoom_on_last_comment();
    let a = update(&mut st, key('e'));
    assert_eq!(a, vec![Action::PrepareComment { key: "BCE-0".into(), start: CommentStart::Edit { id: "7002".into(), wiki: false } }]);
    let a = update(&mut st, Event::CommentReady { key: "BCE-0".into(), start: CommentStart::Edit { id: "7002".into(), wiki: false }, result: Box::new(Ok(comment_ctx("BCE-0"))) });
    let text = editor_text(&a);
    assert!(text.starts_with("Done. See `retry.rs`.\n"), "{text}");

    // No change: nothing is sent.
    let a = update(&mut st, Event::Edited { result: Ok(text.clone()) });
    assert!(a.is_empty());
    assert_eq!(st.status.as_ref().unwrap().text, "No changes.");
    assert!(st.comment_job.is_none());

    // A change: jui reads the comment again, then shows the diff.
    update(&mut st, key('e'));
    let a = update(&mut st, Event::CommentReady { key: "BCE-0".into(), start: CommentStart::Edit { id: "7002".into(), wiki: false }, result: Box::new(Ok(comment_ctx("BCE-0"))) });
    let saved = editor_text(&a).replacen("Done.", "Done!", 1);
    let a = update(&mut st, Event::Edited { result: Ok(saved) });
    assert_eq!(a, vec![Action::RecheckComment { key: "BCE-0".into(), id: "7002".into() }]);
    let now = full_ticket().comments[1].clone();
    update(&mut st, Event::CommentRechecked { key: "BCE-0".into(), id: "7002".into(), result: Box::new(Ok(now)) });
    let Some(Popup::Diff(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(p.subject, "Comment 7002");
    assert!(p.conflict.is_none());
    assert!(p.lines.iter().any(|l| l.tag == '+' && l.text == "Done! See {{retry.rs}}."), "{:?}", p.lines);
    let a = update(&mut st, key('y'));
    assert_eq!(a, vec![Action::SendComment { key: "BCE-0".into(), id: Some("7002".into()), wiki: "Done! See {{retry.rs}}.".into() }]);
    update(&mut st, Event::CommentSent { key: "BCE-0".into(), id: Some("7002".into()), result: Box::new(Ok(sent_comment("7002", "x"))) });
    assert_eq!(st.status.as_ref().unwrap().text, "Saved comment 7002 on BCE-0.");
}

#[test]
fn a_comment_that_changed_in_jira_shows_the_conflict_in_the_diff() {
    let mut st = zoom_on_last_comment();
    update(&mut st, key('W'));
    let a = update(&mut st, Event::CommentReady { key: "BCE-0".into(), start: CommentStart::Edit { id: "7002".into(), wiki: true }, result: Box::new(Ok(comment_ctx("BCE-0"))) });
    let text = editor_text(&a);
    assert!(text.starts_with("Done. See {{retry.rs}}.\n"), "W edits the raw wiki markup: {text}");
    let a = update(&mut st, Event::Edited { result: Ok(text.replacen("Done.", "Done!", 1)) });
    assert_eq!(a.len(), 1);
    let now = sent_comment("7002", "Changed by jdoe.");
    update(&mut st, Event::CommentRechecked { key: "BCE-0".into(), id: "7002".into(), result: Box::new(Ok(now)) });
    let Some(Popup::Diff(p)) = &st.popup else { panic!() };
    assert_eq!(p.conflict.as_deref(), Some("Comment 7002 on BCE-0 changed in Jira while you edited it."));
    // Esc keeps a Draft with the command for the comment.
    let a = update(&mut st, code(KeyCode::Esc));
    assert!(matches!(a.as_slice(), [Action::WriteDraft { command, .. }] if command == "comment edit BCE-0 7002 --wiki"), "{a:?}");
    assert!(st.comment_job.is_none());
}

#[test]
fn n_in_the_comment_diff_opens_the_editor_again() {
    let mut st = zoom_on_last_comment();
    update(&mut st, key('e'));
    let a = update(&mut st, Event::CommentReady { key: "BCE-0".into(), start: CommentStart::Edit { id: "7002".into(), wiki: false }, result: Box::new(Ok(comment_ctx("BCE-0"))) });
    update(&mut st, Event::Edited { result: Ok(editor_text(&a).replacen("Done.", "Done!", 1)) });
    update(&mut st, Event::CommentRechecked { key: "BCE-0".into(), id: "7002".into(), result: Box::new(Ok(full_ticket().comments[1].clone())) });
    let a = update(&mut st, key('n'));
    let text = editor_text(&a);
    assert!(text.starts_with("# ERROR: You did not send the change."), "{text}");
    assert!(text.contains("Done! See"), "the text of the user stays: {text}");
}

#[test]
fn d_on_a_comment_asks_and_deletes() {
    let mut st = zoom_on_last_comment();
    update(&mut st, key('d'));
    let Some(Popup::ConfirmDelete { id, preview, .. }) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(id, "7002");
    assert_eq!(preview, "@jdoe: Done. See retry.rs.");
    // n keeps the comment.
    assert!(update(&mut st, key('n')).is_empty());
    assert!(st.popup.is_none());
    update(&mut st, key('d'));
    let a = update(&mut st, key('y'));
    assert_eq!(a, vec![Action::DeleteComment { key: "BCE-0".into(), id: "7002".into() }]);
    let a = update(&mut st, Event::CommentDeleted { key: "BCE-0".into(), id: "7002".into(), result: Ok(()) });
    assert!(st.popup.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Deleted comment 7002 on BCE-0.");
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }]);
    assert_eq!(st.tabs[0].zoom[0].item, 5, "the selection moves to the item before");
}

#[test]
fn d_without_a_selected_comment_does_nothing() {
    let mut st = loaded(5, 5);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    assert!(update(&mut st, key('d')).is_empty());
    assert!(st.popup.is_none());
    assert!(st.status.as_ref().unwrap().text.contains("Select a comment"));
    // e on a Sub-task edits the Ticket, not a comment.
    let a = update(&mut st, key('e'));
    assert!(matches!(a.as_slice(), [Action::PrepareEdit { .. }]), "{a:?}");
}

#[test]
fn a_comment_job_and_a_template_job_do_not_run_together() {
    let mut st = zoom_on_last_comment();
    update(&mut st, key('e'));
    update(&mut st, Event::CommentReady { key: "BCE-0".into(), start: CommentStart::Edit { id: "7002".into(), wiki: false }, result: Box::new(Ok(comment_ctx("BCE-0"))) });
    update(&mut st, code(KeyCode::Esc));
    assert!(update(&mut st, key('n')).is_empty());
    assert!(st.status.as_ref().unwrap().text.contains("A create or edit is running"));
    assert!(update(&mut st, key('c')).is_empty());
}

#[test]
fn snapshot_comment_popup_and_selected_comment() {
    let mut st = zoom_on_last_comment();
    update(&mut st, key('c'));
    typed(&mut st, "Looks good @jdoe");
    insta::assert_snapshot!(draw(&mut st, 100, 40));
}

// ---------------------------------------------------------------- assign, watch, copy

fn user(name: &str, display: &str) -> model::User {
    model::User { name: name.into(), display_name: display.into() }
}

fn type_text(st: &mut State, text: &str) -> Vec<Action> {
    let mut all = Vec::new();
    for c in text.chars() {
        all.extend(update(st, key(c)));
    }
    all
}

fn user_popup(st: &State) -> &jui::tui::state::UserPopup {
    match &st.popup {
        Some(Popup::User(p)) => p,
        p => panic!("{p:?}"),
    }
}

#[test]
fn a_opens_the_picker_with_me_and_none_and_assigns_me() {
    use jui::core::user::Assignee;
    use jui::tui::state::PickRow;
    let mut st = loaded(5, 5);
    let a = update(&mut st, key('a'));
    assert!(a.is_empty(), "no request before the user types");
    assert_eq!(user_popup(&st).rows(&st.recent_users), [PickRow::Me, PickRow::Nobody]);
    // `n` filters to `none`, Backspace shows both again.
    type_text(&mut st, "n");
    assert_eq!(user_popup(&st).rows(&st.recent_users), [PickRow::Nobody]);
    update(&mut st, code(KeyCode::Backspace));
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::Assign { key: "BCE-0".into(), who: Assignee::Me }]);
    assert!(user_popup(&st).sending);

    let a = update(&mut st, Event::Assigned { key: "BCE-0".into(), who: Assignee::Me, result: Ok(()) });
    assert!(st.popup.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Assigned BCE-0 to you.");
    assert!(a.contains(&Action::FetchTicket { key: "BCE-0".into() }));
    assert!(a.iter().any(|x| matches!(x, Action::CheckMatch { key, .. } if key == "BCE-0")));
}

#[test]
fn the_picker_searches_after_the_delay_and_ignores_old_results() {
    use jui::core::user::Assignee;
    let mut st = loaded(5, 5);
    update(&mut st, key('a'));
    // One character: no search.
    assert!(type_text(&mut st, "j").is_empty());
    // Two characters: a delayed search for each key.
    let a = type_text(&mut st, "an");
    assert_eq!(a, vec![Action::ScheduleUserSearch { seq: 2 }, Action::ScheduleUserSearch { seq: 3 }]);
    // The delay of an old key does nothing.
    assert!(update(&mut st, Event::UserSearchDue { seq: 2 }).is_empty());
    let a = update(&mut st, Event::UserSearchDue { seq: 3 });
    assert_eq!(a, vec![Action::SearchUsers { key: "BCE-0".into(), text: "jan".into(), seq: 3 }]);
    // A late result of an old text is ignored.
    update(&mut st, Event::Users { key: "BCE-0".into(), seq: 2, result: Ok(vec![user("old", "Old")]) });
    assert!(user_popup(&st).results.is_none());
    update(&mut st, Event::Users { key: "BCE-0".into(), seq: 3, result: Ok(vec![user("jdoe", "Jane Doe"), user("jroe", "Jane Roe")]) });
    assert!(!user_popup(&st).searching);
    update(&mut st, code(KeyCode::Down));
    let a = update(&mut st, code(KeyCode::Enter));
    let roe = Assignee::User(user("jroe", "Jane Roe"));
    assert_eq!(a, vec![Action::Assign { key: "BCE-0".into(), who: roe.clone() }]);
    update(&mut st, Event::Assigned { key: "BCE-0".into(), who: roe, result: Ok(()) });
    assert_eq!(st.status.as_ref().unwrap().text, "Assigned BCE-0 to Jane Roe (jroe).");
    // The next picker shows the recent user with no text.
    update(&mut st, key('a'));
    assert_eq!(user_popup(&st).rows(&st.recent_users).len(), 3);
}

#[test]
fn an_assign_error_keeps_the_picker_open_with_the_text() {
    use jui::core::user::Assignee;
    let mut st = loaded(5, 5);
    update(&mut st, key('a'));
    update(&mut st, code(KeyCode::Enter));
    let e = Error::Validation { fields: [("assignee".to_string(), "User 'x' cannot be assigned.".to_string())].into(), messages: vec![] };
    update(&mut st, Event::Assigned { key: "BCE-0".into(), who: Assignee::Me, result: Err(e) });
    let p = user_popup(&st);
    assert!(!p.sending);
    assert_eq!(p.error.as_deref(), Some("User 'x' cannot be assigned."));
    assert_eq!(st.errors.len(), 1);
    // A search error shows in the popup too.
    type_text(&mut st, "zz");
    let seq = user_popup(&st).seq;
    update(&mut st, Event::Users { key: "BCE-0".into(), seq, result: Err(Error::Timeout(30)) });
    assert_eq!(user_popup(&st).error.as_deref(), Some("No response from Jira after 30 s."));
    // Enter with no rows does not send.
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    update(&mut st, code(KeyCode::Esc));
    assert!(st.popup.is_none());
}

#[test]
fn w_toggles_the_watch_from_the_full_ticket() {
    let mut st = loaded(5, 5);
    // No full Ticket yet: jui gets it and asks to push w again.
    let a = update(&mut st, key('w'));
    assert!(a.is_empty() || a == vec![Action::FetchTicket { key: "BCE-0".into() }]);
    assert!(st.status.as_ref().unwrap().text.contains("Push w again"));

    let mut t = ticket_with_key("BCE-0");
    t.watching = true;
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(t)) });
    let a = update(&mut st, key('w'));
    assert_eq!(a, vec![Action::Watch { key: "BCE-0".into(), on: false }]);
    // A second w during the request does nothing.
    assert!(update(&mut st, key('w')).is_empty());
    let a = update(&mut st, Event::Watched { key: "BCE-0".into(), on: false, result: Ok(()) });
    assert_eq!(st.status.as_ref().unwrap().text, "You no longer watch BCE-0.");
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }]);
    // An error goes to the status line.
    let t = ticket_with_key("BCE-0");
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(t)) });
    assert_eq!(update(&mut st, key('w')), vec![Action::Watch { key: "BCE-0".into(), on: true }]);
    update(&mut st, Event::Watched { key: "BCE-0".into(), on: true, result: Err(Error::Forbidden("No.".into())) });
    assert!(st.status.as_ref().unwrap().text.starts_with("You do not have permission: No."));
}

#[test]
fn y_copies_the_selected_key_and_o_opens_the_zoomed_ticket() {
    let mut st = loaded(5, 5);
    update(&mut st, key('j'));
    assert_eq!(update(&mut st, key('y')), vec![Action::Copy { text: "BCE-1".into() }]);
    update(&mut st, Event::Copied { text: "BCE-1".into(), result: Ok("pbcopy") });
    assert_eq!(st.status.as_ref().unwrap().text, "Copied BCE-1 (pbcopy).");
    update(&mut st, code(KeyCode::Enter));
    let a = update(&mut st, key('o'));
    assert_eq!(a, vec![Action::OpenBrowser { url: "https://jira.example/browse/BCE-1".into() }]);
    assert_eq!(update(&mut st, key('y')), vec![Action::Copy { text: "BCE-1".into() }]);
}

#[test]
fn snapshot_user_picker_with_results() {
    let mut st = loaded(5, 5);
    let mut t = ticket_with_key("BCE-0");
    t.watching = true;
    t.watch_count = 2;
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(t)) });
    update(&mut st, key('a'));
    type_text(&mut st, "jdo");
    let seq = user_popup(&st).seq;
    update(&mut st, Event::Users { key: "BCE-0".into(), seq, result: Ok(vec![user("jdoe", "Jane Doe"), user("jdoe2", "John Doe")]) });
    update(&mut st, code(KeyCode::Down));
    insta::assert_snapshot!("snapshot_user_picker", draw(&mut st, 100, 30));
}

// ------------------------------------------------------------------ search and filters

use jui::tui::state::SEARCH;

fn filters() -> Vec<model::Filter> {
    let f = |id: &str, name: &str, jql: &str| model::Filter {
        id: id.into(),
        name: name.into(),
        owner: Some("shacohen".into()),
        jql: jql.into(),
        description: None,
        favourite: true,
        url: format!("https://jira.example/issues/?filter={id}"),
    };
    vec![
        f("201", "BCE bugs", "project = BCE AND type = Bug"),
        f("203", "My open work", "assignee = currentUser() AND statusCategory != Done"),
    ]
}

#[test]
fn slash_opens_the_search_tab_with_the_focus_on_the_input() {
    let mut st = loaded(5, 5);
    let a = update(&mut st, key('5'));
    assert!(a.is_empty(), "the Search tab runs no query at the start: {a:?}");
    update(&mut st, key('1'));
    update(&mut st, key('/'));
    assert_eq!(st.tab, SEARCH);
    assert!(st.search.focus);
    // Letters go to the input; they are not commands.
    let a = type_text(&mut st, "project = BCE");
    assert!(a.is_empty());
    assert!(!st.quit);
    assert_eq!(st.search.input, "project = BCE");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchPage { tab: SEARCH, jql: "project = BCE".into(), start_at: 0, generation: 1 }]);
    assert!(!st.search.focus, "the list gets the focus");
    assert_eq!(st.search.history, ["project = BCE"]);
}

#[test]
fn search_results_use_the_same_list_and_zoom() {
    let mut st = loaded(5, 5);
    update(&mut st, key('/'));
    type_text(&mut st, "x = 1");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Page { tab: SEARCH, generation: 1, start_at: 0, result: page(0, 3, 3) });
    let l = st.tabs[SEARCH].list.as_ref().unwrap();
    assert_eq!(l.rows.len(), 3);
    update(&mut st, key('j'));
    update(&mut st, code(KeyCode::Enter));
    assert_eq!(st.tabs[SEARCH].zoom[0].key, "BCE-1");
    // My Tickets keeps its own rows.
    assert_eq!(list(&st).rows.len(), 5);
}

#[test]
fn search_input_edits_and_history() {
    let mut st = loaded(5, 5);
    update(&mut st, key('/'));
    type_text(&mut st, "a = 1");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, key('/'));
    update(&mut st, ctrl('u'));
    assert_eq!(st.search.input, "");
    type_text(&mut st, "b = 2");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, key('/'));
    // The cursor moves; a new letter goes in at the cursor.
    update(&mut st, code(KeyCode::Home));
    update(&mut st, key('x'));
    assert_eq!(st.search.input, "xb = 2");
    update(&mut st, code(KeyCode::Backspace));
    update(&mut st, code(KeyCode::End));
    update(&mut st, ctrl('w'));
    assert_eq!(st.search.input, "b = ");
    // ↑ shows the last queries; ↓ goes back to the text of the user.
    update(&mut st, code(KeyCode::Up));
    assert_eq!(st.search.input, "b = 2");
    update(&mut st, code(KeyCode::Up));
    assert_eq!(st.search.input, "a = 1");
    update(&mut st, code(KeyCode::Up));
    assert_eq!(st.search.input, "a = 1", "the oldest query stays");
    update(&mut st, code(KeyCode::Down));
    update(&mut st, code(KeyCode::Down));
    assert_eq!(st.search.input, "b = ");
    // Esc gives the focus to the list and keeps the text.
    update(&mut st, code(KeyCode::Esc));
    assert!(!st.search.focus);
    assert_eq!(st.search.input, "b = ");
    // An empty query does not run.
    update(&mut st, key('/'));
    update(&mut st, ctrl('u'));
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    assert!(st.search.error.as_ref().unwrap().starts_with("Type a JQL query"));
}

#[test]
fn a_jql_error_shows_below_the_input_and_gives_it_the_focus() {
    let mut st = loaded(5, 5);
    update(&mut st, key('/'));
    type_text(&mut st, "project =");
    update(&mut st, code(KeyCode::Enter));
    let e = Error::Validation { fields: Default::default(), messages: vec!["Error in the JQL Query: Expecting a value.".into()] };
    update(&mut st, Event::Page { tab: SEARCH, generation: 1, start_at: 0, result: Err(e) });
    assert_eq!(st.search.error.as_deref(), Some("Error in the JQL Query: Expecting a value."));
    assert!(st.search.focus);
    assert!(st.status.is_none(), "the error is in the Search tab, not in the status line");
    assert_eq!(st.errors[0].context, "Search with JQL");
    // A tab change does not run the bad JQL again.
    update(&mut st, code(KeyCode::Esc));
    update(&mut st, key('1'));
    assert!(update(&mut st, key('5')).is_empty());
    update(&mut st, key('/'));
    // A good query removes the error.
    type_text(&mut st, " BCE");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Page { tab: SEARCH, generation: 2, start_at: 0, result: page(0, 2, 2) });
    assert!(st.search.error.is_none());
}

#[test]
fn ctrl_f_runs_a_favourite_filter() {
    let mut st = loaded(5, 5);
    let a = update(&mut st, ctrl('f'));
    assert_eq!(st.tab, SEARCH);
    assert_eq!(a, vec![Action::FetchFavouriteFilters]);
    assert!(matches!(st.popup, Some(Popup::Filters(_))));
    // Enter before the list arrives does nothing.
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::FavouriteFilters { result: Ok(filters()) });
    type_text(&mut st, "open");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(st.popup, None);
    assert_eq!(st.search.input, "assignee = currentUser() AND statusCategory != Done");
    assert_eq!(st.search.filter.as_deref(), Some("My open work"));
    assert!(matches!(&a[..], [Action::FetchPage { tab: SEARCH, start_at: 0, .. }]));
    // A JQL that the user types removes the filter name.
    update(&mut st, key('/'));
    type_text(&mut st, " ORDER BY key");
    update(&mut st, code(KeyCode::Enter));
    assert_eq!(st.search.filter, None);
}

#[test]
fn ctrl_f_with_a_filter_id_gets_that_filter() {
    let mut st = loaded(5, 5);
    update(&mut st, ctrl('f'));
    update(&mut st, Event::FavouriteFilters { result: Ok(Vec::new()) });
    update(&mut st, code(KeyCode::Enter));
    let Some(Popup::Filters(p)) = &st.popup else { panic!() };
    assert!(p.error.as_ref().unwrap().contains("Type a filter ID"));
    type_text(&mut st, "117549");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchFilter { id: "117549".into() }]);
    // A second Enter during the request sends nothing.
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    update(&mut st, Event::FilterLoaded { id: "117549".into(), result: Err(Error::NotFound("Filter 117549".into())) });
    let Some(Popup::Filters(p)) = &st.popup else { panic!() };
    assert_eq!(p.error.as_deref(), Some("Filter 117549 not found."));
    update(&mut st, code(KeyCode::Enter));
    let mut f = filters()[0].clone();
    f.name = "Filter for BCE board".into();
    let a = update(&mut st, Event::FilterLoaded { id: "117549".into(), result: Ok(f) });
    assert_eq!(st.popup, None);
    assert_eq!(st.search.filter.as_deref(), Some("Filter for BCE board"));
    assert_eq!(a.len(), 1);
}

#[test]
fn ctrl_f_in_the_input_and_a_filter_error() {
    let mut st = loaded(5, 5);
    update(&mut st, key('/'));
    let a = update(&mut st, ctrl('f'));
    assert_eq!(a, vec![Action::FetchFavouriteFilters]);
    update(&mut st, Event::FavouriteFilters { result: Err(Error::Server { status: 500, body: "x".into() }) });
    let Some(Popup::Filters(p)) = &st.popup else { panic!() };
    assert!(p.error.as_ref().unwrap().contains("HTTP 500"));
    update(&mut st, code(KeyCode::Esc));
    assert_eq!(st.popup, None);
    assert!(!st.search.focus);
}

#[test]
fn snapshot_search_tab_before_a_query() {
    let mut st = loaded(5, 5);
    update(&mut st, key('/'));
    type_text(&mut st, "project = BCE");
    insta::assert_snapshot!(draw(&mut st, 100, 16));
}

#[test]
fn snapshot_search_results_with_a_filter_and_an_error() {
    let mut st = loaded(5, 5);
    update(&mut st, ctrl('f'));
    update(&mut st, Event::FavouriteFilters { result: Ok(filters()) });
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Page { tab: SEARCH, generation: 1, start_at: 0, result: page(0, 4, 4) });
    insta::assert_snapshot!("search_results", draw(&mut st, 120, 12));
    update(&mut st, key('/'));
    type_text(&mut st, " AND");
    update(&mut st, code(KeyCode::Enter));
    let e = Error::Validation { fields: Default::default(), messages: vec!["Error in the JQL Query: Expecting a field name.".into()] };
    update(&mut st, Event::Page { tab: SEARCH, generation: 2, start_at: 0, result: Err(e) });
    insta::assert_snapshot!("search_jql_error", draw(&mut st, 120, 10));
}

#[test]
fn snapshot_filter_popup() {
    let mut st = loaded(5, 5);
    update(&mut st, ctrl('f'));
    update(&mut st, Event::FavouriteFilters { result: Ok(filters()) });
    update(&mut st, code(KeyCode::Down));
    insta::assert_snapshot!(draw(&mut st, 100, 14));
}

// ------------------------------------------------------------------ Boards

use jui::core::board::{Origin, Suggestion};
use jui::core::model::{Board, BoardColumn, BoardView, SprintRef};
use jui::core::settings::BoardRef;
use jui::tui::state::{BOARDS, BoardPopup};

fn card(key: &str) -> TicketSummary {
    let mut t = row(1).ticket;
    t.key = key.into();
    t.summary = format!("Summary of {key}");
    t
}

fn board_view(id: u64, name: &str, cols: &[(&str, &str, &[&str])]) -> BoardView {
    let columns: Vec<BoardColumn> = cols
        .iter()
        .map(|(n, sid, keys)| BoardColumn { name: (*n).into(), status_ids: vec![(*sid).into()], tickets: keys.iter().map(|k| card(k)).collect() })
        .collect();
    let total = columns.iter().map(|c| c.tickets.len() as u64).sum();
    BoardView {
        board: Board { id, name: name.into(), kind: "scrum".into() },
        url: format!("https://jira.example/secure/RapidBoard.jspa?rapidView={id}"),
        filter_id: Some("1".into()),
        sub_query: None,
        sprints: vec![SprintRef { id: 7, name: "Sprint 21".into(), state: "active".into() }],
        jql: Some("x".into()),
        columns,
        unmapped: 0,
        total,
        loaded: total,
        only_mine: false,
    }
}

fn bce_board() -> BoardView {
    board_view(4166, "BCE board", &[("To Do", "10000", &["BCE-1", "BCE-2"]), ("Review", "5", &[]), ("Done", "10001", &["BCE-3"])])
}

fn boards_settings(dir: &std::path::Path, ids: &[(u64, &str)]) -> Settings {
    Settings {
        base_url: "https://jira.example".into(),
        my_boards: ids.iter().map(|(id, n)| BoardRef { id: *id, name: Some((*n).into()) }).collect(),
        path: dir.join("config.toml"),
        ..Settings::default()
    }
}

fn boards_state(dir: &std::path::Path, ids: &[(u64, &str)]) -> State {
    let mut st = State::new(&boards_settings(dir, ids), 120, 40);
    st.now = Some("2026-10-19T12:00:00Z".parse::<DateTime<Utc>>().unwrap());
    st
}

/// Go to the Boards tab and give it the first Board.
fn on_board(st: &mut State, v: BoardView) {
    let a = update(st, key('2'));
    let gen_ = match a.as_slice() {
        [Action::FetchBoard { generation, .. }] => *generation,
        other => panic!("{other:?}"),
    };
    update(st, Event::BoardLoaded { id: v.board.id, generation: gen_, result: Box::new(Ok(v)) });
}

#[test]
fn boards_tab_gets_the_first_board_and_selects_the_first_card() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board"), (4533, "Kanban")]);
    let a = update(&mut st, key('2'));
    assert_eq!(a, vec![Action::FetchBoard { id: 4166, generation: 1, only_mine: false }]);
    let a = update(&mut st, Event::BoardLoaded { id: 4166, generation: 1, result: Box::new(Ok(bce_board())) });
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-1".into() }]);
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-1"));
    // A second visit keeps the state and sends no request.
    update(&mut st, key('1'));
    assert!(update(&mut st, key('2')).is_empty());
}

#[test]
fn board_keys_move_in_and_between_columns_and_skip_empty_columns() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    assert_eq!(update(&mut st, key('j')), vec![Action::SchedulePreview { key: "BCE-2".into() }]);
    // The last card: j does nothing.
    assert!(update(&mut st, key('j')).is_empty());
    // l skips the empty "Review" column and goes to "Done".
    update(&mut st, key('l'));
    assert_eq!(st.boards.col, 2);
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-3"));
    // h goes back to "To Do". Each column keeps its selected card.
    update(&mut st, key('h'));
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-2"));
    // Enter opens the zoom of the card. Esc goes back to the Board.
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-2".into() }]);
    assert_eq!(st.tabs[BOARDS].zoom.len(), 1);
    update(&mut st, code(KeyCode::Esc));
    assert!(st.tabs[BOARDS].zoom.is_empty());
}

#[test]
fn brackets_change_the_board_and_ignore_late_responses() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board"), (4533, "Kanban"), (6589, "A Team")]);
    on_board(&mut st, bce_board());
    assert_eq!(update(&mut st, key(']')), vec![Action::FetchBoard { id: 4533, generation: 2, only_mine: false }]);
    // The old Board stays, dim, until the new one arrives. No card is selected.
    assert!(st.boards.stale);
    assert_eq!(st.selected_key_any(), None);
    // The user pushes ] again before the answer: the answer of 4533 is late and ignored.
    assert_eq!(update(&mut st, key(']')), vec![Action::FetchBoard { id: 6589, generation: 3, only_mine: false }]);
    update(&mut st, Event::BoardLoaded { id: 4533, generation: 2, result: Box::new(Ok(board_view(4533, "Kanban", &[]))) });
    assert!(st.boards.stale);
    update(&mut st, Event::BoardLoaded { id: 6589, generation: 3, result: Box::new(Ok(board_view(6589, "A Team", &[("To Do", "1", &["AT-1"])]))) });
    assert_eq!(st.selected_key_any().as_deref(), Some("AT-1"));
    // [ goes back to 4533 (not in the memory cache: its answer was ignored), then [ to 4166 (in the cache).
    assert_eq!(update(&mut st, key('[')), vec![Action::FetchBoard { id: 4533, generation: 4, only_mine: false }]);
    // BCE-1 is in the Ticket cache from the first visit, so no request is necessary.
    let a = update(&mut st, key('['));
    assert!(a.is_empty(), "{a:?}");
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-1"));
    assert_eq!(st.boards.view.as_ref().unwrap().board.id, 4166);
    assert!(!st.boards.stale);
}

#[test]
fn refresh_keeps_the_selected_card() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    update(&mut st, key('l'));
    assert_eq!(update(&mut st, key('r')), vec![Action::FetchBoard { id: 4166, generation: 2, only_mine: false }]);
    assert!(!st.boards.stale, "a refresh of the same Board does not dim it");
    // In the new data, BCE-3 is in "To Do".
    let v = board_view(4166, "BCE board", &[("To Do", "10000", &["BCE-1", "BCE-3"]), ("Done", "10001", &[])]);
    update(&mut st, Event::BoardLoaded { id: 4166, generation: 2, result: Box::new(Ok(v)) });
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-3"));
    assert_eq!((st.boards.col, st.boards.rows[0]), (0, 1));
}

#[test]
fn m_shows_only_my_tickets_and_saves_the_setting() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    update(&mut st, key('j'));
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-2"));

    // m: save the Setting, and get the Board again with only my Tickets.
    let a = update(&mut st, key('m'));
    assert_eq!(
        a,
        vec![
            Action::SaveSetting { key: "boards.only_mine".into(), value: Some("on".into()) },
            Action::FetchBoard { id: 4166, generation: 2, only_mine: true },
        ]
    );
    assert!(st.boards.only_mine);
    assert!(st.boards.stale, "the old Board stays dim until the new view arrives");
    assert_eq!(st.status.as_ref().unwrap().text, "Board: only my Tickets. m shows all.");
    let mine = BoardView { only_mine: true, ..board_view(4166, "BCE board", &[("To Do", "10000", &["BCE-2"]), ("Done", "10001", &[])]) };
    update(&mut st, Event::BoardLoaded { id: 4166, generation: 2, result: Box::new(Ok(mine)) });
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-2"), "the selected card stays selected");
    assert!(draw(&mut st, 120, 40).contains("2 Tickets (only mine)") || draw(&mut st, 120, 40).contains("1 Tickets (only mine)"));

    // The save of `m` arrives: no second request.
    let s = Settings { boards_only_mine: true, ..st.settings.clone() };
    let a = update(&mut st, Event::SettingSaved { key: "boards.only_mine".into(), result: Ok(s) });
    assert!(!a.iter().any(|a| matches!(a, Action::FetchBoard { .. })), "{a:?}");

    // m again: all Tickets. The view with all Tickets is in the memory cache: no request.
    let a = update(&mut st, key('m'));
    assert_eq!(a.iter().filter(|a| matches!(a, Action::FetchBoard { .. })).count(), 0, "{a:?}");
    assert!(!st.boards.view.as_ref().unwrap().only_mine);
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-2"));
    assert_eq!(st.status.as_ref().unwrap().text, "Board: all Tickets.");
}

#[test]
fn only_mine_from_the_settings_page_loads_the_board_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    let s = Settings { boards_only_mine: true, ..st.settings.clone() };
    let a = update(&mut st, Event::SettingSaved { key: "boards.only_mine".into(), result: Ok(s) });
    assert!(a.contains(&Action::FetchBoard { id: 4166, generation: 2, only_mine: true }), "{a:?}");

    // At the start, the Setting gives the filter of the first request.
    let s = Settings { boards_only_mine: true, ..boards_settings(dir.path(), &[(4166, "BCE board")]) };
    let mut st = State::new(&s, 120, 40);
    assert_eq!(update(&mut st, key('2')), vec![Action::FetchBoard { id: 4166, generation: 1, only_mine: true }]);
}

#[test]
fn a_transition_moves_the_card_to_its_new_column() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    // BCE-1 is selected. After a Transition, the full Ticket has the Done status.
    let mut t = ticket_with_key("BCE-1");
    t.status = "Done".into();
    t.status_id = Some("10001".into());
    update(&mut st, Event::Ticket { key: "BCE-1".into(), result: Box::new(Ok(t)) });
    let v = st.boards.view.as_ref().unwrap();
    assert_eq!(v.columns[0].tickets.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["BCE-2"]);
    assert_eq!(v.columns[2].tickets.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["BCE-1", "BCE-3"]);
    // The selection follows the card.
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-1"));
    assert_eq!(st.boards.col, 2);
}

#[test]
fn a_missing_board_shows_not_found_and_x_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(9, "Gone"), (4166, "BCE board")]);
    update(&mut st, key('2'));
    update(&mut st, Event::BoardLoaded { id: 9, generation: 1, result: Box::new(Err(Error::NotFound("Board 9".into()))) });
    assert!(st.boards.missing.contains_key(&9));
    assert_eq!(st.my_boards.len(), 2, "jui does not remove a missing Board");
    let screen = draw(&mut st, 100, 12);
    assert!(screen.contains("Board 9 not found."), "{screen}");
    assert!(screen.contains("Push x to remove it from My Boards."));
    update(&mut st, key('x'));
    assert!(matches!(st.popup, Some(Popup::ConfirmRemoveBoard { id: 9, .. })));
    let a = update(&mut st, key('y'));
    let list = vec![BoardRef { id: 4166, name: Some("BCE board".into()) }];
    assert_eq!(a, vec![Action::SaveBoards { path: dir.path().join("config.toml"), list: list.clone() }]);
    let a = update(&mut st, Event::BoardsSaved { list, result: Ok(()) });
    assert_eq!(st.my_boards.len(), 1);
    assert_eq!(a, vec![Action::FetchBoard { id: 4166, generation: 2, only_mine: false }]);
    assert!(st.status.as_ref().unwrap().text.contains("Removed 1 Board from My Boards."));
}

#[test]
fn empty_my_boards_opens_the_picker_and_saves_the_marked_boards() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[]);
    let a = update(&mut st, key('2'));
    assert_eq!(a, vec![Action::FetchBoardSuggestions { project: "BCE".into() }]);
    let sugg = vec![
        Suggestion { board: Board { id: 4537, name: "PostOps".into(), kind: "scrum".into() }, origin: Origin::Recent },
        Suggestion { board: Board { id: 4166, name: "BCE board".into(), kind: "scrum".into() }, origin: Origin::Project },
    ];
    update(&mut st, Event::BoardSuggestions { result: Ok(sugg) });
    // Enter with nothing marked: no save, and the picker closes.
    update(&mut st, code(KeyCode::Down));
    update(&mut st, key(' '));
    let Some(Popup::Boards(p)) = &st.popup else { panic!() };
    assert!(p.is_marked(4166));
    let a = update(&mut st, code(KeyCode::Enter));
    let list = vec![BoardRef { id: 4166, name: Some("BCE board".into()) }];
    assert_eq!(a, vec![Action::SaveBoards { path: dir.path().join("config.toml"), list: list.clone() }]);
    let a = update(&mut st, Event::BoardsSaved { list, result: Ok(()) });
    assert!(st.popup.is_none());
    assert_eq!(a, vec![Action::FetchBoard { id: 4166, generation: 1, only_mine: false }]);
}

#[test]
fn picker_searches_the_server_after_the_delay_and_ignores_old_results() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    let a = update(&mut st, key('+'));
    assert_eq!(a, vec![Action::FetchBoardSuggestions { project: "BCE".into() }]);
    // One letter filters in memory. Two letters start a search after the delay.
    assert!(update(&mut st, key('c')).is_empty());
    assert_eq!(update(&mut st, key('a')), vec![Action::ScheduleBoardSearch { seq: 2 }]);
    assert_eq!(update(&mut st, key('s')), vec![Action::ScheduleBoardSearch { seq: 3 }]);
    assert!(update(&mut st, Event::BoardSearchDue { seq: 2 }).is_empty(), "an old delay does nothing");
    assert_eq!(update(&mut st, Event::BoardSearchDue { seq: 3 }), vec![Action::SearchBoards { text: "cas".into(), seq: 3 }]);
    update(&mut st, Event::BoardsFound { seq: 2, result: Ok(vec![]) });
    let Some(Popup::Boards(p)) = &st.popup else { panic!() };
    assert!(p.searching, "a late result is ignored");
    let found = vec![Board { id: 4749, name: "CAS Bugs".into(), kind: "kanban".into() }];
    update(&mut st, Event::BoardsFound { seq: 3, result: Ok(found) });
    update(&mut st, key(' '));
    let a = update(&mut st, code(KeyCode::Enter));
    let list = vec![BoardRef { id: 4166, name: Some("BCE board".into()) }, BoardRef { id: 4749, name: Some("CAS Bugs".into()) }];
    assert_eq!(a, vec![Action::SaveBoards { path: dir.path().join("config.toml"), list: list.clone() }]);
    // The shown Board stays: a new Board goes to the end of the list.
    let a = update(&mut st, Event::BoardsSaved { list, result: Ok(()) });
    assert!(a.is_empty());
    assert_eq!(st.boards.view.as_ref().unwrap().board.id, 4166);
}

#[test]
fn picker_esc_and_a_save_error_keep_my_boards() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    update(&mut st, key('+'));
    // The rows with no text: My Boards first (marked).
    let Some(Popup::Boards(p)) = &st.popup else { panic!() };
    assert_eq!(p.rows()[0].tag, "mine");
    update(&mut st, key(' '));
    update(&mut st, code(KeyCode::Esc));
    assert!(st.popup.is_none());
    assert_eq!(st.my_boards.len(), 1);
    update(&mut st, key('+'));
    update(&mut st, key(' '));
    let a = update(&mut st, code(KeyCode::Enter));
    assert!(matches!(&a[..], [Action::SaveBoards { list, .. }] if list.is_empty()));
    let e = Error::Settings { path: "/x".into(), message: "read-only".into() };
    update(&mut st, Event::BoardsSaved { list: vec![], result: Err(e) });
    let Some(Popup::Boards(p)) = &st.popup else { panic!("the picker stays open") };
    assert!(p.error.as_ref().unwrap().contains("read-only"));
    assert_eq!(st.my_boards.len(), 1);
}

#[test]
fn a_new_board_name_is_saved_with_no_change_on_the_screen() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "Old name")]);
    update(&mut st, key('2'));
    let a = update(&mut st, Event::BoardLoaded { id: 4166, generation: 1, result: Box::new(Ok(bce_board())) });
    let list = vec![BoardRef { id: 4166, name: Some("BCE board".into()) }];
    assert!(a.contains(&Action::SaveBoards { path: dir.path().join("config.toml"), list: list.clone() }));
    assert!(update(&mut st, Event::BoardsSaved { list, result: Ok(()) }).is_empty());
    assert!(st.status.is_none());
}

#[test]
fn snapshot_boards_tab_with_columns_and_preview() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board"), (4533, "Kanban")]);
    on_board(&mut st, bce_board());
    insta::assert_snapshot!(draw(&mut st, 100, 30));
}

#[test]
fn snapshot_board_picker() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    update(&mut st, key('+'));
    let sugg = vec![
        Suggestion { board: Board { id: 4537, name: "PostOps Stitch".into(), kind: "scrum".into() }, origin: Origin::Recent },
        Suggestion { board: Board { id: 4166, name: "BCE board".into(), kind: "scrum".into() }, origin: Origin::Project },
        Suggestion { board: Board { id: 4533, name: "BCE Kanban Board".into(), kind: "kanban".into() }, origin: Origin::Project },
    ];
    update(&mut st, Event::BoardSuggestions { result: Ok(sugg) });
    update(&mut st, code(KeyCode::Down));
    update(&mut st, key(' '));
    insta::assert_snapshot!(draw(&mut st, 100, 24));
}

#[test]
fn snapshot_boards_tab_empty() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[]);
    update(&mut st, key('2'));
    update(&mut st, code(KeyCode::Esc));
    insta::assert_snapshot!(draw(&mut st, 100, 10));
}

#[test]
fn board_popup_rows_have_no_duplicates() {
    let mut p = BoardPopup::new(&[BoardRef { id: 4166, name: Some("BCE board".into()) }]);
    p.suggestions = Some(vec![Suggestion { board: Board { id: 4166, name: "BCE board".into(), kind: "scrum".into() }, origin: Origin::Recent }]);
    let rows = p.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].board.kind, "scrum", "the type comes from the suggestion");
    assert!(!p.changed());
}

// ------------------------------------------------------------------ Sprints and Epics

use jui::core::agile::{Section, Target};
use jui::core::model::{Epic, Sprint, SprintBoard};
use jui::tui::state::{EpicChildren, EpicRow, SPRINT};

const BACKLOG_JQL: &str = "filter = 1 AND backlog";

fn sprint_board(sprints: &[(u64, &str)]) -> SprintBoard {
    SprintBoard {
        board: Board { id: 4166, name: "BCE board".into(), kind: "scrum".into() },
        sprints: sprints.iter().map(|(id, n)| SprintRef { id: *id, name: (*n).into(), state: "active".into() }).collect(),
        backlog_jql: Some(BACKLOG_JQL.into()),
    }
}

fn open_sprint(id: u64, name: &str, state: &str) -> Sprint {
    Sprint { id, name: name.into(), state: state.into(), start: None, end: None, complete: None, goal: None, board_id: Some(4166) }
}

fn epic(key: &str, name: &str) -> Epic {
    Epic { key: key.into(), name: name.into(), summary: name.into(), done: false }
}

/// Go to the Sprint tab (Board 4166 is on the Boards tab) and give it the Board.
fn on_sprint_tab(st: &mut State, b: SprintBoard) {
    let a = update(st, key('3'));
    let gen_ = match a.as_slice() {
        [Action::FetchSprintBoard { generation, .. }] => *generation,
        other => panic!("{other:?}"),
    };
    update(st, Event::SprintBoardLoaded { id: 4166, generation: gen_, result: Box::new(Ok(b)) });
}

fn sprint_list(st: &State) -> &jui::tui::state::TicketList {
    st.tabs[SPRINT].list.as_ref().unwrap()
}

#[test]
fn sprint_tab_uses_the_board_of_the_boards_tab_and_its_sprints() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    let a = update(&mut st, key('3'));
    let sprints = Some(vec![SprintRef { id: 7, name: "Sprint 21".into(), state: "active".into() }]);
    assert_eq!(a, vec![Action::FetchSprintBoard { id: 4166, generation: 1, sprints }], "the Boards tab gives the active Sprints");
    let a = update(&mut st, Event::SprintBoardLoaded { id: 4166, generation: 1, result: Box::new(Ok(sprint_board(&[(7, "Sprint 21")]))) });
    assert_eq!(a, vec![Action::FetchPage { tab: SPRINT, jql: "sprint = 7 ORDER BY Rank ASC".into(), start_at: 0, generation: 1 }]);
    assert_eq!(st.sprint.current(), Some(Section::Sprint(SprintRef { id: 7, name: "Sprint 21".into(), state: "active".into() })));
    // A second visit keeps the state.
    update(&mut st, key('1'));
    assert!(update(&mut st, key('3')).is_empty());
}

#[test]
fn m_on_the_sprint_tab_shows_only_my_tickets_in_each_part() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    on_sprint_tab(&mut st, sprint_board(&[(7, "Sprint 21")]));
    update(&mut st, Event::Page { tab: SPRINT, generation: 1, start_at: 0, result: page(0, 5, 5) });
    update(&mut st, key('j'));
    update(&mut st, key('j'));
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-2"));

    // m: save the Setting and get the same part again with the assignee.
    let mine = "(sprint = 7) AND assignee = currentUser() ORDER BY Rank ASC";
    let a = update(&mut st, key('m'));
    assert_eq!(
        a,
        vec![
            Action::SaveSetting { key: "sprint.only_mine".into(), value: Some("on".into()) },
            Action::FetchPage { tab: SPRINT, jql: mine.into(), start_at: 0, generation: 2 },
        ]
    );
    assert!(!st.boards.only_mine, "the Boards tab has its own filter");
    assert_eq!(st.status.as_ref().unwrap().text, "Sprint: only my Tickets. m shows all.");
    let rows = vec![row(2), row(4)];
    update(&mut st, Event::Page { tab: SPRINT, generation: 2, start_at: 0, result: Ok(Page { items: rows, start_at: 0, total: 2 }) });
    assert_eq!(st.selected_key_any().as_deref(), Some("BCE-2"), "the selected Ticket stays selected");
    assert!(draw(&mut st, 120, 20).contains("only mine"));

    // The backlog also has the filter.
    let a = update(&mut st, key(']'));
    let backlog_mine = format!("({BACKLOG_JQL}) AND assignee = currentUser()");
    assert!(matches!(a.as_slice(), [Action::FetchPage { jql, .. }] if *jql == backlog_mine), "{a:?}");

    // The save of `m` arrives: no second request. m again: all Tickets.
    let s = Settings { sprint_only_mine: true, ..st.settings.clone() };
    let a = update(&mut st, Event::SettingSaved { key: "sprint.only_mine".into(), result: Ok(s) });
    assert!(!a.iter().any(|a| matches!(a, Action::FetchPage { .. })), "{a:?}");
    let a = update(&mut st, key('m'));
    assert!(a.iter().any(|a| matches!(a, Action::FetchPage { jql, .. } if jql == BACKLOG_JQL)), "{a:?}");
}

#[test]
fn sprint_only_mine_from_the_settings_is_used_at_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let s = Settings { sprint_only_mine: true, ..boards_settings(dir.path(), &[(4166, "BCE board")]) };
    let mut st = State::new(&s, 120, 40);
    on_board(&mut st, bce_board());
    update(&mut st, key('3'));
    let a = update(&mut st, Event::SprintBoardLoaded { id: 4166, generation: 1, result: Box::new(Ok(sprint_board(&[(7, "Sprint 21")]))) });
    assert_eq!(
        a,
        vec![Action::FetchPage { tab: SPRINT, jql: "(sprint = 7) AND assignee = currentUser() ORDER BY Rank ASC".into(), start_at: 0, generation: 1 }]
    );
}

#[test]
fn sprint_tab_with_empty_my_boards_shows_a_message() {
    let mut st = state(100, 10);
    let a = update(&mut st, key('3'));
    assert!(a.is_empty());
    let screen = draw(&mut st, 100, 10);
    assert!(screen.contains("My Boards is empty. Push 2, then + to select a Scrum Board."), "{screen}");
}

#[test]
fn brackets_change_between_sprints_and_the_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    on_sprint_tab(&mut st, sprint_board(&[(7, "Sprint 21"), (8, "Sprint 21b")]));
    update(&mut st, Event::Page { tab: SPRINT, generation: 1, start_at: 0, result: page(0, 5, 5) });
    update(&mut st, key('j'));
    let a = update(&mut st, key(']'));
    assert!(matches!(a.as_slice(), [Action::FetchPage { jql, generation: 2, .. }] if jql == "sprint = 8 ORDER BY Rank ASC"), "{a:?}");
    assert_eq!(sprint_list(&st).selected, 0, "a new part selects the first row");
    let a = update(&mut st, key(']'));
    assert!(matches!(a.as_slice(), [Action::FetchPage { jql, .. }] if jql == BACKLOG_JQL), "{a:?}");
    // A late page of the old part is ignored.
    update(&mut st, Event::Page { tab: SPRINT, generation: 2, start_at: 0, result: page(0, 2, 2) });
    assert!(sprint_list(&st).rows.len() == 5 && sprint_list(&st).stale);
    update(&mut st, key(']'));
    assert_eq!(st.sprint.section, 0, "] goes round to the first Sprint");
    update(&mut st, key('['));
    assert_eq!(st.sprint.current(), Some(Section::Backlog));
}

#[test]
fn refresh_on_the_sprint_tab_gets_the_sprints_again_and_keeps_the_part() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    on_sprint_tab(&mut st, sprint_board(&[(7, "Sprint 21")]));
    update(&mut st, key(']'));
    update(&mut st, Event::Page { tab: SPRINT, generation: 2, start_at: 0, result: page(0, 5, 5) });
    update(&mut st, key('j'));
    let a = update(&mut st, key('r'));
    assert_eq!(a, vec![Action::FetchSprintBoard { id: 4166, generation: 2, sprints: None }]);
    // The Sprint ended: only the backlog stays. The selected part and row stay.
    let a = update(&mut st, Event::SprintBoardLoaded { id: 4166, generation: 2, result: Box::new(Ok(sprint_board(&[]))) });
    assert_eq!(st.sprint.current(), Some(Section::Backlog));
    assert!(matches!(a.as_slice(), [Action::FetchPage { jql, .. }] if jql == BACKLOG_JQL));
    assert_eq!(sprint_list(&st).keep_key.as_deref(), Some("BCE-1"));
}

#[test]
fn s_opens_the_sprint_popup_and_moves_the_ticket() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    update(&mut st, Event::Ticket { key: "BCE-1".into(), result: Box::new(Ok(ticket_with_key("BCE-1"))) });
    update(&mut st, code(KeyCode::Enter));
    assert!(update(&mut st, key('s')).contains(&Action::FetchOpenSprints { board_id: 4166, key: "BCE-1".into() }));
    let list = vec![open_sprint(9, "Future", "future"), open_sprint(43198, "BCE-26-Sprint-21", "active"), open_sprint(1, "Old", "closed")];
    update(&mut st, Event::OpenSprints { board_id: 4166, key: "BCE-1".into(), result: Ok(list) });
    let Some(Popup::Sprint(p)) = &st.popup else { panic!() };
    let rows = p.rows();
    assert_eq!(rows.len(), 3, "active, future, then the backlog; the closed Sprint is removed");
    assert!(matches!(&rows[0], Target::Sprint(s) if s.id == 43198));
    assert_eq!(rows[2], Target::Backlog);
    // The Ticket is already in Sprint 43198 (the fixture): Enter gives an error.
    update(&mut st, code(KeyCode::Enter));
    let Some(Popup::Sprint(p)) = &st.popup else { panic!() };
    assert!(p.error.as_deref().unwrap().contains("already in Sprint BCE-26-Sprint-21"));
    update(&mut st, key('b'));
    update(&mut st, key('a'));
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::MoveSprint { key: "BCE-1".into(), target: Target::Backlog }]);
    let a = update(&mut st, Event::SprintMoved { key: "BCE-1".into(), target: Target::Backlog, result: Ok(()) });
    assert!(st.popup.is_none());
    assert!(a.contains(&Action::FetchTicket { key: "BCE-1".into() }));
    assert_eq!(st.status.as_ref().unwrap().text, "Moved BCE-1 to the backlog.");
    // The second popup uses the Sprints in memory: no request.
    let a = update(&mut st, key('s'));
    assert!(a.is_empty(), "{a:?}");
}

#[test]
fn a_sprint_move_error_keeps_the_popup_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, key('s'));
    update(&mut st, Event::OpenSprints { board_id: 4166, key: "BCE-1".into(), result: Ok(vec![open_sprint(9, "Future", "future")]) });
    let a = update(&mut st, code(KeyCode::Enter));
    let target = Target::Sprint(open_sprint(9, "Future", "future"));
    assert_eq!(a, vec![Action::MoveSprint { key: "BCE-1".into(), target: target.clone() }]);
    let e = Error::Validation { fields: Default::default(), messages: vec!["Issue BCE-1 is a subtask.".into()] };
    update(&mut st, Event::SprintMoved { key: "BCE-1".into(), target, result: Err(e) });
    let Some(Popup::Sprint(p)) = &st.popup else { panic!("the popup stays open") };
    assert!(!p.sending);
    assert_eq!(p.error.as_deref(), Some("Issue BCE-1 is a subtask."));
}

#[test]
fn s_with_no_board_tells_the_user_to_add_one() {
    let mut st = loaded(3, 3);
    update(&mut st, code(KeyCode::Enter));
    assert!(update(&mut st, key('s')).is_empty());
    assert!(st.status.as_ref().unwrap().text.contains("My Boards is empty"));
}

#[test]
fn e_opens_the_epic_picker_searches_and_sets_the_epic() {
    let mut st = loaded(3, 3);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    let a = update(&mut st, Event::Key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT)));
    assert_eq!(a, vec![Action::SearchEpics { key: "BCE-0".into(), project: "BCE".into(), text: String::new(), seq: 0 }]);
    update(&mut st, Event::Epics { key: "BCE-0".into(), seq: 0, result: Ok(vec![epic("BCE-47921", "Scanner reliability"), epic("BCE-2", "Two")]) });
    let Some(Popup::Epic(p)) = &st.popup else { panic!() };
    assert_eq!(p.rows().len(), 3, "none, then the open Epics");
    // The current Epic (BCE-47921 in the fixture) gives an error.
    update(&mut st, code(KeyCode::Down));
    update(&mut st, code(KeyCode::Enter));
    let Some(Popup::Epic(p)) = &st.popup else { panic!() };
    assert!(p.error.as_deref().unwrap().contains("already in BCE-47921"));
    // Two letters start a search after the delay. An old result is ignored.
    update(&mut st, key('c'));
    let a = update(&mut st, key('i'));
    assert_eq!(a, vec![Action::ScheduleEpicSearch { seq: 2 }]);
    let a = update(&mut st, Event::EpicSearchDue { seq: 2 });
    assert_eq!(a, vec![Action::SearchEpics { key: "BCE-0".into(), project: "BCE".into(), text: "ci".into(), seq: 2 }]);
    update(&mut st, Event::Epics { key: "BCE-0".into(), seq: 1, result: Ok(vec![epic("BCE-9", "Old")]) });
    let Some(Popup::Epic(p)) = &st.popup else { panic!() };
    assert!(p.results.is_none() && p.searching);
    update(&mut st, Event::Epics { key: "BCE-0".into(), seq: 2, result: Ok(vec![epic("BCE-13070", "Cider")]) });
    let Some(Popup::Epic(p)) = &st.popup else { panic!() };
    assert_eq!(p.rows(), vec![EpicRow::Epic(epic("BCE-13070", "Cider"))]);
    let a = update(&mut st, code(KeyCode::Enter));
    let set = Action::SetEpic { key: "BCE-0".into(), epic: Some(epic("BCE-13070", "Cider")), old: Some("BCE-47921".into()) };
    assert_eq!(a, vec![set]);
    let a = update(&mut st, Event::EpicSet { key: "BCE-0".into(), epic: Some(epic("BCE-13070", "Cider")), old: Some("BCE-47921".into()), result: Ok(()) });
    assert!(st.popup.is_none());
    assert!(a.contains(&Action::FetchTicket { key: "BCE-0".into() }));
    assert_eq!(st.status.as_ref().unwrap().text, "Set the Epic of BCE-0 to BCE-13070 (Cider).");
}

#[test]
fn e_none_removes_the_epic_and_an_epic_cannot_get_an_epic() {
    let mut st = loaded(3, 3);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT)));
    update(&mut st, key('n'));
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::SetEpic { key: "BCE-0".into(), epic: None, old: Some("BCE-47921".into()) }]);
    update(&mut st, code(KeyCode::Esc));
    let mut e = ticket_with_key("BCE-1");
    e.kind = "Epic".into();
    update(&mut st, Event::Ticket { key: "BCE-1".into(), result: Box::new(Ok(e)) });
    update(&mut st, code(KeyCode::Esc));
    update(&mut st, key('j'));
    update(&mut st, code(KeyCode::Enter));
    let a = update(&mut st, Event::Key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT)));
    assert!(a.is_empty());
    assert!(st.status.as_ref().unwrap().text.contains("An Epic cannot be in an Epic"));
}

#[test]
fn the_zoom_of_an_epic_gets_and_shows_its_tickets() {
    let mut st = loaded(3, 3);
    let mut e = ticket_with_key("BCE-0");
    e.kind = "Epic".into();
    e.subtasks.clear();
    e.links.clear();
    e.comments.clear();
    e.comment_total = 0;
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(e)) });
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchEpicChildren { key: "BCE-0".into() }]);
    assert_eq!(st.epic_children.get("BCE-0"), Some(&EpicChildren::Loading));
    let kids = Page { items: vec![card("BCE-11"), card("BCE-12")], start_at: 0, total: 2 };
    update(&mut st, Event::EpicChildren { key: "BCE-0".into(), result: Ok(kids) });
    update(&mut st, key('j'));
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-12".into() }], "Enter opens a child Ticket");
    update(&mut st, code(KeyCode::Esc));
    let screen = draw(&mut st, 100, 30);
    assert!(screen.contains("Tickets in this Epic (2)"), "{screen}");
    assert!(screen.contains("BCE-11"));
    // `r` gets the Ticket and its child Tickets again.
    update(&mut st, key('r'));
    assert!(!st.epic_children.contains_key("BCE-0"));
}

#[test]
fn n_in_the_zoom_of_an_epic_creates_a_ticket_in_the_epic() {
    let mut st = loaded(3, 3);
    let mut e = ticket_with_key("BCE-0");
    e.kind = "Epic".into();
    e.subtasks.clear();
    e.links.clear();
    e.comments.clear();
    e.comment_total = 0;
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(e)) });
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::EpicChildren { key: "BCE-0".into(), result: Ok(Page { items: vec![card("BCE-11")], start_at: 0, total: 1 }) });
    assert!(draw(&mut st, 120, 30).contains("n new Ticket in this Epic"));

    let a = update(&mut st, key('n'));
    let [Action::PrepareCreate { kind, project, issue_type }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(kind, &TemplateKind::InEpic { epic: "BCE-0".into() });
    assert_eq!((project.as_str(), issue_type.as_str()), ("BCE", "Task"), "the default project and type");
    update(&mut st, Event::CreateReady { kind: kind.clone(), result: Box::new(Ok((task_meta(), "shacohen".into()))) });
    let p = create_form(&mut st);
    assert_eq!(p.title, "Create a Task in BCE (Epic BCE-0)");
    let epic = p.fields.iter().position(|f| f.id == "customfield_10016").unwrap();
    assert_eq!(p.inputs[epic], "BCE-0", "the Epic is set");

    typed(&mut st, "New");
    update(&mut st, code(KeyCode::Tab));
    typed(&mut st, "DCO-Flow");
    let a = update(&mut st, ctrl('s'));
    let [Action::SendCreate { body }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(body["fields"]["customfield_10016"], "BCE-0");

    let a = update(&mut st, Event::Created { result: Ok("BCE-99".into()) });
    assert_eq!(st.status.as_ref().unwrap().text, "Created BCE-99 in BCE-0.");
    assert!(a.contains(&Action::FetchEpicChildren { key: "BCE-0".into() }), "the zoom of the Epic gets the new Ticket: {a:?}");
}

#[test]
fn n_in_the_zoom_of_a_ticket_that_is_not_an_epic_is_a_plain_create() {
    let mut st = loaded(3, 3);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    let a = update(&mut st, key('n'));
    assert!(matches!(a.as_slice(), [Action::PrepareCreate { kind: TemplateKind::Create, .. }]), "{a:?}");
}

#[test]
fn snapshot_sprint_tab_and_sprint_popup() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = boards_state(dir.path(), &[(4166, "BCE board")]);
    on_board(&mut st, bce_board());
    on_sprint_tab(&mut st, sprint_board(&[(7, "Sprint 21")]));
    update(&mut st, Event::Page { tab: SPRINT, generation: 1, start_at: 0, result: page(0, 4, 4) });
    insta::assert_snapshot!("sprint_tab", draw(&mut st, 100, 12));
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, key('s'));
    update(&mut st, Event::OpenSprints {
        board_id: 4166,
        key: "BCE-0".into(),
        result: Ok(vec![open_sprint(43198, "BCE-26-Sprint-21", "active"), open_sprint(43199, "BCE-26-Sprint-22", "future")]),
    });
    insta::assert_snapshot!("sprint_popup", draw(&mut st, 100, 20));
}

#[test]
fn snapshot_epic_picker() {
    let mut st = loaded(3, 3);
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Key(KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT)));
    update(&mut st, Event::Epics { key: "BCE-0".into(), seq: 0, result: Ok(vec![epic("BCE-47921", "Scanner reliability"), epic("BCE-13070", "Proxy - Cider Integration MVP")]) });
    insta::assert_snapshot!("epic_picker", draw(&mut st, 100, 20));
}

// ---------------------------------------------------------------- Links and Worklogs

use jui::core::worklog::Clock;
use jui::tui::state::{DeleteKind, WorklogStart};

fn la(n: chrono::NaiveDateTime) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    use chrono::TimeZone;
    chrono::FixedOffset::west_opt(7 * 3600).unwrap().from_local_datetime(&n).single()
}

fn fixed_clock() -> Clock {
    Clock { now: chrono::NaiveDate::from_ymd_opt(2026, 10, 19).unwrap().and_hms_opt(11, 0, 0).unwrap(), local: la }
}

fn link_types() -> Vec<model::LinkType> {
    let t = |id: &str, name: &str, inward: &str, outward: &str| model::LinkType { id: id.into(), name: name.into(), inward: inward.into(), outward: outward.into() };
    vec![t("10000", "Blocks", "is blocked by", "blocks"), t("10003", "Relates", "relates to", "relates to"), t("10002", "Duplicate", "is duplicated by", "duplicates")]
}

/// The zoom of BCE-0 with the item `n` selected (0 Sub-task, 1-2 Links, 3 Worklog, 4-5 comments).
fn zoom_on_item(n: usize) -> State {
    let mut st = loaded(5, 5);
    st.clock = Some(fixed_clock());
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(ticket_with_key("BCE-0"))) });
    update(&mut st, code(KeyCode::Enter));
    for _ in 0..n {
        update(&mut st, key('j'));
    }
    st
}

fn sent_worklog(id: &str) -> model::Worklog {
    model::Worklog {
        id: id.into(),
        author: Some(user("me", "Me")),
        started: chrono::DateTime::parse_from_rfc3339("2026-10-19T10:00:00-07:00").ok(),
        updated: None,
        time_spent: "1h 30m".into(),
        time_spent_seconds: 5400,
        comment: None,
    }
}

#[test]
fn l_selects_a_relation_and_adds_the_link() {
    let mut st = zoom_on_item(0);
    let a = update(&mut st, key('l'));
    assert_eq!(a, vec![Action::FetchLinkTypes]);
    update(&mut st, Event::LinkTypes { result: Ok(link_types()) });
    // "is bl" filters to one relation; Enter moves to the key input.
    type_text(&mut st, "is bl");
    let Some(Popup::Link(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(p.shown(st.link_types.as_ref().unwrap()).len(), 1);
    update(&mut st, code(KeyCode::Enter));
    type_text(&mut st, "70000");
    let a = update(&mut st, code(KeyCode::Enter));
    let [Action::AddLink { plan }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(plan.describe(), "BCE-0 is blocked by BCE-70000");
    assert_eq!(plan.sides(), ("BCE-70000", "BCE-0"));
    let a = update(&mut st, Event::LinkAdded { plan: plan.clone(), result: Ok(()) });
    assert!(st.popup.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Added the Link: BCE-0 is blocked by BCE-70000.");
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }]);
    // The second `l` does not get the Link types again.
    assert!(update(&mut st, key('l')).is_empty());
}

#[test]
fn l_with_an_existing_link_or_a_bad_key_sends_nothing() {
    let mut st = zoom_on_item(0);
    st.link_types = Some(link_types());
    update(&mut st, key('l'));
    update(&mut st, code(KeyCode::Enter)); // "blocks" is the first relation.
    type_text(&mut st, "BCE-63001");
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    assert!(st.popup.is_none());
    assert!(st.status.as_ref().unwrap().text.contains("The Link exists"));

    update(&mut st, key('l'));
    update(&mut st, code(KeyCode::Enter));
    type_text(&mut st, "x y");
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    let Some(Popup::Link(p)) = &st.popup else { panic!() };
    assert_eq!(p.error.as_deref(), Some("`x y` is not a Ticket key."));
    // A failed request keeps the popup open with the error.
    let mut p = p.clone();
    p.target = "70000".into();
    st.popup = Some(Popup::Link(p));
    let a = update(&mut st, code(KeyCode::Enter));
    let [Action::AddLink { plan }] = a.as_slice() else { panic!("{a:?}") };
    let e = Error::Validation { fields: Default::default(), messages: vec!["No permission.".into()] };
    update(&mut st, Event::LinkAdded { plan: plan.clone(), result: Err(e) });
    let Some(Popup::Link(p)) = &st.popup else { panic!() };
    assert!(!p.sending);
    assert!(p.error.as_deref().unwrap().contains("No permission."), "{:?}", p.error);
}

#[test]
fn enter_on_a_link_opens_it_and_d_deletes_it() {
    let mut st = zoom_on_item(2);
    st.cache.put(ticket_with_key("BCE-61200"));
    update(&mut st, key('d'));
    let Some(Popup::ConfirmDelete { what, id, preview, .. }) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!((*what, id.as_str(), preview.as_str()), (DeleteKind::Link, "502", "BCE-0 relates to BCE-61200"));
    let a = update(&mut st, key('y'));
    assert_eq!(a, vec![Action::DeleteLink { key: "BCE-0".into(), id: "502".into() }]);
    let a = update(&mut st, Event::LinkDeleted { key: "BCE-0".into(), id: "502".into(), result: Ok(()) });
    assert_eq!(st.status.as_ref().unwrap().text, "Deleted Link 502 on BCE-0.");
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }]);
    assert!(!st.cache.contains("BCE-61200"), "the other Ticket also changed");
}

#[test]
fn big_l_popup_adds_a_worklog() {
    let mut st = zoom_on_item(0);
    assert!(update(&mut st, key('L')).is_empty());
    type_text(&mut st, "1 hour");
    update(&mut st, code(KeyCode::Tab));
    type_text(&mut st, "10:00");
    update(&mut st, code(KeyCode::Tab));
    type_text(&mut st, "Review of **PR**");
    // A bad time shows below the field; nothing is sent.
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    let Some(Popup::Worklog(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert!(p.errors[0].as_deref().unwrap().contains("is not a time"));
    assert_eq!(p.focus, 0);
    for _ in 0..6 {
        update(&mut st, code(KeyCode::Backspace));
    }
    type_text(&mut st, "1h30m");
    let a = update(&mut st, code(KeyCode::Enter));
    let [Action::SendWorklog { key: k, plan }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(k, "BCE-0");
    assert_eq!(plan.body, serde_json::json!({"timeSpentSeconds": 5400, "started": "2026-10-19T10:00:00.000-0700", "comment": "Review of *PR*"}));
    let a = update(&mut st, Event::WorklogSent { key: "BCE-0".into(), id: None, result: Box::new(Ok(sent_worklog("302"))) });
    assert!(st.popup.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Added Worklog 302 to BCE-0: 1h 30m on 2026-10-19 10:00.");
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }]);
}

#[test]
fn big_l_validation_error_stays_in_the_popup_and_ctrl_e_opens_the_template() {
    let mut st = zoom_on_item(0);
    update(&mut st, key('L'));
    type_text(&mut st, "2h");
    update(&mut st, code(KeyCode::Enter));
    let e = Error::Validation { fields: [("timeLogged".to_string(), "Too much.".to_string())].into(), messages: vec![] };
    update(&mut st, Event::WorklogSent { key: "BCE-0".into(), id: None, result: Box::new(Err(e)) });
    let Some(Popup::Worklog(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(p.errors[0].as_deref(), Some("Too much."));
    assert_eq!(p.inputs[0], "2h", "the values of the user stay");
    // Ctrl-E moves the values to the Worklog Template.
    let a = update(&mut st, Event::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)));
    let [Action::PrepareWorklog { key: k, start: WorklogStart::Add(q) }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!((k.as_str(), q.time_spent.as_str()), ("BCE-0", "2h"));
}

fn worklog_ctx() -> model::WorklogContext {
    let t = full_ticket();
    model::WorklogContext { key: "BCE-0".into(), summary: t.summary.clone(), worklogs: t.worklogs.clone(), ..Default::default() }
}

#[test]
fn e_on_a_worklog_edits_it_and_shows_the_comment_diff() {
    let mut st = zoom_on_item(4);
    let a = update(&mut st, key('e'));
    assert_eq!(a, vec![Action::PrepareWorklog { key: "BCE-0".into(), start: WorklogStart::Edit { id: "300".into() } }]);
    let a = update(&mut st, Event::WorklogReady { key: "BCE-0".into(), start: WorklogStart::Edit { id: "300".into() }, result: Box::new(Ok(worklog_ctx())) });
    let text = editor_text(&a);
    assert!(text.contains("time_spent = \"1h 30m\""), "{text}");
    // A change of the time only: no diff, jui sends after the recheck.
    let a = update(&mut st, Event::Edited { result: Ok(text.replace("\"1h 30m\"", "\"2h\"")) });
    assert_eq!(a, vec![Action::RecheckWorklog { key: "BCE-0".into(), id: "300".into() }]);
    let now = full_ticket().worklogs[0].clone();
    let a = update(&mut st, Event::WorklogRechecked { key: "BCE-0".into(), id: "300".into(), result: Box::new(Ok(now.clone())) });
    let [Action::SendWorklog { plan, .. }] = a.as_slice() else { panic!("{a:?}") };
    assert_eq!(plan.seconds, 7200);
    update(&mut st, Event::WorklogSent { key: "BCE-0".into(), id: Some("300".into()), result: Box::new(Ok(now.clone())) });
    assert_eq!(st.status.as_ref().unwrap().text, "Saved Worklog 300 on BCE-0: 2h on 2026-10-12 10:00.");
    assert!(st.worklog_job.is_none());

    // A change of the comment shows the diff popup.
    update(&mut st, key('e'));
    let a = update(&mut st, Event::WorklogReady { key: "BCE-0".into(), start: WorklogStart::Edit { id: "300".into() }, result: Box::new(Ok(worklog_ctx())) });
    let text = editor_text(&a);
    update(&mut st, Event::Edited { result: Ok(text.replacen("\nInvestigation", "\nRoot cause found", 1)) });
    update(&mut st, Event::WorklogRechecked { key: "BCE-0".into(), id: "300".into(), result: Box::new(Ok(now)) });
    let Some(Popup::Diff(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(p.subject, "The comment of Worklog 300");
    assert!(p.lines.iter().any(|l| l.tag == '+' && l.text == "Root cause found"), "{:?}", p.lines);
    let a = update(&mut st, key('y'));
    assert!(matches!(a.as_slice(), [Action::SendWorklog { plan, .. }] if plan.comment == "Root cause found"), "{a:?}");
}

#[test]
fn a_worklog_that_changed_in_jira_shows_the_conflict_and_esc_keeps_a_draft() {
    let mut st = zoom_on_item(4);
    update(&mut st, key('e'));
    let a = update(&mut st, Event::WorklogReady { key: "BCE-0".into(), start: WorklogStart::Edit { id: "300".into() }, result: Box::new(Ok(worklog_ctx())) });
    update(&mut st, Event::Edited { result: Ok(editor_text(&a).replace("\"1h 30m\"", "\"2h\"")) });
    let mut now = full_ticket().worklogs[0].clone();
    now.time_spent_seconds = 600;
    update(&mut st, Event::WorklogRechecked { key: "BCE-0".into(), id: "300".into(), result: Box::new(Ok(now)) });
    let Some(Popup::Diff(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(p.conflict.as_deref(), Some("Worklog 300 on BCE-0 changed in Jira while you edited it."));
    let a = update(&mut st, code(KeyCode::Esc));
    assert!(matches!(a.as_slice(), [Action::WriteDraft { command, .. }] if command == "worklog edit BCE-0 300"), "{a:?}");
    assert!(st.worklog_job.is_none());
}

#[test]
fn d_on_a_worklog_asks_and_deletes() {
    let mut st = zoom_on_item(4);
    update(&mut st, key('d'));
    let Some(Popup::ConfirmDelete { what, id, preview, .. }) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!((*what, id.as_str()), (DeleteKind::Worklog, "300"));
    assert_eq!(preview, "2026-10-12 10:00  1h 30m  @jdoe  Investigation");
    let a = update(&mut st, key('y'));
    assert_eq!(a, vec![Action::DeleteWorklog { key: "BCE-0".into(), id: "300".into() }]);
    update(&mut st, Event::WorklogDeleted { key: "BCE-0".into(), id: "300".into(), result: Ok(()) });
    assert_eq!(st.status.as_ref().unwrap().text, "Deleted Worklog 300 on BCE-0.");
    assert_eq!(st.tabs[0].zoom[0].item, 3);
}

#[test]
fn snapshot_link_popup() {
    let mut st = zoom_on_item(2);
    st.link_types = Some(link_types());
    update(&mut st, key('l'));
    type_text(&mut st, "bl");
    update(&mut st, code(KeyCode::Tab));
    type_text(&mut st, "700");
    insta::assert_snapshot!(draw(&mut st, 100, 40));
}

#[test]
fn snapshot_worklog_popup_with_error() {
    let mut st = zoom_on_item(4);
    update(&mut st, key('L'));
    type_text(&mut st, "abc");
    update(&mut st, code(KeyCode::Enter));
    insta::assert_snapshot!(draw(&mut st, 100, 40));
}

// ------------------------------------------------------------------ attachments

fn added_attachment(id: &str, name: &str) -> model::Attachment {
    model::Attachment { id: id.into(), filename: name.into(), size: 5, mime_type: None, author: None, created: None, url: None }
}

#[test]
fn u_popup_uploads_the_path_and_refreshes() {
    let mut st = zoom_on_item(0);
    assert!(update(&mut st, key('u')).is_empty());
    // Enter with no path sends nothing.
    assert!(update(&mut st, code(KeyCode::Enter)).is_empty());
    let Some(Popup::Upload(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!(p.error.as_deref(), Some("Type or paste the path of a file."));
    type_text(&mut st, "'/tmp/a b.txt'");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::Upload { key: "BCE-0".into(), path: "'/tmp/a b.txt'".into() }]);
    // Keys wait while the upload runs.
    update(&mut st, key('x'));
    let Some(Popup::Upload(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert!(p.sending && p.input == "'/tmp/a b.txt'");
    let a = update(&mut st, Event::Uploaded { key: "BCE-0".into(), result: Ok(vec![added_attachment("950", "a b.txt")]) });
    assert!(st.popup.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Uploaded a b.txt to BCE-0.");
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }]);
}

#[test]
fn a_bad_path_stays_in_the_u_popup_and_ctrl_u_clears_it() {
    let mut st = zoom_on_item(0);
    update(&mut st, key('u'));
    type_text(&mut st, "/nope");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Uploaded { key: "BCE-0".into(), result: Err(Error::input("The file /nope does not exist.")) });
    let Some(Popup::Upload(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert!(!p.sending);
    assert_eq!(p.error.as_deref(), Some("The file /nope does not exist."));
    assert_eq!(st.errors.len(), 1, "the ! popup also has the error");
    update(&mut st, ctrl('u'));
    let Some(Popup::Upload(p)) = &st.popup else { panic!("{:?}", st.popup) };
    assert!(p.input.is_empty() && p.error.is_none());
    update(&mut st, code(KeyCode::Esc));
    assert!(st.popup.is_none());
}

#[test]
fn an_upload_after_esc_reports_in_the_status_line() {
    let mut st = zoom_on_item(0);
    update(&mut st, key('u'));
    type_text(&mut st, "/tmp/x");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, code(KeyCode::Esc));
    assert!(st.popup.is_none());
    update(&mut st, Event::Uploaded { key: "BCE-0".into(), result: Err(Error::Server { status: 500, body: "boom".into() }) });
    assert!(st.status.as_ref().unwrap().text.contains("boom"));
}

#[test]
fn enter_on_an_attachment_opens_it_and_d_deletes_it() {
    let mut st = zoom_on_item(3);
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::OpenBrowser { url: "https://jira.example/secure/attachment/900/trace.log".into() }]);
    assert_eq!(st.tabs[0].zoom.len(), 1, "the zoom does not change");
    update(&mut st, key('d'));
    let Some(Popup::ConfirmDelete { what, id, preview, .. }) = &st.popup else { panic!("{:?}", st.popup) };
    assert_eq!((*what, id.as_str(), preview.as_str()), (DeleteKind::Attachment, "900", "trace.log  2 KB  @jdoe  2026-10-11"));
    assert!(update(&mut st, key('n')).is_empty());
    assert!(st.popup.is_none());
    update(&mut st, key('d'));
    let a = update(&mut st, key('y'));
    assert_eq!(a, vec![Action::DeleteAttachment { key: "BCE-0".into(), id: "900".into() }]);
    let a = update(&mut st, Event::AttachmentDeleted { key: "BCE-0".into(), id: "900".into(), result: Ok(()) });
    assert_eq!(st.status.as_ref().unwrap().text, "Deleted attachment 900 on BCE-0.");
    assert_eq!(a, vec![Action::FetchTicket { key: "BCE-0".into() }]);
    assert_eq!(st.tabs[0].zoom[0].item, 2);
}

#[test]
fn a_failed_attachment_delete_shows_the_error() {
    let mut st = zoom_on_item(3);
    update(&mut st, key('d'));
    update(&mut st, key('y'));
    update(&mut st, Event::AttachmentDeleted { key: "BCE-0".into(), id: "900".into(), result: Err(Error::Forbidden("No permission.".into())) });
    assert!(st.popup.is_none());
    assert!(st.status.as_ref().unwrap().text.contains("No permission."));
}

#[test]
fn snapshot_upload_popup_on_a_selected_attachment() {
    let mut st = zoom_on_item(3);
    update(&mut st, key('u'));
    type_text(&mut st, "~/Desktop/screen.png");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::Uploaded { key: "BCE-0".into(), result: Err(Error::input("~/Desktop/screen.png has 60.0 MB. The upload limit of Jira is 50.0 MB.")) });
    insta::assert_snapshot!(draw(&mut st, 100, 40));
}

// ------------------------------------------------------------------ Settings page

use jui::core::settings::Source;
use jui::core::settings::PAGE;
use jui::tui::state::{SETTINGS, SettingsRow};

fn settings_state() -> State {
    let mut s = settings();
    s.path = "/tmp/jui-test/config.toml".into();
    s.my_boards = vec![BoardRef { id: 1, name: Some("One".into()) }, BoardRef { id: 2, name: Some("Two".into()) }];
    s.sources = vec![("tickets.default_project", Source::File), ("my_boards", Source::File)];
    let mut st = State::new(&s, 120, 30);
    st.now = Some("2026-10-19T12:00:00Z".parse::<DateTime<Utc>>().unwrap());
    init(&mut st);
    update(&mut st, key('6'));
    assert_eq!(st.tab, SETTINGS);
    st
}

/// Move the selection to the row of a key.
fn select_setting(st: &mut State, k: &str) {
    let i = st
        .settings_rows()
        .iter()
        .position(|r| matches!(r, SettingsRow::Value(m) if m.key == k))
        .unwrap();
    st.settings_page.selected = i;
}

/// The Settings that the runner reads after a save.
fn saved_settings(st: &State, change: impl FnOnce(&mut Settings)) -> Settings {
    let mut s = st.settings.clone();
    change(&mut s);
    s
}

#[test]
fn the_settings_tab_makes_no_requests() {
    let mut st = state(120, 30);
    init(&mut st);
    let a = update(&mut st, key('6'));
    assert!(a.is_empty(), "{a:?}");
}

#[test]
fn settings_rows_have_all_settings_then_my_boards() {
    let st = settings_state();
    let rows = st.settings_rows();
    let n = PAGE.len();
    assert_eq!(rows.len(), n + 2 + 1);
    assert!(matches!(rows[n], SettingsRow::Board(0)));
    assert!(matches!(rows[n + 2], SettingsRow::AddBoards));
}

#[test]
fn j_and_k_move_and_ticket_keys_do_nothing_on_the_settings_page() {
    let mut st = settings_state();
    update(&mut st, key('j'));
    update(&mut st, key('j'));
    assert_eq!(st.settings_page.selected, 2);
    update(&mut st, key('k'));
    assert_eq!(st.settings_page.selected, 1);
    update(&mut st, key('G'));
    assert_eq!(st.settings_page.selected, PAGE.len() + 2);
    for c in ['t', 'a', 'c', 'w', 'o', 'y'] {
        let a = update(&mut st, key(c));
        assert!(a.is_empty(), "{c}: {a:?}");
        assert!(st.popup.is_none(), "{c}");
    }
}

#[test]
fn a_text_setting_is_checked_and_saved() {
    let mut st = settings_state();
    select_setting(&mut st, "jira.base_url");
    update(&mut st, code(KeyCode::Enter));
    let Some(Popup::Setting(p)) = &st.popup else { panic!("no popup") };
    assert_eq!(p.input, "https://jira.example");
    // A URL with no https:// stays in the popup with an error.
    update(&mut st, ctrl('u'));
    type_text(&mut st, "jira.other");
    let a = update(&mut st, code(KeyCode::Enter));
    assert!(a.is_empty());
    let Some(Popup::Setting(p)) = &st.popup else { panic!("the popup closed") };
    assert!(p.error.as_deref().unwrap().contains("https://"), "{:?}", p.error);
    // A valid URL is saved.
    update(&mut st, ctrl('u'));
    type_text(&mut st, "https://jira.other/");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::SaveSetting { key: "jira.base_url".into(), value: Some("https://jira.other".into()) }]);
    let Some(Popup::Setting(p)) = &st.popup else { panic!() };
    assert!(p.saving);
}

#[test]
fn a_new_base_url_makes_a_new_client_and_loads_my_tickets_again() {
    let mut st = settings_state();
    select_setting(&mut st, "jira.base_url");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, ctrl('u'));
    type_text(&mut st, "https://jira.other");
    update(&mut st, code(KeyCode::Enter));
    let s = saved_settings(&st, |s| {
        s.base_url = "https://jira.other".into();
        s.sources.push(("jira.base_url", Source::File));
    });
    let a = update(&mut st, Event::SettingSaved { key: "jira.base_url".into(), result: Ok(s) });
    assert!(st.popup.is_none());
    assert!(a.contains(&Action::UseConnection {
        base_url: "https://jira.other".into(),
        timeout_secs: 30,
        editor_command: String::new()
    }));
    assert!(a.iter().any(|a| matches!(a, Action::FetchPage { tab: MY_TICKETS, start_at: 0, .. })), "{a:?}");
    assert_eq!(st.browse_url("BCE-1"), "https://jira.other/browse/BCE-1");
    assert!(st.status.as_ref().unwrap().text.starts_with("Saved Base URL to /tmp/jui-test/config.toml"));
}

#[test]
fn a_new_my_tickets_jql_loads_the_tab_again() {
    let mut st = settings_state();
    select_setting(&mut st, "tickets.my_tickets_jql");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, ctrl('u'));
    type_text(&mut st, "assignee = jdoe");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::SaveSetting { key: "tickets.my_tickets_jql".into(), value: Some("assignee = jdoe".into()) }]);
    let s = saved_settings(&st, |s| s.my_tickets_jql = "assignee = jdoe".into());
    let a = update(&mut st, Event::SettingSaved { key: "tickets.my_tickets_jql".into(), result: Ok(s) });
    assert!(a.contains(&Action::FetchPage { tab: MY_TICKETS, jql: "assignee = jdoe".into(), start_at: 0, generation: 2 }), "{a:?}");
    assert!(!a.iter().any(|a| matches!(a, Action::UseConnection { .. })));
}

#[test]
fn a_number_accepts_only_digits() {
    let mut st = settings_state();
    select_setting(&mut st, "jira.timeout_secs");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, ctrl('u'));
    update(&mut st, key('x'));
    let Some(Popup::Setting(p)) = &st.popup else { panic!() };
    assert_eq!(p.input, "");
    assert_eq!(p.error.as_deref(), Some("Type only digits."));
    type_text(&mut st, "0");
    update(&mut st, code(KeyCode::Enter));
    let Some(Popup::Setting(p)) = &st.popup else { panic!() };
    assert!(p.error.as_deref().unwrap().contains("larger than 0"), "{:?}", p.error);
    update(&mut st, code(KeyCode::Backspace));
    let a = type_text(&mut st, "60");
    assert!(a.is_empty());
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::SaveSetting { key: "jira.timeout_secs".into(), value: Some("60".into()) }]);
}

#[test]
fn a_choice_uses_a_list_and_starts_on_the_current_value() {
    let mut st = settings_state();
    select_setting(&mut st, "display.date_format");
    update(&mut st, code(KeyCode::Enter));
    let Some(Popup::Setting(p)) = &st.popup else { panic!() };
    assert_eq!(p.shown(), vec!["relative", "iso", "local"]);
    assert_eq!(p.selected, 0);
    update(&mut st, code(KeyCode::Down));
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::SaveSetting { key: "display.date_format".into(), value: Some("iso".into()) }]);
    // A filter that matches nothing gives an error.
    st.popup = None;
    update(&mut st, code(KeyCode::Enter));
    type_text(&mut st, "zz");
    let a = update(&mut st, code(KeyCode::Enter));
    assert!(a.is_empty());
    let Some(Popup::Setting(p)) = &st.popup else { panic!() };
    assert!(p.error.is_some());
}

#[test]
fn the_same_value_is_not_saved_again() {
    let mut st = settings_state();
    select_setting(&mut st, "tickets.default_project");
    update(&mut st, code(KeyCode::Enter));
    let a = update(&mut st, code(KeyCode::Enter));
    assert!(a.is_empty(), "{a:?}");
    assert!(st.popup.is_none());
    assert_eq!(st.status.as_ref().unwrap().text, "Default project did not change.");
}

#[test]
fn a_project_is_checked_on_the_server_before_the_save() {
    let mut st = settings_state();
    select_setting(&mut st, "tickets.default_project");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, ctrl('u'));
    type_text(&mut st, "nope");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::CheckProject { input: "NOPE".into() }]);
    // Keys wait during the check.
    assert!(update(&mut st, key('x')).is_empty());
    update(&mut st, Event::ProjectChecked { input: "NOPE".into(), result: Err(Error::NotFound("project NOPE".into())) });
    let Some(Popup::Setting(p)) = &st.popup else { panic!() };
    assert_eq!(p.error.as_deref(), Some("Project NOPE does not exist, or you cannot see it."));
    assert!(!p.checking);

    update(&mut st, ctrl('u'));
    type_text(&mut st, "xsup");
    update(&mut st, code(KeyCode::Enter));
    let a = update(&mut st, Event::ProjectChecked { input: "XSUP".into(), result: Ok("XSUP".into()) });
    assert_eq!(a, vec![Action::SaveSetting { key: "tickets.default_project".into(), value: Some("XSUP".into()) }]);
}

#[test]
fn a_late_project_check_after_esc_is_ignored() {
    let mut st = settings_state();
    select_setting(&mut st, "tickets.default_project");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, ctrl('u'));
    type_text(&mut st, "xsup");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, code(KeyCode::Esc));
    assert!(st.popup.is_none());
    let a = update(&mut st, Event::ProjectChecked { input: "XSUP".into(), result: Ok("XSUP".into()) });
    assert!(a.is_empty(), "no save after Esc: {a:?}");
}

#[test]
fn the_ticket_type_popup_gets_the_types_of_the_default_project() {
    let mut st = settings_state();
    select_setting(&mut st, "tickets.default_type");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchTicketTypes { project: "BCE".into() }]);
    update(&mut st, Event::TicketTypes { project: "BCE".into(), result: Ok(vec!["Bug".into(), "Task".into(), "Story".into()]) });
    let Some(Popup::Setting(p)) = &st.popup else { panic!() };
    assert_eq!(p.selected, 1, "the current type is selected");
    type_text(&mut st, "bu");
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::SaveSetting { key: "tickets.default_type".into(), value: Some("Bug".into()) }]);
    // The second time, the types come from memory.
    st.popup = None;
    let a = update(&mut st, code(KeyCode::Enter));
    assert!(a.is_empty());
}

#[test]
fn d_removes_the_key_and_says_when_it_is_the_default() {
    let mut st = settings_state();
    select_setting(&mut st, "display.color");
    let a = update(&mut st, key('d'));
    assert!(a.is_empty());
    assert_eq!(st.status.as_ref().unwrap().text, "Colors (CLI) already uses the default value.");

    select_setting(&mut st, "tickets.default_project");
    let a = update(&mut st, key('d'));
    assert_eq!(a, vec![Action::SaveSetting { key: "tickets.default_project".into(), value: None }]);
    let s = saved_settings(&st, |s| s.sources.retain(|(k, _)| *k != "tickets.default_project"));
    update(&mut st, Event::SettingSaved { key: "tickets.default_project".into(), result: Ok(s) });
    assert!(st.status.as_ref().unwrap().text.starts_with("Default project uses the default value."));
}

#[test]
fn a_failed_save_stays_in_the_popup() {
    let mut st = settings_state();
    select_setting(&mut st, "editor.command");
    update(&mut st, code(KeyCode::Enter));
    type_text(&mut st, "hx");
    update(&mut st, code(KeyCode::Enter));
    let e = Error::Settings { path: "/tmp/jui-test/config.toml".into(), message: "Permission denied".into() };
    update(&mut st, Event::SettingSaved { key: "editor.command".into(), result: Err(e) });
    let Some(Popup::Setting(p)) = &st.popup else { panic!() };
    assert!(!p.saving);
    assert!(p.error.as_deref().unwrap().contains("Permission denied"));
}

#[test]
fn a_new_editor_command_is_used_at_once() {
    let mut st = settings_state();
    let s = saved_settings(&st, |s| s.editor_command = "hx".into());
    let a = update(&mut st, Event::SettingSaved { key: "editor.command".into(), result: Ok(s) });
    assert_eq!(
        a,
        vec![Action::UseConnection { base_url: "https://jira.example".into(), timeout_secs: 30, editor_command: "hx".into() }]
    );
}

#[test]
fn enter_on_the_my_boards_rows_opens_the_board_picker() {
    let mut st = settings_state();
    update(&mut st, key('G'));
    let a = update(&mut st, code(KeyCode::Enter));
    assert_eq!(a, vec![Action::FetchBoardSuggestions { project: "BCE".into() }]);
    assert!(matches!(st.popup, Some(Popup::Boards(_))));
}

#[test]
fn big_k_and_big_j_change_the_order_of_my_boards() {
    let mut st = settings_state();
    st.settings_page.selected = PAGE.len() + 1; // Board 2.
    let a = update(&mut st, key('K'));
    let order = vec![BoardRef { id: 2, name: Some("Two".into()) }, BoardRef { id: 1, name: Some("One".into()) }];
    assert_eq!(a, vec![Action::SaveBoards { path: "/tmp/jui-test/config.toml".into(), list: order.clone() }]);
    assert_eq!(st.settings_page.selected, PAGE.len(), "the selection follows the Board");
    // At the top, K does nothing.
    assert!(update(&mut st, key('K')).is_empty());
    let a = update(&mut st, Event::BoardsSaved { list: order.clone(), result: Ok(()) });
    assert!(a.is_empty(), "no Board request from the Settings page: {a:?}");
    assert_eq!(st.my_boards, order);
    assert_eq!(st.settings.my_boards, order);
    assert_eq!(st.status.as_ref().unwrap().text, "Saved the order of My Boards.");
    // K and J on a Setting row only show a message.
    st.settings_page.selected = 0;
    assert!(update(&mut st, key('J')).is_empty());
}

#[test]
fn d_on_a_board_row_asks_and_removes_the_board() {
    let mut st = settings_state();
    st.settings_page.selected = PAGE.len() + 1;
    update(&mut st, key('d'));
    assert!(matches!(st.popup, Some(Popup::ConfirmRemoveBoard { id: 2, .. })));
    let a = update(&mut st, key('y'));
    let left = vec![BoardRef { id: 1, name: Some("One".into()) }];
    assert_eq!(a, vec![Action::SaveBoards { path: "/tmp/jui-test/config.toml".into(), list: left.clone() }]);
    update(&mut st, Event::BoardsSaved { list: left, result: Ok(()) });
    assert_eq!(st.settings_page.selected, PAGE.len() + 1, "the selection stays in the rows");
    assert!(matches!(st.selected_setting(), Some(SettingsRow::AddBoards)));
}

#[test]
fn big_e_opens_the_file_and_checks_the_text() {
    let mut st = settings_state();
    let a = update(&mut st, key('E'));
    assert_eq!(a, vec![Action::EditConfig { original: None, text: None }]);
    assert!(st.job_running());
    // No change.
    let a = update(&mut st, Event::ConfigEdited { original: "[jira]\n".into(), result: Ok("[jira]\n".into()) });
    assert!(a.is_empty());
    assert!(!st.job_running());

    // A syntax error opens a popup: e edits the same text again.
    update(&mut st, key('E'));
    update(&mut st, Event::ConfigEdited { original: "[jira]\n".into(), result: Ok("[jira\n".into()) });
    assert!(matches!(st.popup, Some(Popup::ConfigInvalid(_))));
    let a = update(&mut st, key('e'));
    assert_eq!(a, vec![Action::EditConfig { original: Some("[jira]\n".into()), text: Some("[jira\n".into()) }]);
    // A valid text is saved; the new Settings have an effect.
    let a = update(&mut st, Event::ConfigEdited { original: "[jira]\n".into(), result: Ok("[display]\ndate_format = \"iso\"\n".into()) });
    assert_eq!(a, vec![Action::SaveConfigText { text: "[display]\ndate_format = \"iso\"\n".into() }]);
    let s = saved_settings(&st, |s| s.date_format = jui::core::settings::DateFormat::Iso);
    update(&mut st, Event::ConfigSaved { result: Ok(s) });
    assert!(!st.job_running());
    assert_eq!(st.date_format, jui::core::settings::DateFormat::Iso);
}

#[test]
fn big_e_then_q_discards_the_changes() {
    let mut st = settings_state();
    update(&mut st, key('E'));
    update(&mut st, Event::ConfigEdited { original: String::new(), result: Ok("bad = = 1".into()) });
    let a = update(&mut st, key('q'));
    assert!(a.is_empty());
    assert!(st.popup.is_none());
    assert!(!st.job_running());
    assert_eq!(st.status.as_ref().unwrap().text, "The config file did not change.");
}

#[test]
fn r_reads_the_file_again() {
    let mut st = settings_state();
    assert_eq!(update(&mut st, key('r')), vec![Action::ReloadSettings]);
    let mut s = st.settings.clone();
    s.warnings = vec!["Unknown Setting `jira.base_ulr`. Check the spelling.".into()];
    update(&mut st, Event::SettingsReloaded { result: Ok(s) });
    assert!(st.status.as_ref().unwrap().text.contains("base_ulr"));
}

#[test]
fn snapshot_settings_page() {
    let mut st = settings_state();
    let mut s = st.settings.clone();
    s.base_url = "http://127.0.0.1:9".into();
    s.file_base_url = Some("https://jira.example".into());
    s.sources.push(("jira.base_url", Source::Env));
    s.editor_command = "code --wait".into();
    s.sources.push(("editor.command", Source::File));
    st.apply_settings(s);
    st.base_url = "http://127.0.0.1:9".into();
    st.status = None;
    insta::assert_snapshot!(draw(&mut st, 110, 32));
}

#[test]
fn snapshot_settings_project_popup_with_error() {
    let mut st = settings_state();
    select_setting(&mut st, "tickets.default_project");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, ctrl('u'));
    type_text(&mut st, "nope");
    update(&mut st, code(KeyCode::Enter));
    update(&mut st, Event::ProjectChecked { input: "NOPE".into(), result: Err(Error::NotFound("project NOPE".into())) });
    insta::assert_snapshot!(draw(&mut st, 100, 30));
}

// ------------------------------------------------------------------ My Tickets by Sprint

fn sprint(id: u64, state: &str) -> Option<model::SprintRef> {
    Some(model::SprintRef { id, name: format!("BCE-26-Sprint-{id}"), state: state.into() })
}

/// A row with a Sprint.
fn row_in(n: usize, s: Option<model::SprintRef>) -> ListRow {
    let mut r = row(n);
    r.ticket.sprint = s;
    r
}

/// The TUI with the grouping on, and the first page of My Tickets.
fn grouped(items: Vec<ListRow>, total: u64) -> State {
    let s = Settings { group_by_sprint: true, ..settings() };
    let mut st = State::new(&s, 120, 30);
    st.now = Some("2026-10-19T12:00:00Z".parse::<DateTime<Utc>>().unwrap());
    init(&mut st);
    update(&mut st, Event::Page { tab: MY_TICKETS, generation: 1, start_at: 0, result: Ok(Page { items, start_at: 0, total }) });
    st
}

fn keys(st: &State) -> Vec<String> {
    list(st).rows.iter().map(|r| r.ticket.key.clone()).collect()
}

#[test]
fn my_tickets_rows_are_in_sprint_groups() {
    // Query order: 0 (none), 1 (future 22), 2 (active 21), 3 (none), 4 (active 21).
    let items = vec![row_in(0, None), row_in(1, sprint(22, "future")), row_in(2, sprint(21, "active")), row_in(3, None), row_in(4, sprint(21, "active"))];
    let st = grouped(items, 5);
    assert_eq!(keys(&st), ["BCE-2", "BCE-4", "BCE-1", "BCE-0", "BCE-3"]);
    let titles: Vec<String> = list(&st).sprint_groups().iter().map(|g| g.title()).collect();
    assert_eq!(titles, ["BCE-26-Sprint-21 (active)", "BCE-26-Sprint-22 (future)", "No Sprint"]);
    // The first row of the first group is selected, and its Ticket is fetched.
    assert_eq!(list(&st).selected_key(), Some("BCE-2"));
    // j and k move over Tickets only: the heading rows are not selectable.
    let mut st = st;
    update(&mut st, key('j'));
    update(&mut st, key('j'));
    assert_eq!(list(&st).selected_key(), Some("BCE-1"));
}

#[test]
fn a_new_page_adds_rows_to_their_groups_and_keeps_the_selection() {
    let first: Vec<ListRow> = (0..100).map(|n| row_in(n, if n < 10 { sprint(21, "active") } else { None })).collect();
    let mut st = grouped(first, 150);
    update(&mut st, key('G'));
    assert_eq!(list(&st).selected_key(), Some("BCE-99"));
    // The next page has more Tickets of the active Sprint. They go above the selection.
    let items: Vec<ListRow> = (100..150).map(|n| row_in(n, if n < 105 { sprint(21, "active") } else { None })).collect();
    update(&mut st, Event::Page { tab: MY_TICKETS, generation: 1, start_at: 100, result: Ok(Page { items, start_at: 100, total: 150 }) });
    assert_eq!(list(&st).rows.len(), 150);
    assert_eq!(list(&st).selected_key(), Some("BCE-99"), "the selected Ticket stays selected");
    let groups = list(&st).sprint_groups();
    assert_eq!(groups[0].range, 0..15);
    assert_eq!(list(&st).rows[14].ticket.key, "BCE-104");
    assert_eq!(list(&st).count().as_deref(), Some("150"));
}

#[test]
fn a_ticket_that_moves_to_a_sprint_moves_to_its_group() {
    let mut st = grouped(vec![row_in(0, sprint(21, "active")), row_in(1, None), row_in(2, None)], 3);
    update(&mut st, key('j'));
    update(&mut st, key('j'));
    assert_eq!(list(&st).selected_key(), Some("BCE-2"));
    // The full Ticket says BCE-2 is in the active Sprint now (for example after `s`).
    let mut t = ticket_with_key("BCE-2");
    t.sprint = sprint(21, "active");
    update(&mut st, Event::Ticket { key: "BCE-2".into(), result: Box::new(Ok(t)) });
    assert_eq!(keys(&st), ["BCE-0", "BCE-2", "BCE-1"]);
    assert_eq!(list(&st).selected_key(), Some("BCE-2"), "the selection follows the Ticket");

    // A full Ticket with no Sprint data (the Agile request failed) keeps the Sprint of the row.
    let mut t = ticket_with_key("BCE-0");
    t.sprint = None;
    update(&mut st, Event::Ticket { key: "BCE-0".into(), result: Box::new(Ok(t)) });
    assert_eq!(keys(&st), ["BCE-0", "BCE-2", "BCE-1"]);
}

#[test]
fn group_by_sprint_off_loads_the_query_order_again_and_on_groups_at_once() {
    let items = vec![row_in(0, None), row_in(1, sprint(21, "active"))];
    let mut st = grouped(items, 2);
    st.settings.path = "/tmp/jui-test/config.toml".into();
    assert_eq!(keys(&st), ["BCE-1", "BCE-0"]);

    // Off: the rows must come back in the query order, so jui gets the first page again.
    let s = Settings { group_by_sprint: false, ..st.settings.clone() };
    let a = update(&mut st, Event::SettingSaved { key: "tickets.group_by_sprint".into(), result: Ok(s) });
    assert!(a.iter().any(|a| matches!(a, Action::FetchPage { tab: MY_TICKETS, start_at: 0, .. })), "{a:?}");
    update(&mut st, Event::Page { tab: MY_TICKETS, generation: 2, start_at: 0, result: Ok(Page { items: vec![row_in(0, None), row_in(1, sprint(21, "active"))], start_at: 0, total: 2 }) });
    assert_eq!(keys(&st), ["BCE-0", "BCE-1"]);
    assert!(list(&st).sprint_groups().is_empty());

    // On: the rows have the Sprint data, so no request is necessary.
    let s = Settings { group_by_sprint: true, ..st.settings.clone() };
    let a = update(&mut st, Event::SettingSaved { key: "tickets.group_by_sprint".into(), result: Ok(s) });
    assert!(!a.iter().any(|a| matches!(a, Action::FetchPage { .. })), "{a:?}");
    assert_eq!(keys(&st), ["BCE-1", "BCE-0"]);
}

#[test]
fn snapshot_my_tickets_in_sprint_groups() {
    let items = vec![
        row_in(0, None),
        row_in(1, sprint(22, "future")),
        row_in(2, sprint(21, "active")),
        row_in(3, None),
        row_in(4, sprint(21, "active")),
    ];
    let mut st = grouped(items, 9);
    update(&mut st, key('j'));
    insta::assert_snapshot!(draw(&mut st, 120, 16));
}

// ------------------------------------------------------------------ Epics tab

use jui::tui::state::EPICS;

fn epic_row(key: &str, done: bool) -> ListRow {
    let mut r = row(1);
    r.ticket.key = key.into();
    r.ticket.kind = "Epic".into();
    r.ticket.summary = format!("Epic {key}");
    if done {
        r.ticket.status = "Done".into();
        r.ticket.status_category = model::StatusCategory::Done;
    }
    r
}

fn epic_counts() -> Vec<(String, usize)> {
    vec![("BCE-7".into(), 2), ("CRTX-9".into(), 1)]
}

/// Open the Epics tab and give it the counts. Returns the actions of the counts event.
fn epics_tab(st: &mut State) -> Vec<Action> {
    let a = update(st, key('4'));
    assert_eq!(st.tab, EPICS);
    assert_eq!(a, vec![Action::FetchMyEpicCounts { generation: 1 }]);
    update(st, Event::MyEpicCounts { generation: 1, result: Ok(epic_counts()) })
}

#[test]
fn the_epics_tab_gets_the_counts_then_the_epics() {
    let mut st = loaded(3, 3);
    let a = epics_tab(&mut st);
    assert_eq!(a, vec![Action::FetchPage { tab: EPICS, jql: "key in (BCE-7, CRTX-9) ORDER BY updated DESC".into(), start_at: 0, generation: 1 }]);
    assert_eq!(st.epics.counts.get("BCE-7"), Some(&2));
    // Come back later: no new request.
    update(&mut st, key('1'));
    assert!(update(&mut st, key('4')).is_empty());
}

#[test]
fn r_on_the_epics_tab_gets_the_counts_again_and_a_late_response_is_ignored() {
    let mut st = loaded(3, 3);
    epics_tab(&mut st);
    update(&mut st, Event::Page { tab: EPICS, generation: 1, start_at: 0, result: Ok(Page { items: vec![epic_row("BCE-7", false)], start_at: 0, total: 1 }) });
    let a = update(&mut st, key('r'));
    assert_eq!(a, vec![Action::FetchMyEpicCounts { generation: 2 }]);
    assert!(update(&mut st, Event::MyEpicCounts { generation: 1, result: Ok(vec![]) }).is_empty());
    assert!(st.epics.loading);
}

#[test]
fn no_epics_shows_a_message_and_makes_no_search() {
    let mut st = loaded(3, 3);
    update(&mut st, key('4'));
    let a = update(&mut st, Event::MyEpicCounts { generation: 1, result: Ok(vec![]) });
    assert!(a.is_empty(), "{a:?}");
    assert!(draw(&mut st, 100, 8).contains("None of your open Tickets is in an Epic."));
}

#[test]
fn a_counts_error_marks_the_epics_tab_offline() {
    let mut st = loaded(3, 3);
    update(&mut st, key('4'));
    let e = Error::Network { host: "jira.example".into(), cause: "refused".into() };
    update(&mut st, Event::MyEpicCounts { generation: 1, result: Err(e) });
    assert!(!st.epics.loading);
    assert!(st.tabs[EPICS].list.as_ref().unwrap().offline);
}

#[test]
fn snapshot_epics_tab() {
    let mut st = loaded(3, 3);
    epics_tab(&mut st);
    let items = vec![epic_row("BCE-7", false), epic_row("CRTX-9", true)];
    update(&mut st, Event::Page { tab: EPICS, generation: 1, start_at: 0, result: Ok(Page { items, start_at: 0, total: 2 }) });
    insta::assert_snapshot!(draw(&mut st, 120, 10));
}
