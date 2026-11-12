//! Sprints and Epics: the pure rules. See ticket 23 "Build: Sprints and Epics".
//!
//! The Agile issue endpoints are slow on this server (7 to 12 s for 100 Tickets). jui uses
//! `/rest/api/2/search` with JQL that gives the same Tickets in the same order. A check on
//! the real server found these rules:
//! - Sprint Tickets: `sprint = <id> ORDER BY Rank ASC` (Sprint 43198: 686 Tickets, same order).
//! - Epic Tickets: `"Epic Link" = <key> ORDER BY Rank ASC` (BCE-13070: 17 Tickets, same order).
//! - Backlog of a Scrum Board: the Board filter, no open or future Sprint, no Epics, and the
//!   statuses of all columns except the last column (Boards 4166 and 4537: same Tickets).
//! - Epics of a Board: the Board filter, the type Epic, and an Epic Status that is not Done
//!   (Board 4166: 2131 Epics, same order). The Agile Epic list needs 43 pages of 2 to 3 s.
//!
//! This module has no I/O.

use super::error::{Error, Result};
use super::model::{BoardConfig, Sprint, SprintBoard, SprintRef};

/// The Agile write endpoints accept at most this number of Tickets in one request.
pub const MOVE_CHUNK: usize = 50;
/// The page size of the Epic picker search.
pub const EPIC_PICK_MAX: u64 = 50;

/// The JQL of the Tickets of a Sprint.
pub fn sprint_jql(id: u64) -> String {
    format!("sprint = {id} ORDER BY Rank ASC")
}

/// The same query with only the Tickets that are assigned to the current user. The
/// condition goes before `ORDER BY`, so the order does not change.
pub fn only_mine(jql: &str) -> String {
    const MINE: &str = "assignee = currentUser()";
    let lower = jql.to_lowercase();
    let (body, order) = match lower.rfind("order by") {
        Some(i) => (jql[..i].trim(), Some(jql[i..].trim())),
        None => (jql.trim(), None),
    };
    let body = if body.is_empty() { MINE.to_string() } else { format!("({body}) AND {MINE}") };
    match order {
        Some(o) => format!("{body} {o}"),
        None => body,
    }
}

// ------------------------------------------------------------ my Epics

/// The Tickets that make the user a part of an Epic: assigned to the user, not Done, and in
/// an Epic. The Epic Link of each Ticket gives the Epic.
pub const MY_EPIC_TICKETS_JQL: &str = "assignee = currentUser() AND statusCategory != Done AND \"Epic Link\" is not EMPTY";

/// Count the open Tickets of the user in each Epic. The input is the Epic Link of each
/// Ticket. The output has the most Tickets first, then the Epic keys in alphabetical order.
pub fn count_by_epic<'a>(links: impl IntoIterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for l in links {
        let l = l.trim();
        if !l.is_empty() {
            *counts.entry(l).or_default() += 1;
        }
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// The JQL of the Epics of the user: the Epics by key, the most recently updated first.
/// `None` when the user has no open Ticket in an Epic.
pub fn my_epics_jql(counts: &[(String, usize)]) -> Option<String> {
    if counts.is_empty() {
        return None;
    }
    let keys: Vec<&str> = counts.iter().map(|(k, _)| k.as_str()).collect();
    Some(format!("key in ({}) ORDER BY updated DESC", keys.join(", ")))
}

/// The JQL of the Tickets of an Epic.
pub fn epic_jql(key: &str) -> String {
    format!("\"Epic Link\" = {key} ORDER BY Rank ASC")
}

/// The JQL of the backlog of a Scrum Board. `None` when the Board has no filter.
pub fn backlog_jql(c: &BoardConfig) -> Option<String> {
    let f = c.filter_id.as_deref()?;
    let mut parts = vec![format!("filter = {f}")];
    if let Some(q) = c.sub_query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        parts.push(format!("({q})"));
    }
    parts.push("(sprint is EMPTY OR sprint not in (openSprints(), futureSprints()))".into());
    parts.push("issuetype != Epic".into());
    // The last column (Done) is not in the backlog.
    let ids: Vec<&str> = match c.columns.split_last() {
        Some((_, rest)) => rest.iter().flat_map(|(_, ids)| ids.iter().map(String::as_str)).collect(),
        None => Vec::new(),
    };
    if !ids.is_empty() {
        parts.push(format!("status in ({})", ids.join(", ")));
    }
    Some(format!("{} ORDER BY Rank ASC", parts.join(" AND ")))
}

/// The condition "the Epic is not Done". Epic Status is `customfield_10017`. An Epic with
/// no Epic Status is open.
const EPIC_OPEN: &str = "(cf[10017] is EMPTY OR cf[10017] != Done)";

