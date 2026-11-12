//! `jui sprint ...` and `jui epic ...`. See ticket 23 "Build: Sprints and Epics".

use serde::Serialize;

use crate::core::agile;
use crate::core::model::{Epic, Sprint, SprintRef, TicketSummary};

/// The `--json` shape of a Sprint and its Tickets.
#[derive(Debug, Serialize)]
pub struct SprintTickets<S: Serialize> {
    pub sprint: S,
    pub total: u64,
    pub tickets: Vec<TicketSummary>,
}

/// One line of a Sprint date: `2026-10-12`.
fn day(d: Option<chrono::DateTime<chrono::FixedOffset>>) -> String {
    d.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "-".into())
}

/// The text of `jui sprint list`: ID, state, start, end, and name. One line each.
pub fn render_sprints(sprints: &[Sprint]) -> String {
    let id_w = sprints.iter().map(|s| s.id.to_string().len()).max().unwrap_or(2).max(2);
    let mut out = format!("{:<id_w$}  {:<6}  {:<10}  {:<10}  Name\n", "ID", "State", "Start", "End");
    for s in sprints {
        out.push_str(&format!("{:<id_w$}  {:<6}  {:<10}  {:<10}  {}\n", s.id, s.state, day(s.start), day(s.end), s.name));
    }
    out.trim_end().to_string()
}

/// The head of `jui sprint view`.
pub fn render_sprint_head(s: &Sprint) -> String {
    let mut out = format!("{}  {}\n", s.id, s.name);
    out.push_str(&format!("State      {}\n", s.state));
    out.push_str(&format!("Dates      {} to {}\n", day(s.start), day(s.end)));
    if s.complete.is_some() {
        out.push_str(&format!("Completed  {}\n", day(s.complete)));
    }
    if let Some(g) = &s.goal {
        out.push_str(&format!("Goal       {}\n", g.replace('\n', " ")));
    }
    if let Some(b) = s.board_id {
        out.push_str(&format!("Board      {b}\n"));
    }
    out.trim_end().to_string()
}

/// The text of `jui epic list`: key, name, and (with `--done`) the Done mark.
pub fn render_epics(epics: &[Epic], show_done: bool) -> String {
    let key_w = epics.iter().map(|e| e.key.len()).max().unwrap_or(3).max(3);
    let mut out = String::new();
    for e in epics {
        let mark = if show_done && e.done { "  (done)" } else { "" };
        out.push_str(&format!("{:<key_w$}  {}{mark}\n", e.key, e.name));
    }
    out.trim_end().to_string()
}

/// One row of `jui epic mine`: the Epic, and the number of open Tickets of the user in it.
#[derive(Debug, Clone, Serialize)]
pub struct MyEpicRow {
    #[serde(flatten)]
    pub epic: Epic,
    pub my_open_tickets: usize,
}

/// The rows of `jui epic mine`, in the order of the counts (the most Tickets first). An Epic
/// that the search did not give (the user cannot see it) is not in the rows.
pub fn my_epic_rows(epics: Vec<Epic>, counts: &[(String, usize)]) -> Vec<MyEpicRow> {
    let mut by_key: std::collections::HashMap<String, Epic> = epics.into_iter().map(|e| (e.key.clone(), e)).collect();
    counts.iter().filter_map(|(k, n)| by_key.remove(k).map(|epic| MyEpicRow { epic, my_open_tickets: *n })).collect()
}

/// The text of `jui epic mine`: key, the number of my open Tickets, and the name.
pub fn render_my_epics(rows: &[MyEpicRow]) -> String {
    let key_w = rows.iter().map(|r| r.epic.key.len()).max().unwrap_or(3).max(3);
    let mut out = String::new();
    for r in rows {
        let mark = if r.epic.done { "  (done)" } else { "" };
        out.push_str(&format!("{:<key_w$}  {:>3}  {}{mark}\n", r.epic.key, r.my_open_tickets, r.epic.name));
    }
    out.trim_end().to_string()
}

/// The line on stderr before the Tickets of an active Sprint.
pub fn sprint_title(s: &SprintRef, shown: usize, total: u64) -> String {
    let count = if (shown as u64) < total { format!("{shown} of {total}") } else { total.to_string() };
    format!("Sprint {} ({}): {count} Tickets", s.name, s.id)
}

/// Remove duplicate keys and keep the order.
pub fn unique(keys: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in keys {
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// "Moved BCE-1, BCE-2 to Sprint BCE-26-Sprint-21."
pub fn moved_message(keys: &[String], target: &agile::Target) -> String {
    format!("Moved {} to {}.", agile::key_list(keys), target.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::parse_jira_date;

    fn sprint(id: u64, name: &str, state: &str) -> Sprint {
        Sprint {
            id,
            name: name.into(),
            state: state.into(),
            start: parse_jira_date("2026-10-12T01:07:00.000-0700"),
            end: parse_jira_date("2026-10-25T00:07:00.000-0700"),
            complete: None,
            goal: Some("Ship it".into()),
            board_id: Some(4166),
        }
    }

    #[test]
    fn my_epics_keep_the_count_order_and_mark_done() {
        let e = |k: &str, done| Epic { key: k.into(), name: format!("Name {k}"), summary: String::new(), done };
        let counts = vec![("BCE-2".to_string(), 3), ("GONE-1".to_string(), 2), ("CRTX-10".to_string(), 1)];
        let rows = my_epic_rows(vec![e("CRTX-10", true), e("BCE-2", false)], &counts);
        assert_eq!(rows.iter().map(|r| (r.epic.key.as_str(), r.my_open_tickets)).collect::<Vec<_>>(), [("BCE-2", 3), ("CRTX-10", 1)]);
        assert_eq!(render_my_epics(&rows), "BCE-2      3  Name BCE-2\nCRTX-10    1  Name CRTX-10  (done)");
        let v = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(v["key"], "BCE-2");
        assert_eq!(v["my_open_tickets"], 3);
    }

    #[test]
    fn sprint_text() {
        let text = render_sprints(&[sprint(43198, "BCE-26-Sprint-21", "active"), Sprint { start: None, end: None, ..sprint(43199, "BCE-26-Sprint-22", "future") }]);
        assert_eq!(
            text,
            "ID     State   Start       End         Name\n\
             43198  active  2026-10-12  2026-10-25  BCE-26-Sprint-21\n\
             43199  future  -           -           BCE-26-Sprint-22"
        );
        let head = render_sprint_head(&sprint(43198, "BCE-26-Sprint-21", "active"));
        assert!(head.starts_with("43198  BCE-26-Sprint-21\nState      active\nDates      2026-10-12 to 2026-10-25\nGoal       Ship it"), "{head}");
    }

    #[test]
    fn epic_text_and_messages() {
        let e = |k: &str, n: &str, done| Epic { key: k.into(), name: n.into(), summary: String::new(), done };
        assert_eq!(render_epics(&[e("BCE-1", "One", false), e("BCE-1000", "Two", true)], true), "BCE-1     One\nBCE-1000  Two  (done)");
        assert_eq!(unique(vec!["A-1".into(), "A-2".into(), "A-1".into()]), vec!["A-1", "A-2"]);
        let s = SprintRef { id: 7, name: "S7".into(), state: "active".into() };
        assert_eq!(sprint_title(&s, 50, 686), "Sprint S7 (7): 50 of 686 Tickets");
        assert_eq!(moved_message(&["A-1".into()], &agile::Target::Backlog), "Moved A-1 to the backlog.");
    }
}
