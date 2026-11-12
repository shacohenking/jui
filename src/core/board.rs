//! Boards and My Boards: the pure rules. See ticket 10 "Source of My Boards when the board
//! list is empty", ticket 12 "Large results in the TUI" (part 3), and ticket 22.
//!
//! - A number is a Board ID. Other text is a part of a Board name.
//! - My Boards is only the board list in the Settings. jui keeps its order.
//! - The Board picker suggests the Recent Boards (best effort), then the Boards of the
//!   default project (Scrum first, then by name).
//!
//! This module has no I/O.

use super::api;
use super::error::{Error, Result};
use super::model::{Board, BoardColumn, TicketSummary};
use super::settings::BoardRef;

/// jui gets at most this number of Tickets for one Board.
pub const BOARD_LIMIT: u64 = 1000;
/// The page size of the Board Tickets.
pub const BOARD_PAGE: u64 = 200;
/// The page size of the Board list and of a Board search.
pub const LIST_PAGE: u64 = 50;
/// The Kanban Done column shows the Tickets of this number of days.
pub const KANBAN_DONE_DAYS: u32 = 14;

/// The parts of a Board that make its JQL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardQueryParts<'a> {
    pub filter_id: &'a str,
    pub is_scrum: bool,
    pub sprint_ids: &'a [u64],
    pub sub_query: Option<&'a str>,
    /// The status IDs of all columns.
    pub status_ids: &'a [String],
    /// Only the Tickets that are assigned to the current user.
    pub only_mine: bool,
}

/// The JQL of the Tickets that the Board shows, for `/rest/api/2/search`. `None` for a
/// Scrum Board with no active Sprint.
///
/// The Agile issue endpoint gives the same Tickets, but it takes about 11 s for 200 Tickets
/// on this server, and the search takes about 1 s. A check on 4 Boards found these rules
/// (the totals are the same as the Agile endpoint):
/// - The Board shows only the statuses of its columns.
/// - A Scrum Board does not show Epics.
/// - A Scrum Board shows its active Sprints. A Kanban Board shows its sub-query, and the
///   Done Tickets of the last 14 days.
pub fn board_jql(p: &BoardQueryParts<'_>) -> Option<String> {
    let mut parts = vec![format!("filter = {}", p.filter_id)];
    if let Some(q) = p.sub_query.map(str::trim).filter(|q| !q.is_empty()) {
        parts.push(format!("({q})"));
    }
    if p.is_scrum {
        if p.sprint_ids.is_empty() {
            return None;
        }
        let ids: Vec<String> = p.sprint_ids.iter().map(u64::to_string).collect();
        parts.push(format!("sprint in ({})", ids.join(", ")));
        parts.push("issuetype != Epic".into());
    } else {
        parts.push(format!("(statusCategory != Done OR resolved >= -{KANBAN_DONE_DAYS}d)"));
    }
    if !p.status_ids.is_empty() {
        parts.push(format!("status in ({})", p.status_ids.join(", ")));
    }
    if p.only_mine {
        parts.push("assignee = currentUser()".into());
    }
    Some(format!("{} ORDER BY Rank ASC", parts.join(" AND ")))
}

/// Put each Ticket in the column that contains its status. Returns the columns and the
/// number of Tickets with a status that is in no column (the Board does not show them).
pub fn sort_columns(columns: &[(String, Vec<String>)], tickets: Vec<(Option<String>, TicketSummary)>) -> (Vec<BoardColumn>, u64) {
    let mut out: Vec<BoardColumn> = columns
        .iter()
        .map(|(name, ids)| BoardColumn { name: name.clone(), status_ids: ids.clone(), tickets: Vec::new() })
        .collect();
    let mut unmapped = 0;
    for (status_id, t) in tickets {
        let col = status_id.as_deref().and_then(|id| out.iter().position(|c| c.status_ids.iter().any(|s| s == id)));
        match col {
            Some(i) => out[i].tickets.push(t),
            None => unmapped += 1,
        }
    }
    (out, unmapped)
}

// ---------------------------------------------------------------- arguments

/// A Board argument from the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardArg {
    Id(u64),
    Name(String),
}

impl BoardArg {
    pub fn parse(s: &str) -> Self {
        let t = s.trim();
        match t.parse::<u64>() {
            Ok(id) if id > 0 => BoardArg::Id(id),
            _ => BoardArg::Name(t.to_string()),
        }
    }
}