/// The JQL of the Epics of a Board. With `done`, also the Epics that are Done.
pub fn board_epics_jql(filter_id: &str, done: bool) -> String {
    let open = if done { String::new() } else { format!(" AND {EPIC_OPEN}") };
    format!("issuetype = Epic AND filter = {filter_id}{open} ORDER BY Rank ASC")
}

/// The JQL of the Epic picker: the open Epics of a project. A text finds a key, or words at
/// the start of the summary. With no text, the most recently updated Epics come first.
pub fn epic_search_jql(project: &str, text: &str) -> String {
    let t = text.trim();
    let base = format!("issuetype = Epic AND project = {project} AND {EPIC_OPEN}");
    if t.is_empty() {
        return format!("{base} ORDER BY updated DESC");
    }
    let key = super::model::normalize_key(t, project);
    let words = escape(t);
    if super::model::looks_like_key(&key) {
        format!("{base} AND (key = {key} OR summary ~ \"{words}*\") ORDER BY updated DESC")
    } else {
        format!("{base} AND summary ~ \"{words}*\" ORDER BY updated DESC")
    }
}

/// Make a text safe in a JQL string. Lucene special characters become spaces, because a
/// search with them gives a JQL error.
fn escape(t: &str) -> String {
    let s: String = t.chars().map(|c| if "\"\\+-&|!(){}[]^~*?:/".contains(c) { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The Sprint tab data from a Board configuration and its active Sprints.
pub fn sprint_board(c: &BoardConfig, sprints: Vec<SprintRef>) -> SprintBoard {
    let backlog_jql = if c.board.is_scrum() { backlog_jql(c) } else { None };
    SprintBoard { board: c.board.clone(), sprints, backlog_jql }
}

/// A part of the Sprint tab: one active Sprint, or the backlog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    Sprint(SprintRef),
    Backlog,
}

impl Section {
    pub fn name(&self) -> String {
        match self {
            Section::Sprint(s) => s.name.clone(),
            Section::Backlog => "Backlog".into(),
        }
    }
}

/// The parts of the Sprint tab: the active Sprints, then the backlog.
pub fn sections(b: &SprintBoard) -> Vec<Section> {
    let mut out: Vec<Section> = b.sprints.iter().cloned().map(Section::Sprint).collect();
    if b.backlog_jql.is_some() {
        out.push(Section::Backlog);
    }
    out
}

/// The JQL of a part of the Sprint tab.
pub fn section_jql(b: &SprintBoard, s: &Section) -> Option<String> {
    match s {
        Section::Sprint(r) => Some(sprint_jql(r.id)),
        Section::Backlog => b.backlog_jql.clone(),
    }
}

/// A destination of a Sprint move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Sprint(Sprint),
    Backlog,
}

impl Target {
    /// "Sprint BCE-26-Sprint-21", or "the backlog".
    pub fn label(&self) -> String {
        match self {
            Target::Sprint(s) => format!("Sprint {}", s.name),
            Target::Backlog => "the backlog".into(),
        }
    }
}

/// The Sprints that a Ticket can move to: active Sprints first, then future Sprints. Closed
/// Sprints are removed: Jira does not move Tickets to a closed Sprint.
pub fn open_sprints(mut sprints: Vec<Sprint>) -> Vec<Sprint> {
    sprints.retain(|s| !s.is_closed());
    sprints.sort_by_key(|s| !s.is_active());
    sprints
}

/// True when the Sprint name contains the text (ignores case), or the ID is the text.
pub fn sprint_matches(s: &Sprint, text: &str) -> bool {
    let t = text.trim().to_lowercase();
    t.is_empty() || s.name.to_lowercase().contains(&t) || s.id.to_string() == t
}

/// Check that a Ticket can move to the Sprint. A closed Sprint gives an error.
pub fn check_target(s: &Sprint) -> Result<()> {
    if s.is_closed() {
        return Err(Error::input_hint(
            format!("Sprint {} ({}) is closed.", s.name, s.id),
            "Jira does not move Tickets to a closed Sprint. Use `jui sprint list <board>` to find an open Sprint.",
        ));
    }
    Ok(())
}

// ------------------------------------------------------------ My Tickets by Sprint

/// The order of the Sprint groups of My Tickets: active Sprints, then future Sprints, then
/// the Tickets with no open Sprint. In each state, the lowest Sprint ID (the oldest) is first.
pub fn sprint_rank(s: Option<&SprintRef>) -> (u8, u64) {
    match s {
        Some(s) if s.is_active() => (0, s.id),
        Some(s) => (1, s.id),
        None => (2, 0),
    }
}

/// Put the items in Sprint group order. The sort is stable: in each group, the items keep the
/// order of the query (for example `ORDER BY updated DESC`).
pub fn sort_by_sprint<T>(items: &mut [T], sprint: impl Fn(&T) -> Option<&SprintRef>) {
    items.sort_by_key(|i| sprint_rank(sprint(i)));
}

/// One Sprint group: the Sprint (`None` for "No Sprint") and the range of its items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SprintGroup {
    pub sprint: Option<SprintRef>,
    pub range: std::ops::Range<usize>,
}

