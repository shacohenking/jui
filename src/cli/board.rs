//! `jui board list|view|mine|add|remove`. See ticket 10 "Source of My Boards when the board
//! list is empty" and ticket 22 "Build: Boards and My Boards".

use std::io::{BufRead, Write};

use owo_colors::{OwoColorize, Style};
use serde::Serialize;

use super::prompt;
use crate::core::board::{self as bd, BoardArg, Pick};
use crate::core::jira::BoardQuery;
use crate::core::model::{Board, BoardView};
use crate::core::settings::{self, BoardRef, Settings};
use crate::core::{Error, Jira, Result};

/// Where the questions come from. `input` is `None` in a script.
pub struct Ask<'a, 'b> {
    pub input: Option<&'a mut (dyn BufRead + 'b)>,
    pub out: &'a mut (dyn Write + 'b),
}

/// A row of `jui board list --json`: the Board and the My Boards mark.
#[derive(Debug, Serialize)]
pub struct ListRow {
    #[serde(flatten)]
    pub board: Board,
    pub mine: bool,
}

/// A row of `jui board mine --json`. `found` is false when Jira gives HTTP 404 for the Board.
#[derive(Debug, Serialize)]
pub struct MineRow {
    pub id: u64,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub found: bool,
}

/// Find one Board from an argument: an ID gets that Board; a name searches all Boards
/// with the `name` parameter (at most 50 results).
pub async fn find(j: &Jira, arg: &str, ask: &mut Ask<'_, '_>) -> Result<Board> {
    match BoardArg::parse(arg) {
        BoardArg::Id(id) => j.board(id).await,
        BoardArg::Name(text) => {
            if text.is_empty() {
                return Err(Error::input_hint("The Board name is empty.", "Give a Board ID or a part of the name."));
            }
            let q = BoardQuery { name: Some(text.clone()), ..Default::default() };
            let page = j.boards(&q, 0, bd::LIST_PAGE).await?;
            let pick = bd::pick(&page.items, &text, |b| b.name.as_str());
            if ask.input.is_none()
                && let Pick::Many(bs) = &pick
            {
                return Err(bd::many_match_of(&text, bs, page.total));
            }
            let title = format!("More than one Board matches `{text}`:");
            let out = &mut *ask.out;
            let mut choose = ask.input.as_deref_mut().map(|input| {
                move |bs: &[Board]| {
                    let items: Vec<String> = bs.iter().map(Board::label).collect();
                    prompt::select(input, out, &title, &items)
                }
            });
            bd::resolve(pick, &text, choose.as_mut().map(|f| f as bd::Chooser<'_>))
        }
    }
}

/// `jui board add`: find each Board, then add them to the list and save the file.
/// Returns the change.
pub async fn add(j: &Jira, s: &Settings, args: &[String], ask: &mut Ask<'_, '_>) -> Result<bd::Change> {
    let mut boards = Vec::new();
    for a in args {
        boards.push(find(j, a, ask).await?);
    }
    let change = bd::add(&s.my_boards, &boards);
    if change.list != s.my_boards {
        settings::write_boards(&s.path, &change.list)?;
    }
    Ok(change)
}

/// `jui board remove`: find each Board in My Boards (no request to Jira), then save the file.
pub fn remove(s: &Settings, args: &[String], ask: &mut Ask<'_, '_>) -> Result<bd::Change> {
    let mut ids = Vec::new();
    for a in args {
        let r = match bd::find_mine(&s.my_boards, a)? {
            Pick::One(r) => r,
            Pick::None => {
                return Err(Error::input_hint(
                    format!("`{a}` is not in My Boards."),
                    "Use `jui board mine` to see My Boards.",
                ));
            }
            Pick::Many(rs) => match ask.input.as_deref_mut() {
                Some(input) => {
                    let items: Vec<String> = rs.iter().map(bd::ref_label).collect();
                    let i = prompt::select(input, ask.out, &format!("More than one Board in My Boards matches `{a}`:"), &items)?;
                    rs[i].clone()
                }
                None => {
                    let names: Vec<String> = rs.iter().map(bd::ref_label).collect();
                    return Err(Error::input_hint(
                        format!("`{a}` matches {} Boards in My Boards.", rs.len()),
                        format!("Use the Board ID. Matches: {}.", names.join(", ")),
                    ));
                }
            },
        };
        ids.push(r.id);
    }
    let change = bd::remove(&s.my_boards, &ids);
    if change.list != s.my_boards {
        settings::write_boards(&s.path, &change.list)?;
    }
    Ok(change)
}

/// `jui board mine` when the list is empty: show the suggestions and read the numbers.
/// Returns the Boards that the user selected (they are saved). An empty answer selects nothing.
pub async fn pick_first(j: &Jira, s: &Settings, ask: &mut Ask<'_, '_>) -> Result<Vec<BoardRef>> {
    let Some(input) = ask.input.as_deref_mut() else { return Err(bd::empty_in_script()) };
    let sugg = j.board_suggestions(&s.default_project).await?;
    if sugg.is_empty() {
        return Err(Error::input_hint(
            format!("My Boards is empty, and jui found no Boards for project {}.", s.default_project),
            "Use `jui board list --name <text>`, then `jui board add <id>`.",
        ));
    }
    let _ = writeln!(ask.out, "My Boards is empty. Select Boards to add (recent Boards first, then project {}):", s.default_project);
    for (i, sg) in sugg.iter().enumerate() {
        let tag = if sg.origin == bd::Origin::Recent { "  (recent)" } else { "" };
        let _ = writeln!(ask.out, "  {:>2}) {}{tag}", i + 1, sg.board.label());
    }
    let chosen = loop {
        let answer = prompt::ask(input, ask.out, "Numbers, for example `1 3` (Enter selects nothing): ")?;
        match bd::parse_numbers(&answer, sugg.len()) {
            Ok(n) => break n,
            Err(e) => {
                let _ = writeln!(ask.out, "{e}");
            }
        }
    };
    if chosen.is_empty() {
        return Ok(Vec::new());
    }
    let boards: Vec<Board> = chosen.iter().map(|&i| sugg[i].board.clone()).collect();
    let change = bd::add(&s.my_boards, &boards);
    settings::write_boards(&s.path, &change.list)?;
    Ok(change.changed)
}

/// `jui board mine` with a list: get each Board to find the Boards that are not found, and
/// to update the stored names.
pub async fn mine(j: &Jira, s: &Settings) -> Result<Vec<MineRow>> {
    let results = futures::future::join_all(s.my_boards.iter().map(|r| j.board(r.id))).await;
    let mut rows = Vec::new();
    let mut found = Vec::new();
    for (r, res) in s.my_boards.iter().zip(results) {
        match res {
            Ok(b) => {
                rows.push(MineRow { id: b.id, name: Some(b.name.clone()), kind: Some(b.kind.clone()), found: true });
                found.push(b);
            }
            Err(Error::NotFound(_)) => rows.push(MineRow { id: r.id, name: r.name.clone(), kind: None, found: false }),
            Err(e) => return Err(e),
        }
    }
    save_names(s, &found);
    Ok(rows)
}

/// Update the stored names of My Boards from the names in Jira. An error is only a warning.
pub fn save_names(s: &Settings, boards: &[Board]) {
    if let Some(list) = bd::renamed(&s.my_boards, boards)
        && let Err(e) = settings::write_boards(&s.path, &list)
    {
        super::output::print_warning(&format!("jui could not update the Board names in the Settings: {e}"));
    }
}

/// The text of `jui board view`.
pub fn render(v: &BoardView, color: bool) -> String {
    let st = |s: Style, t: &str| if color { t.style(s).to_string() } else { t.to_string() };
    let mut out = format!("{}  {}\n", st(Style::new().cyan().bold(), &v.board.id.to_string()), st(Style::new().bold(), &v.board.name));
    out.push_str(&format!("Type       {}\n", v.board.kind));
    if v.board.is_scrum() {
        let sprints = if v.sprints.is_empty() {
            "no active Sprint".to_string()
        } else {
            v.sprints.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", ")
        };
        out.push_str(&format!("Sprint     {sprints}\n"));
    }
    if let Some(f) = &v.filter_id {
        out.push_str(&format!("Filter     {f}\n"));
    }
    out.push_str(&format!("URL        {}\n", v.url));
    if v.no_active_sprint() {
        out.push_str("\nThe Board has no active Sprint.");
        return out;
    }
    let mine = if v.only_mine { " (only mine)" } else { "" };
    out.push_str(&format!("Tickets    {}{mine}\n", v.count()));
    for c in &v.columns {
        out.push_str(&format!("\n{}\n", st(Style::new().bold(), &format!("{} ({})", c.name, c.tickets.len()))));
        let key_w = c.tickets.iter().map(|t| t.key.len()).max().unwrap_or(0);
        for t in &c.tickets {
            let who = t.assignee.as_ref().map(|u| format!("  @{}", u.name)).unwrap_or_default();
            let key = format!("{:<key_w$}", t.key);
            out.push_str(&format!("  {}  {}{}\n", st(Style::new().cyan(), &key), t.summary, st(Style::new().dimmed(), &who)));
        }
    }
    if v.unmapped > 0 {
        out.push_str(&format!("\n{} Tickets have a status that is in no column. The Board does not show them.\n", v.unmapped));
    }
    out.trim_end().to_string()
}

/// The note on stderr when the Board has more Tickets than jui got.
pub fn truncated_note(v: &BoardView) -> Option<String> {
    v.truncated().then(|| {
        format!("Showing {} of {} Tickets. Board has more Tickets. Use `jui search` with JQL.", v.loaded, v.total)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{BoardColumn, SprintRef, StatusCategory, TicketSummary};

    fn view(sprints: Vec<SprintRef>) -> BoardView {
        let t = TicketSummary {
            key: "BCE-1".into(),
            summary: "Fix the login".into(),
            kind: "Bug".into(),
            status: "To Do".into(),
            status_category: StatusCategory::ToDo,
            priority: None,
            assignee: None,
            updated: None,
            sprint: None,
        };
        BoardView {
            board: Board { id: 4166, name: "BCE board".into(), kind: "scrum".into() },
            url: "https://jira.example/secure/RapidBoard.jspa?rapidView=4166".into(),
            filter_id: Some("117549".into()),
            sub_query: None,
            sprints,
            jql: Some("sprint in openSprints()".into()),
            columns: vec![
                BoardColumn { name: "To Do".into(), status_ids: vec!["1".into()], tickets: vec![t] },
                BoardColumn { name: "Done".into(), status_ids: vec!["2".into()], tickets: vec![] },
            ],
            unmapped: 1,
            total: 2,
            loaded: 2,
            only_mine: false,
        }
    }

    #[test]
    fn render_shows_columns_and_the_sprint() {
        let text = render(&view(vec![SprintRef { id: 1, name: "Sprint 21".into(), state: "active".into() }]), false);
        assert!(text.starts_with("4166  BCE board\nType       scrum\nSprint     Sprint 21\nFilter     117549\n"), "{text}");
        assert!(text.contains("\nTo Do (1)\n  BCE-1  Fix the login\n"), "{text}");
        assert!(text.contains("\nDone (0)\n"));
        assert!(text.contains("1 Tickets have a status that is in no column."));
        assert!(text.contains("\nTickets    2\n"), "{text}");
        let mine = BoardView { only_mine: true, ..view(vec![SprintRef { id: 1, name: "S".into(), state: "active".into() }]) };
        assert!(render(&mine, false).contains("\nTickets    2 (only mine)\n"));
    }

    #[test]
    fn render_without_an_active_sprint() {
        let text = render(&view(vec![]), false);
        assert!(text.contains("Sprint     no active Sprint"));
        assert!(text.ends_with("The Board has no active Sprint."), "{text}");
    }
}