/// The result of a name match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick<T> {
    One(T),
    None,
    Many(Vec<T>),
}

/// Find items by name:
/// 1. A name that is the same as the text (ignores case) wins.
/// 2. Else, the names that start with the text.
/// 3. Else, the names that contain the text.
pub fn pick<T: Clone>(items: &[T], text: &str, name: impl for<'a> Fn(&'a T) -> &'a str) -> Pick<T> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Pick::None;
    }
    if let Some(i) = items.iter().find(|i| name(i).trim().to_lowercase() == t) {
        return Pick::One(i.clone());
    }
    let starts: Vec<&T> = items.iter().filter(|i| name(i).trim().to_lowercase().starts_with(&t)).collect();
    let found = if starts.is_empty() { items.iter().filter(|i| name(i).to_lowercase().contains(&t)).collect() } else { starts };
    match found.as_slice() {
        [] => Pick::None,
        [one] => Pick::One((*one).clone()),
        many => Pick::Many(many.iter().map(|i| (*i).clone()).collect()),
    }
}

/// The error for "more than one Board matches" in a script.
pub fn many_match(text: &str, boards: &[Board]) -> Error {
    many_match_of(text, boards, boards.len() as u64)
}

/// The error for "more than one Board matches", with the total from the server (the
/// server can find more Boards than one page).
pub fn many_match_of(text: &str, boards: &[Board], total: u64) -> Error {
    let names: Vec<String> = boards.iter().take(8).map(|b| format!("{} ({})", b.name.trim(), b.id)).collect();
    let more = if boards.len() > 8 { ", …" } else { "" };
    let hint = if total > boards.len() as u64 {
        format!("Use the Board ID, or a longer part of the name. Matches: {}{more}.", names.join(", "))
    } else {
        format!("Use the Board ID. Matches: {}{more}.", names.join(", "))
    };
    Error::input_hint(format!("`{text}` matches {} Boards.", total.max(boards.len() as u64)), hint)
}

/// The error for "no Board matches".
pub fn no_match(text: &str) -> Error {
    Error::input_hint(format!("No Board matches `{text}`."), "Use `jui board list --name <text>` to find the Board ID.")
}

/// The error for "My Boards is empty" in a script.
pub fn empty_in_script() -> Error {
    Error::input_hint("My Boards is empty.", "Use `jui board add <id>`, or run `jui board mine` on a terminal.")
}

/// Asks the user to select one Board on a terminal. Returns the index.
pub type Chooser<'a> = &'a mut dyn FnMut(&[Board]) -> Result<usize>;

/// Make a `Pick` into one Board, or an error. `choose` is `None` in a script.
pub fn resolve(pick: Pick<Board>, text: &str, choose: Option<Chooser<'_>>) -> Result<Board> {
    match pick {
        Pick::One(b) => Ok(b),
        Pick::None => Err(no_match(text)),
        Pick::Many(bs) => match choose {
            Some(c) => {
                let i = c(&bs)?;
                bs.into_iter().nth(i).ok_or(Error::Cancelled)
            }
            None => Err(many_match(text, &bs)),
        },
    }
}

// ---------------------------------------------------------------- My Boards

/// The result of an add to My Boards.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Change {
    pub list: Vec<BoardRef>,
    /// The Boards that changed the list (added or removed).
    pub changed: Vec<BoardRef>,
    /// The Boards that did not change the list (already in it, or not in it).
    pub skipped: Vec<BoardRef>,
}

pub fn board_ref(b: &Board) -> BoardRef {
    BoardRef { id: b.id, name: Some(b.name.clone()) }
}

/// Add Boards at the end of the list, with no duplicates. A Board that is already in the
/// list stays at its position, and jui updates its name.
pub fn add(list: &[BoardRef], boards: &[Board]) -> Change {
    let mut c = Change { list: list.to_vec(), ..Default::default() };
    for b in boards {
        match c.list.iter_mut().find(|r| r.id == b.id) {
            Some(r) => {
                r.name = Some(b.name.clone());
                c.skipped.push(board_ref(b));
            }
            None => {
                c.list.push(board_ref(b));
                c.changed.push(board_ref(b));
            }
        }
    }
    c
}