impl SprintGroup {
    /// `BCE-26-Sprint-21 (active)`, `BCE-26-Sprint-22 (future)`, or `No Sprint`.
    pub fn title(&self) -> String {
        match &self.sprint {
            Some(s) => format!("{} ({})", s.name, s.state),
            None => "No Sprint".into(),
        }
    }
}

/// The groups of items that `sort_by_sprint` sorted. Each group is one run of the same Sprint.
pub fn sprint_groups<T>(items: &[T], sprint: impl Fn(&T) -> Option<&SprintRef>) -> Vec<SprintGroup> {
    let mut out: Vec<SprintGroup> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let s = sprint(item);
        match out.last_mut() {
            Some(g) if g.sprint.as_ref().map(|x| x.id) == s.map(|x| x.id) => g.range.end = i + 1,
            _ => out.push(SprintGroup { sprint: s.cloned(), range: i..i + 1 }),
        }
    }
    out
}

/// Split the keys into requests of at most `MOVE_CHUNK` keys.
pub fn chunks(keys: &[String]) -> impl Iterator<Item = &[String]> {
    keys.chunks(MOVE_CHUNK)
}

/// A short list of keys for messages: `BCE-1, BCE-2, and 3 more`.
pub fn key_list(keys: &[String]) -> String {
    match keys {
        [] => String::new(),
        [one] => one.clone(),
        _ if keys.len() <= 5 => keys.join(", "),
        _ => format!("{}, and {} more", keys[..4].join(", "), keys.len() - 4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Board;

    fn config(kind: &str) -> BoardConfig {
        BoardConfig {
            board: Board { id: 4166, name: "BCE board".into(), kind: kind.into() },
            filter_id: Some("117549".into()),
            sub_query: None,
            columns: vec![
                ("To Do".into(), vec!["10000".into(), "4".into()]),
                ("In Progress".into(), vec!["3".into()]),
                ("Done".into(), vec!["10001".into(), "20311".into()]),
            ],
        }
    }

    fn sprint(id: u64, name: &str, state: &str) -> Sprint {
        Sprint { id, name: name.into(), state: state.into(), start: None, end: None, complete: None, goal: None, board_id: None }
    }

    #[test]
    fn my_epics_count_the_open_tickets_in_each_epic() {
        let counts = count_by_epic(["BCE-2", "CRTX-9", "BCE-2", " ", "BCE-1"]);
        assert_eq!(counts, vec![("BCE-2".to_string(), 2), ("BCE-1".to_string(), 1), ("CRTX-9".to_string(), 1)]);
        assert_eq!(my_epics_jql(&counts).as_deref(), Some("key in (BCE-2, BCE-1, CRTX-9) ORDER BY updated DESC"));
        assert_eq!(my_epics_jql(&[]), None);
    }

    #[test]
    fn only_mine_adds_the_assignee_before_the_order() {
        assert_eq!(only_mine("sprint = 7 ORDER BY Rank ASC"), "(sprint = 7) AND assignee = currentUser() ORDER BY Rank ASC");
        assert_eq!(
            only_mine("filter = 1 OR project = X order by updated"),
            "(filter = 1 OR project = X) AND assignee = currentUser() order by updated"
        );
        assert_eq!(only_mine("project = BCE"), "(project = BCE) AND assignee = currentUser()");
        assert_eq!(only_mine("ORDER BY Rank"), "assignee = currentUser() ORDER BY Rank");
    }

    #[test]
    fn my_tickets_groups_are_active_then_future_then_no_sprint() {
        let r = |id: u64, state: &str| Some(SprintRef { id, name: format!("S{id}"), state: state.into() });
        // (key, Sprint) in the order of the query.
        let mut items = vec![
            ("A", None),
            ("B", r(30, "future")),
            ("C", r(21, "active")),
            ("D", None),
            ("E", r(20, "active")),
            ("F", r(21, "active")),
        ];
        sort_by_sprint(&mut items, |i| i.1.as_ref());
        let keys: Vec<&str> = items.iter().map(|i| i.0).collect();
        assert_eq!(keys, ["E", "C", "F", "B", "A", "D"], "stable in each group");
        let groups = sprint_groups(&items, |i| i.1.as_ref());
        let titles: Vec<String> = groups.iter().map(SprintGroup::title).collect();
        assert_eq!(titles, ["S20 (active)", "S21 (active)", "S30 (future)", "No Sprint"]);
        assert_eq!(groups[1].range, 1..3);
        assert_eq!(groups[3].range, 4..6);
        assert!(sprint_groups::<(&str, Option<SprintRef>)>(&[], |i| i.1.as_ref()).is_empty());
    }

    #[test]
    fn jql_for_sprint_epic_and_backlog() {
        assert_eq!(sprint_jql(43198), "sprint = 43198 ORDER BY Rank ASC");
        assert_eq!(epic_jql("BCE-13070"), "\"Epic Link\" = BCE-13070 ORDER BY Rank ASC");
        assert_eq!(
            backlog_jql(&config("scrum")).as_deref(),
            Some(
                "filter = 117549 AND (sprint is EMPTY OR sprint not in (openSprints(), futureSprints())) \
                 AND issuetype != Epic AND status in (10000, 4, 3) ORDER BY Rank ASC"
            )
        );
        let no_filter = BoardConfig { filter_id: None, ..config("scrum") };
        assert_eq!(backlog_jql(&no_filter), None);
        let with_sub = BoardConfig { sub_query: Some("component = UI".into()), columns: vec![], ..config("scrum") };
        assert_eq!(
            backlog_jql(&with_sub).as_deref(),
            Some(
                "filter = 117549 AND (component = UI) AND (sprint is EMPTY OR sprint not in (openSprints(), futureSprints())) \
                 AND issuetype != Epic ORDER BY Rank ASC"
            )
        );
    }

    #[test]
    fn jql_for_epics() {
        assert_eq!(
            board_epics_jql("117549", false),
            "issuetype = Epic AND filter = 117549 AND (cf[10017] is EMPTY OR cf[10017] != Done) ORDER BY Rank ASC"
        );
        assert_eq!(board_epics_jql("117549", true), "issuetype = Epic AND filter = 117549 ORDER BY Rank ASC");
        let base = "issuetype = Epic AND project = BCE AND (cf[10017] is EMPTY OR cf[10017] != Done)";
        assert_eq!(epic_search_jql("BCE", " "), format!("{base} ORDER BY updated DESC"));
        assert_eq!(epic_search_jql("BCE", "proxy cider"), format!("{base} AND summary ~ \"proxy cider*\" ORDER BY updated DESC"));
        assert_eq!(
            epic_search_jql("BCE", "13070"),
            format!("{base} AND (key = BCE-13070 OR summary ~ \"13070*\") ORDER BY updated DESC")
        );
        assert_eq!(epic_search_jql("BCE", "a \"b\" (c)*"), format!("{base} AND summary ~ \"a b c*\" ORDER BY updated DESC"));
    }

    #[test]
    fn sections_are_the_active_sprints_then_the_backlog() {
        let s = SprintRef { id: 1, name: "S1".into(), state: "active".into() };
        let b = sprint_board(&config("scrum"), vec![s.clone()]);
        assert_eq!(sections(&b), vec![Section::Sprint(s.clone()), Section::Backlog]);
        assert_eq!(section_jql(&b, &Section::Sprint(s)).as_deref(), Some("sprint = 1 ORDER BY Rank ASC"));
        let k = sprint_board(&config("kanban"), vec![]);
        assert!(sections(&k).is_empty(), "a Kanban Board has no Sprints and no backlog in jui");
    }

    #[test]
    fn open_sprints_put_active_first_and_remove_closed() {
        let list = vec![sprint(3, "Future", "future"), sprint(1, "Old", "closed"), sprint(2, "Now", "active")];
        let ids: Vec<u64> = open_sprints(list).iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![2, 3]);
        assert!(check_target(&sprint(1, "Old", "closed")).is_err());
        assert!(check_target(&sprint(2, "Now", "active")).is_ok());
        assert!(sprint_matches(&sprint(43198, "BCE-26-Sprint-21", "active"), "sprint-21"));
        assert!(sprint_matches(&sprint(43198, "BCE-26-Sprint-21", "active"), "43198"));
        assert!(!sprint_matches(&sprint(43198, "BCE-26-Sprint-21", "active"), "22"));
    }

    #[test]
    fn keys_in_chunks_and_messages() {
        let keys: Vec<String> = (1..=120).map(|n| format!("BCE-{n}")).collect();
        let sizes: Vec<usize> = chunks(&keys).map(<[String]>::len).collect();
        assert_eq!(sizes, vec![50, 50, 20]);
        assert_eq!(key_list(&keys[..2]), "BCE-1, BCE-2");
        assert_eq!(key_list(&keys[..7]), "BCE-1, BCE-2, BCE-3, BCE-4, and 3 more");
    }
}