/// Remove Boards from the list by ID.
pub fn remove(list: &[BoardRef], ids: &[u64]) -> Change {
    let mut c = Change::default();
    for r in list {
        if ids.contains(&r.id) {
            c.changed.push(r.clone());
        } else {
            c.list.push(r.clone());
        }
    }
    for id in ids {
        if !list.iter().any(|r| r.id == *id) {
            c.skipped.push(BoardRef { id: *id, name: None });
        }
    }
    c
}

/// Find a Board in My Boards for `jui board remove`: an ID in the list, or a name match.
pub fn find_mine(list: &[BoardRef], text: &str) -> Result<Pick<BoardRef>> {
    match BoardArg::parse(text) {
        BoardArg::Id(id) => Ok(match list.iter().find(|r| r.id == id) {
            Some(r) => Pick::One(r.clone()),
            None => Pick::None,
        }),
        BoardArg::Name(t) => Ok(pick(list, &t, |r| r.name.as_deref().unwrap_or(""))),
    }
}

/// Update the stored names from the names in Jira. Returns `None` when no name changed.
pub fn renamed(list: &[BoardRef], boards: &[Board]) -> Option<Vec<BoardRef>> {
    let mut changed = false;
    let out = list
        .iter()
        .map(|r| match boards.iter().find(|b| b.id == r.id) {
            Some(b) if r.name.as_deref() != Some(b.name.as_str()) => {
                changed = true;
                BoardRef { id: r.id, name: Some(b.name.clone()) }
            }
            _ => r.clone(),
        })
        .collect();
    changed.then_some(out)
}

/// The text for a Board in My Boards: `4166  BCE board`.
pub fn ref_label(r: &BoardRef) -> String {
    match &r.name {
        Some(n) => format!("{}  {n}", r.id),
        None => r.id.to_string(),
    }
}

// ---------------------------------------------------------------- suggestions

/// Where a suggestion comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Recent,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub board: Board,
    pub origin: Origin,
}

/// Read the Recent Boards from the private menu endpoint. A shape that jui does not know
/// gives an empty list, not an error.
pub fn recent_from_menu(menu: &api::Menu) -> Vec<Board> {
    let Some(section) = menu.sections.iter().find(|s| s.id.ends_with("_recent")) else { return Vec::new() };
    section
        .items
        .iter()
        .filter_map(|it| {
            let id = it
                .id
                .strip_prefix("rapidb_lnk_")
                .and_then(|n| n.parse::<u64>().ok())
                .or_else(|| it.url.split("rapidView=").nth(1).and_then(|r| r.split('&').next()).and_then(|n| n.parse().ok()))?;
            let t = it.title.to_lowercase();
            let kind = if t.contains("scrum") {
                "scrum"
            } else if t.contains("kanban") {
                "kanban"
            } else {
                ""
            };
            Some(Board { id, name: it.label.clone(), kind: kind.to_string() })
        })
        .collect()
}

/// The suggestions of the Board picker: the Recent Boards, then the Boards of the default
/// project (Scrum first, then by name), with no duplicates.
pub fn suggestions(recent: Vec<Board>, mut project: Vec<Board>) -> Vec<Suggestion> {
    project.sort_by_key(|b| (!b.is_scrum(), b.name.to_lowercase()));
    let mut out: Vec<Suggestion> = Vec::new();
    for (boards, origin) in [(recent, Origin::Recent), (project, Origin::Project)] {
        for b in boards {
            if !out.iter().any(|s| s.board.id == b.id) {
                out.push(Suggestion { board: b, origin });
            }
        }
    }
    out
}

/// True when the Board name or ID contains the text (ignores case). The picker filters the
/// suggestions with it.
pub fn matches(b: &Board, text: &str) -> bool {
    let t = text.trim().to_lowercase();
    t.is_empty() || b.name.to_lowercase().contains(&t) || b.id.to_string() == t
}

/// Read a list of numbers for the CLI picker, for example `1 3` or `1,3`. The numbers
/// start at 1. An empty answer gives an empty list.
pub fn parse_numbers(answer: &str, max: usize) -> std::result::Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for part in answer.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()) {
        match part.parse::<usize>() {
            Ok(n) if (1..=max).contains(&n) => {
                if !out.contains(&(n - 1)) {
                    out.push(n - 1);
                }
            }
            _ => return Err(format!("`{part}` is not a number from 1 to {max}.")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::StatusCategory;

    fn b(id: u64, name: &str, kind: &str) -> Board {
        Board { id, name: name.into(), kind: kind.into() }
    }

    fn r(id: u64, name: &str) -> BoardRef {
        BoardRef { id, name: Some(name.into()) }
    }

    fn t(key: &str) -> TicketSummary {
        TicketSummary {
            key: key.into(),
            summary: String::new(),
            kind: "Task".into(),
            status: String::new(),
            status_category: StatusCategory::ToDo,
            priority: None,
            assignee: None,
            updated: None,
            sprint: None,
        }
    }

    #[test]
    fn jql_for_scrum_and_kanban() {
        let ids = vec!["1".to_string(), "3".to_string()];
        let scrum = BoardQueryParts { filter_id: "117549", is_scrum: true, sprint_ids: &[43198, 43199], sub_query: None, status_ids: &ids, only_mine: false };
        assert_eq!(
            board_jql(&scrum).as_deref(),
            Some("filter = 117549 AND sprint in (43198, 43199) AND issuetype != Epic AND status in (1, 3) ORDER BY Rank ASC")
        );
        assert_eq!(board_jql(&BoardQueryParts { sprint_ids: &[], ..scrum.clone() }), None);
        let kanban = BoardQueryParts {
            filter_id: "96374",
            is_scrum: false,
            sprint_ids: &[],
            sub_query: Some("fixVersion is EMPTY OR fixVersion in unreleasedVersions()"),
            status_ids: &ids,
            only_mine: false,
        };
        assert_eq!(
            board_jql(&kanban).as_deref(),
            Some(
                "filter = 96374 AND (fixVersion is EMPTY OR fixVersion in unreleasedVersions()) \
                 AND (statusCategory != Done OR resolved >= -14d) AND status in (1, 3) ORDER BY Rank ASC"
            )
        );
        assert_eq!(
            board_jql(&BoardQueryParts { sub_query: Some(" "), status_ids: &[], ..kanban }).as_deref(),
            Some("filter = 96374 AND (statusCategory != Done OR resolved >= -14d) ORDER BY Rank ASC")
        );
        // Only my Tickets: one more condition before the order.
        assert_eq!(
            board_jql(&BoardQueryParts { only_mine: true, ..scrum.clone() }).as_deref(),
            Some(
                "filter = 117549 AND sprint in (43198, 43199) AND issuetype != Epic AND status in (1, 3) \
                 AND assignee = currentUser() ORDER BY Rank ASC"
            )
        );
        assert_eq!(board_jql(&BoardQueryParts { only_mine: true, sprint_ids: &[], ..scrum }), None);
    }

    #[test]
    fn tickets_go_to_the_column_of_their_status() {
        let cols = vec![("To Do".to_string(), vec!["1".to_string()]), ("Done".to_string(), vec!["3".to_string(), "4".to_string()])];
        let (out, unmapped) =
            sort_columns(&cols, vec![(Some("4".into()), t("A-1")), (Some("1".into()), t("A-2")), (Some("9".into()), t("A-3")), (None, t("A-4"))]);
        assert_eq!(out[0].tickets.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["A-2"]);
        assert_eq!(out[1].tickets.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["A-1"]);
        assert_eq!(unmapped, 2);
    }

    #[test]
    fn parse_id_or_name_and_pick() {
        assert_eq!(BoardArg::parse(" 4166 "), BoardArg::Id(4166));
        assert_eq!(BoardArg::parse("0"), BoardArg::Name("0".into()));
        assert_eq!(BoardArg::parse("BCE board"), BoardArg::Name("BCE board".into()));
        let bs = vec![b(1, "BCE board", "scrum"), b(2, "BCE Kanban Board", "kanban"), b(3, "CAS Bugs", "kanban")];
        fn name(x: &Board) -> &str {
            x.name.as_str()
        }
        assert_eq!(pick(&bs, "bce BOARD", name), Pick::One(bs[0].clone()));
        assert_eq!(pick(&bs, "bce", name), Pick::Many(vec![bs[0].clone(), bs[1].clone()]));
        assert_eq!(pick(&bs, "bugs", name), Pick::One(bs[2].clone()));
        assert_eq!(pick(&bs, "zz", name), Pick::None);
    }

    #[test]
    fn resolve_asks_on_a_terminal_and_stops_in_a_script() {
        let two = vec![b(1, "A one", "scrum"), b(2, "A two", "kanban")];
        let mut choose = |_: &[Board]| Ok(1);
        assert_eq!(resolve(Pick::Many(two.clone()), "a", Some(&mut choose)).unwrap().id, 2);
        let e = resolve(Pick::Many(two), "a", None).unwrap_err();
        assert_eq!(e.to_string(), "`a` matches 2 Boards.");
        assert!(e.hint().unwrap().contains("A one (1), A two (2)"));
        assert_eq!(resolve(Pick::None, "zz", None).unwrap_err().to_string(), "No Board matches `zz`.");
    }

    #[test]
    fn add_keeps_the_order_and_has_no_duplicates() {
        let list = vec![r(1, "Old name"), r(2, "Two")];
        let c = add(&list, &[b(3, "Three", "scrum"), b(1, "New name", "scrum"), b(3, "Three", "scrum")]);
        assert_eq!(c.list, vec![r(1, "New name"), r(2, "Two"), r(3, "Three")]);
        assert_eq!(c.changed, vec![r(3, "Three")]);
        assert_eq!(c.skipped.len(), 2);
    }

    #[test]
    fn remove_and_find_mine() {
        let list = vec![r(1, "BCE board"), r(2, "BCE Kanban Board"), r(3, "CAS Bugs")];
        let c = remove(&list, &[2, 9]);
        assert_eq!(c.list, vec![r(1, "BCE board"), r(3, "CAS Bugs")]);
        assert_eq!(c.changed, vec![r(2, "BCE Kanban Board")]);
        assert_eq!(c.skipped, vec![BoardRef { id: 9, name: None }]);
        assert_eq!(find_mine(&list, "3").unwrap(), Pick::One(r(3, "CAS Bugs")));
        assert_eq!(find_mine(&list, "9").unwrap(), Pick::None);
        assert_eq!(find_mine(&list, "kanban").unwrap(), Pick::One(r(2, "BCE Kanban Board")));
        assert!(matches!(find_mine(&list, "bce").unwrap(), Pick::Many(v) if v.len() == 2));
    }

    #[test]
    fn renamed_changes_only_the_names_that_changed() {
        let list = vec![r(1, "A"), BoardRef { id: 2, name: None }];
        assert_eq!(renamed(&list, &[b(1, "A", "scrum")]), None);
        assert_eq!(renamed(&list, &[b(2, "Two", "scrum")]), Some(vec![r(1, "A"), r(2, "Two")]));
    }

    #[test]
    fn recent_boards_from_the_menu() {
        let menu: api::Menu = serde_json::from_str(
            r#"{"sections":[{"id":"greenhopper_menu_dropdown_recent","items":[
                {"label":"PostOps Stitch","title":"View Scrum Board 'PostOps Stitch'","id":"rapidb_lnk_4537","url":"/secure/RapidBoard.jspa?rapidView=4537&useStoredSettings=true"},
                {"label":"K","title":"View Kanban Board 'K'","id":"other","url":"/secure/RapidBoard.jspa?rapidView=12"},
                {"label":"Bad","title":"","id":"x","url":"/nothing"}]},
              {"id":"greenhopper_menu_dropdown","items":[{"label":"View all boards","id":"ghx-manageviews-mlink","url":"/secure/ManageRapidViews.jspa"}]}]}"#,
        )
        .unwrap();
        assert_eq!(recent_from_menu(&menu), vec![b(4537, "PostOps Stitch", "scrum"), b(12, "K", "kanban")]);
        assert!(recent_from_menu(&api::Menu::default()).is_empty());
    }

    #[test]
    fn suggestions_put_recent_first_then_scrum_by_name() {
        let s = suggestions(
            vec![b(9, "Recent", "kanban"), b(2, "beta", "scrum")],
            vec![b(1, "Zed", "kanban"), b(2, "beta", "scrum"), b(3, "alpha", "scrum"), b(4, "Kan", "kanban")],
        );
        let ids: Vec<(u64, Origin)> = s.iter().map(|s| (s.board.id, s.origin)).collect();
        assert_eq!(ids, vec![(9, Origin::Recent), (2, Origin::Recent), (3, Origin::Project), (4, Origin::Project), (1, Origin::Project)]);
    }

    #[test]
    fn numbers_for_the_cli_picker() {
        assert_eq!(parse_numbers("1 3", 5), Ok(vec![0, 2]));
        assert_eq!(parse_numbers("2,2, 1", 5), Ok(vec![1, 0]));
        assert_eq!(parse_numbers("", 5), Ok(vec![]));
        assert!(parse_numbers("6", 5).is_err());
        assert!(parse_numbers("x", 5).is_err());
    }
}
