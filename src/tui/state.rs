//! The TUI state. `update` changes it, and `view` draws it.

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use ratatui::text::Line;
use ratatui::widgets::TableState;

use crate::core::model::{ListRow, ScreenField, Ticket, Transition, User};
use crate::core::user::{Assignee, UserArg};
use crate::core::transition::filter_matches;
use crate::core::settings::{DateFormat, Settings};

/// The page size of Ticket lists. See ticket 12 "Large results in the TUI".
pub const PAGE_SIZE: u64 = 100;
/// jui gets the next page when the selection is this number of rows (or less) from the end.
pub const NEXT_PAGE_DISTANCE: usize = 20;
/// The time that the selection must stay on one row before jui gets the full Ticket.
pub const PREVIEW_DELAY_MS: u64 = 150;
/// The number of full Tickets that jui keeps in memory.
pub const CACHE_SIZE: usize = 50;
/// The time between two `Tick` events.
pub const TICK_MS: u64 = 250;
/// A status line message stays for 8 seconds (32 ticks).
pub const STATUS_TICKS: u32 = 32;
/// The number of errors that the `!` popup keeps.
pub const ERROR_LOG_SIZE: usize = 20;
/// Narrower than this, jui hides the preview.
pub const NARROW: u16 = 100;
/// Wider than this, the list shows more columns.
pub const WIDE: u16 = 160;

pub const TAB_TITLES: [&str; 6] = ["My Tickets", "Boards", "Sprint", "Epics", "Search", "Settings"];
pub const MY_TICKETS: usize = 0;
pub const BOARDS: usize = 1;
pub const SPRINT: usize = 2;
pub const EPICS: usize = 3;
pub const SEARCH: usize = 4;
pub const SETTINGS: usize = 5;

/// The Epics tab: the Epics that have at least one open Ticket of the user. The list of the
/// tab (`tabs[EPICS].list`) shows the Epics; this struct has the counts.
#[derive(Debug, Default)]
pub struct EpicsTab {
    /// The number of open Tickets of the user in each Epic.
    pub counts: HashMap<String, usize>,
    /// The generation of the last count request. Responses of older generations are ignored.
    pub generation: u64,
    /// jui is getting the counts.
    pub loading: bool,
}

/// One selectable row of the Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    /// A Setting with one value.
    Value(&'static crate::core::settings::Meta),
    /// Board `n` of My Boards.
    Board(usize),
    /// The last row of the My Boards group: Enter opens the Board picker.
    AddBoards,
}

/// The Settings page: the selected row and the scroll position of its table.
#[derive(Debug, Default)]
pub struct SettingsPage {
    /// The index in `State::settings_rows()`.
    pub selected: usize,
    /// The scroll position of the table (the view writes it).
    pub table: TableState,
}

/// The edit popup of one Setting on the Settings page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingPopup {
    pub meta: &'static crate::core::settings::Meta,
    /// The text input. For a list of values, it filters the values.
    pub input: String,
    /// The permitted values (a choice or the Ticket types). `None` for a text input, or
    /// while jui gets the Ticket types.
    pub choices: Option<Vec<String>>,
    /// The index in the matching values.
    pub selected: usize,
    /// jui checks the project (`GET /project/<key>`).
    pub checking: bool,
    /// jui writes the file.
    pub saving: bool,
    pub error: Option<String>,
}

impl SettingPopup {
    /// True when the popup shows a list of values.
    pub fn is_list(&self) -> bool {
        use crate::core::settings::Kind;
        matches!(self.meta.kind, Kind::Choice(_) | Kind::TicketType)
    }

    /// The values that match the text (all values when the text is the current value).
    pub fn shown(&self) -> Vec<String> {
        let t = self.input.trim().to_lowercase();
        self.choices.iter().flatten().filter(|c| c.to_lowercase().contains(&t)).cloned().collect()
    }

    pub fn busy(&self) -> bool {
        self.checking || self.saving
    }
}

/// The config file after `E` and the editor: not valid. `e` opens the editor again with
/// the text, `q` discards the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInvalid {
    /// The file text before the edit.
    pub original: String,
    /// The text of the user, with the error.
    pub text: String,
    pub message: String,
}
/// jui gets at most this number of child Tickets for the zoom of an Epic.
pub const EPIC_CHILDREN_MAX: u64 = 500;

/// The Sprint tab: the Board, its active Sprints and backlog, and the selected part. The
/// Tickets of the selected part are in the list of the tab (`tabs[SPRINT].list`).
#[derive(Debug, Default)]
pub struct SprintTab {
    /// The Board that the tab shows (or is getting).
    pub board_id: Option<u64>,
    pub data: Option<crate::core::model::SprintBoard>,
    pub loading: bool,
    /// The generation of the last request. Responses of older generations are ignored.
    pub generation: u64,
    /// The selected part (an index in `sections()`).
    pub section: usize,
    /// After a refresh: select this part again, and keep the selected Ticket.
    pub refresh: Option<crate::core::agile::Section>,
    pub error: Option<String>,
    /// Show only the Tickets that are assigned to the current user (`sprint.only_mine`).
    pub only_mine: bool,
}

impl SprintTab {
    pub fn sections(&self) -> Vec<crate::core::agile::Section> {
        self.data.as_ref().map(crate::core::agile::sections).unwrap_or_default()
    }

    pub fn current(&self) -> Option<crate::core::agile::Section> {
        self.sections().get(self.section).cloned()
    }
}

/// The `s` popup: the open Sprints of a Board, and the backlog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SprintPopup {
    pub key: String,
    pub board_id: u64,
    pub board_name: String,
    /// `None` while jui gets the Sprints.
    pub list: Option<Vec<crate::core::model::Sprint>>,
    pub filter: String,
    pub selected: usize,
    pub sending: bool,
    pub error: Option<String>,
}

impl SprintPopup {
    /// The rows: the open Sprints that match the filter (active first), then the backlog.
    pub fn rows(&self) -> Vec<crate::core::agile::Target> {
        use crate::core::agile::{Target, sprint_matches};
        let mut rows: Vec<Target> =
            self.list.iter().flatten().filter(|s| sprint_matches(s, &self.filter)).cloned().map(Target::Sprint).collect();
        if self.list.is_some() && "backlog".contains(&self.filter.trim().to_lowercase()) {
            rows.push(Target::Backlog);
        }
        rows
    }
}

/// One row of the Epic picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpicRow {
    /// Remove the Epic.
    Nobody,
    Epic(crate::core::model::Epic),
}

/// The `E` popup: a text input and a list of open Epics of the project.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EpicPopup {
    pub key: String,
    pub project: String,
    pub input: String,
    /// The open Epics of the project (the list with no text). `None` while jui gets them.
    pub base: Option<Vec<crate::core::model::Epic>>,
    /// The server results for the current text. `None` before the first result.
    pub results: Option<Vec<crate::core::model::Epic>>,
    /// The number of the last search. Number 0 is the list with no text.
    pub seq: u64,
    pub searching: bool,
    pub selected: usize,
    pub sending: bool,
    pub error: Option<String>,
}

impl EpicPopup {
    /// True when the text is long enough for a search on the server.
    pub fn searches(&self) -> bool {
        self.input.trim().chars().count() >= USER_SEARCH_MIN
    }

    /// The rows. With a short text: `none` and the open Epics that match, in memory.
    /// With a longer text: the server results.
    pub fn rows(&self) -> Vec<EpicRow> {
        let t = self.input.trim().to_lowercase();
        if self.searches() {
            let mut rows = Vec::new();
            if "none".starts_with(&t) {
                rows.push(EpicRow::Nobody);
            }
            rows.extend(self.results.iter().flatten().cloned().map(EpicRow::Epic));
            return rows;
        }
        let mut rows = Vec::new();
        if "none".starts_with(&t) {
            rows.push(EpicRow::Nobody);
        }
        let hit = |e: &&crate::core::model::Epic| e.key.to_lowercase().contains(&t) || e.name.to_lowercase().contains(&t);
        rows.extend(self.base.iter().flatten().filter(hit).cloned().map(EpicRow::Epic));
        rows
    }
}

/// The child Tickets of an Epic, for its zoom.
#[derive(Debug, Clone, PartialEq)]
pub enum EpicChildren {
    Loading,
    Loaded { tickets: Vec<crate::core::model::TicketSummary>, total: u64 },
    Failed(String),
}
/// The number of Boards that jui keeps in memory (the Boards tab).
pub const BOARD_CACHE: usize = 10;

/// The Boards tab: My Boards, the selected Board, its columns, and the selected card.
#[derive(Debug, Default)]
pub struct BoardsTab {
    /// The index of the shown Board in My Boards.
    pub index: usize,
    /// The Board that the tab shows (or shows dim while a different Board loads).
    pub view: Option<crate::core::model::BoardView>,
    /// The ID of the Board that jui is getting now.
    pub loading: Option<u64>,
    /// The generation of the last Board request. Responses of older generations are ignored.
    pub generation: u64,
    /// The view is from a different Board, and a new Board is loading.
    pub stale: bool,
    /// The selected column.
    pub col: usize,
    /// The selected card in each column.
    pub rows: Vec<usize>,
    /// After a refresh, select this key again if the Board still has it.
    pub keep_key: Option<String>,
    /// Boards that Jira did not find (HTTP 404), with the message.
    pub missing: HashMap<u64, String>,
    /// The error of the last request for a Board.
    pub error: Option<(u64, String)>,
    /// The Boards that jui got before, with the most recently used last.
    pub cache: Vec<crate::core::model::BoardView>,
    /// Show only the Tickets that are assigned to the current user (`boards.only_mine`).
    pub only_mine: bool,
}

impl BoardsTab {
    /// The selected card of the shown Board.
    pub fn selected_key(&self) -> Option<&str> {
        if self.stale {
            return None;
        }
        let v = self.view.as_ref()?;
        let col = v.columns.get(self.col)?;
        let row = self.rows.get(self.col).copied().unwrap_or(0);
        col.tickets.get(row).map(|t| t.key.as_str())
    }

    /// Show a Board: select the card with `keep` (or the first card of the first column that has cards).
    pub fn show(&mut self, v: crate::core::model::BoardView, keep: Option<&str>) {
        self.rows = vec![0; v.columns.len()];
        self.col = v.columns.iter().position(|c| !c.tickets.is_empty()).unwrap_or(0);
        if let Some(k) = keep {
            for (ci, c) in v.columns.iter().enumerate() {
                if let Some(ri) = c.tickets.iter().position(|t| t.key == k) {
                    self.col = ci;
                    self.rows[ci] = ri;
                }
            }
        }
        // One view for each Board and each filter (all Tickets, only mine).
        self.cache.retain(|c| c.board.id != v.board.id || c.only_mine != v.only_mine);
        self.cache.push(v.clone());
        if self.cache.len() > BOARD_CACHE {
            self.cache.remove(0);
        }
        self.view = Some(v);
        self.stale = false;
    }

    /// A Board in the memory cache, with the current filter (all Tickets or only mine).
    pub fn cached(&self, id: u64) -> Option<&crate::core::model::BoardView> {
        self.cache.iter().find(|v| v.board.id == id && v.only_mine == self.only_mine)
    }

    /// True when the shown Board has the Ticket.
    pub fn has_key(&self, key: &str) -> bool {
        self.view.as_ref().is_some_and(|v| v.columns.iter().any(|c| c.tickets.iter().any(|t| t.key == key)))
    }
}

/// One row of the Board picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardPickRow {
    pub board: crate::core::model::Board,
    /// "mine", "recent", or "" (a project Board or a search result).
    pub tag: &'static str,
}

/// The Board picker (`+` on the Boards tab): a text input and a list. With no text, it
/// shows My Boards and the suggestions. With text, it searches all Boards on the server.
/// `Space` marks a Board, `Enter` saves the marked Boards as My Boards.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BoardPopup {
    pub input: String,
    /// The suggestions (Recent Boards, then the Boards of the default project). `None` while jui gets them.
    pub suggestions: Option<Vec<crate::core::board::Suggestion>>,
    /// The server results for the current text. `None` before the first result.
    pub results: Option<Vec<crate::core::model::Board>>,
    /// The number of the last search. A result with a different number is ignored.
    pub seq: u64,
    pub searching: bool,
    pub selected: usize,
    /// The marked Boards, in the order of My Boards (new Boards at the end).
    pub marked: Vec<crate::core::settings::BoardRef>,
    /// My Boards when the popup opened.
    pub mine: Vec<crate::core::settings::BoardRef>,
    pub saving: bool,
    pub error: Option<String>,
}

impl BoardPopup {
    pub fn new(mine: &[crate::core::settings::BoardRef]) -> Self {
        BoardPopup { marked: mine.to_vec(), mine: mine.to_vec(), ..Default::default() }
    }

    /// True when the text is long enough for a search on the server.
    pub fn searches(&self) -> bool {
        self.input.trim().chars().count() >= USER_SEARCH_MIN
    }

    pub fn is_marked(&self, id: u64) -> bool {
        self.marked.iter().any(|r| r.id == id)
    }

    /// Mark or unmark a Board.
    pub fn toggle(&mut self, b: &crate::core::model::Board) {
        if self.is_marked(b.id) {
            self.marked.retain(|r| r.id != b.id);
        } else {
            self.marked.push(crate::core::board::board_ref(b));
        }
    }

    /// The rows. With a short text: My Boards, then the suggestions, filtered in memory.
    /// With a longer text: the server results.
    pub fn rows(&self) -> Vec<BoardPickRow> {
        use crate::core::model::Board;
        if self.searches() {
            return self.results.iter().flatten().map(|b| BoardPickRow { board: b.clone(), tag: "" }).collect();
        }
        let mut rows: Vec<BoardPickRow> = Vec::new();
        let mine = self.mine.iter().map(|r| {
            let name = r.name.clone().unwrap_or_else(|| format!("Board {}", r.id));
            let kind = self.suggestions.iter().flatten().find(|s| s.board.id == r.id).map(|s| s.board.kind.clone());
            BoardPickRow { board: Board { id: r.id, name, kind: kind.unwrap_or_default() }, tag: "mine" }
        });
        let sugg = self.suggestions.iter().flatten().map(|s| BoardPickRow {
            board: s.board.clone(),
            tag: if s.origin == crate::core::board::Origin::Recent { "recent" } else { "" },
        });
        for r in mine.chain(sugg) {
            if !rows.iter().any(|x| x.board.id == r.board.id) && crate::core::board::matches(&r.board, &self.input) {
                rows.push(r);
            }
        }
        rows
    }

    /// True when the marked Boards are not the same as My Boards.
    pub fn changed(&self) -> bool {
        self.marked != self.mine
    }
}
/// The number of JQL queries that the Search tab keeps in its history (this session only).
pub const HISTORY_SIZE: usize = 20;

/// The JQL input of the Search tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchBox {
    pub input: String,
    /// The cursor position, in characters.
    pub cursor: usize,
    /// True when the keys go to the input.
    pub focus: bool,
    /// The queries that ran, most recent first.
    pub history: Vec<String>,
    /// The position in the history while the user pushes ↑ and ↓. `None`: the user's own text.
    pub history_pos: Option<usize>,
    /// The text of the user before the first ↑.
    pub draft: String,
    /// The name of the filter that the list shows. `None` for a JQL that the user typed.
    pub filter: Option<String>,
    /// The error of the last query (for example, a JQL error). It shows below the input.
    pub error: Option<String>,
}

impl SearchBox {
    pub fn set_text(&mut self, text: &str) {
        self.input = text.to_string();
        self.cursor = self.input.chars().count();
    }

    /// The byte index of the cursor.
    pub fn byte_at(&self, chars: usize) -> usize {
        self.input.char_indices().nth(chars).map(|(i, _)| i).unwrap_or(self.input.len())
    }

    pub fn insert(&mut self, c: char) {
        let i = self.byte_at(self.cursor);
        self.input.insert(i, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let i = self.byte_at(self.cursor);
            self.input.remove(i);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.input.chars().count() {
            let i = self.byte_at(self.cursor);
            self.input.remove(i);
        }
    }

    /// Put a query first in the history, with no duplicates.
    pub fn remember(&mut self, jql: &str) {
        let jql = jql.trim();
        if jql.is_empty() {
            return;
        }
        self.history.retain(|h| h != jql);
        self.history.insert(0, jql.to_string());
        self.history.truncate(HISTORY_SIZE);
    }
}

/// The `Ctrl-F` popup: the favourite filters, with a text that filters them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterPopup {
    pub input: String,
    /// `None` while jui gets the favourite filters.
    pub list: Option<Vec<crate::core::model::Filter>>,
    /// The index in the shown list.
    pub selected: usize,
    /// jui gets the filter with this ID (the user typed an ID).
    pub loading_id: Option<String>,
    pub error: Option<String>,
}

impl FilterPopup {
    /// The indexes of the favourite filters that match the text.
    pub fn shown(&self) -> Vec<usize> {
        match &self.list {
            Some(fs) => fs
                .iter()
                .enumerate()
                .filter(|(_, f)| crate::core::filter::name_matches(f, &self.input))
                .map(|(i, _)| i)
                .collect(),
            None => Vec::new(),
        }
    }

    /// The ID that the user typed, when the text is a number.
    pub fn typed_id(&self) -> Option<String> {
        match crate::core::filter::FilterArg::parse(&self.input) {
            crate::core::filter::FilterArg::Id(id) => Some(id),
            _ => None,
        }
    }
}

/// A list of Tickets from a JQL query, with infinite scroll.
#[derive(Debug, Default)]
pub struct TicketList {
    pub jql: String,
    pub rows: Vec<ListRow>,
    /// The total from the server. `None` before the first page.
    pub total: Option<u64>,
    pub selected: usize,
    /// The generation of the current query. Responses of older generations are ignored.
    pub generation: u64,
    /// A page request is running.
    pub loading: bool,
    /// The rows are from an old query, and a new query is running.
    pub stale: bool,
    /// The last request had a network error or a timeout.
    pub offline: bool,
    /// After a refresh, select this key again if it is in the new rows.
    pub keep_key: Option<String>,
    /// Rows that changed and do not match the query any more. They stay (dim) until the next refresh.
    pub gone: HashSet<String>,
    /// The scroll position of the table (the view writes it).
    pub table: TableState,
    /// Show the rows in groups by Sprint (My Tickets with `tickets.group_by_sprint`). The rows
    /// are in group order; `sprint_groups` gives the heading rows.
    /// `regroup` only changes the order, so `rows.len()` stays the number of fetched rows.
    pub by_sprint: bool,
}

impl TicketList {
    pub fn new(jql: &str) -> Self {
        TicketList { jql: jql.to_string(), ..Default::default() }
    }

    /// Put the rows in Sprint group order, and keep the selected Ticket selected.
    pub fn regroup(&mut self) {
        if !self.by_sprint {
            return;
        }
        let key = self.selected_key().map(str::to_string);
        crate::core::agile::sort_by_sprint(&mut self.rows, |r| r.ticket.sprint.as_ref());
        if let Some(k) = key {
            self.selected = self.rows.iter().position(|r| r.ticket.key == k).unwrap_or(self.selected);
        }
    }

    /// The Sprint groups of the rows. Empty when the list is not grouped.
    pub fn sprint_groups(&self) -> Vec<crate::core::agile::SprintGroup> {
        if !self.by_sprint {
            return Vec::new();
        }
        crate::core::agile::sprint_groups(&self.rows, |r| r.ticket.sprint.as_ref())
    }

    pub fn has_more(&self) -> bool {
        self.total.is_some_and(|t| (self.rows.len() as u64) < t)
    }

    pub fn selected_key(&self) -> Option<&str> {
        self.rows.get(self.selected).map(|r| r.ticket.key.as_str())
    }

    /// The count text for the list title, for example `100 of 495`.
    pub fn count(&self) -> Option<String> {
        let total = self.total?;
        if self.has_more() { Some(format!("{} of {total}", self.rows.len())) } else { Some(total.to_string()) }
    }
}

/// One step of the zoom: a Ticket at full width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoomFrame {
    pub key: String,
    /// The selected Sub-task or Link (Sub-tasks first, then Links).
    pub item: usize,
    pub scroll: u16,
    /// When true, the view scrolls to keep the selected item visible.
    pub follow: bool,
}

impl ZoomFrame {
    pub fn new(key: &str) -> Self {
        ZoomFrame { key: key.to_string(), item: 0, scroll: 0, follow: true }
    }
}

#[derive(Debug, Default)]
pub struct Tab {
    /// `None` when a later build ticket makes the content of the tab.
    pub list: Option<TicketList>,
    pub zoom: Vec<ZoomFrame>,
    pub preview_scroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusMsg {
    pub text: String,
    pub kind: MsgKind,
    pub ticks_left: u32,
}

/// One entry of the `!` popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedError {
    /// What jui tried to do, for example "Get page 1 of My Tickets".
    pub context: String,
    pub kind: &'static str,
    pub message: String,
    pub hint: Option<String>,
}

/// The Transition popup: the Transitions of one Ticket, with a filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionPopup {
    pub key: String,
    /// `None` while jui gets the Transitions.
    pub list: Option<Vec<Transition>>,
    pub filter: String,
    /// The index in the filtered list.
    pub selected: usize,
    pub sending: bool,
    pub error: Option<String>,
}

impl TransitionPopup {
    /// The indexes of the Transitions that match the filter.
    pub fn filtered(&self) -> Vec<usize> {
        match &self.list {
            Some(ts) => ts.iter().enumerate().filter(|(_, t)| filter_matches(t, &self.filter)).map(|(i, _)| i).collect(),
            None => Vec::new(),
        }
    }
}

/// The second popup: it asks for each required field of a Transition, one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldsPopup {
    pub key: String,
    pub transition: Transition,
    /// The fields that the popup asks for.
    pub fields: Vec<ScreenField>,
    /// The field that has the focus.
    pub index: usize,
    /// The text of each field. For a field with permitted values, it filters the values.
    pub inputs: Vec<String>,
    /// The selected permitted value (index in the filtered values).
    pub choice: usize,
    /// The value of each field after Enter (the shown text and the JSON).
    pub values: Vec<Option<(String, serde_json::Value)>>,
    /// The error below each field.
    pub errors: Vec<Option<String>>,
    /// An error that is not about one field.
    pub general: Option<String>,
    pub sending: bool,
}

impl FieldsPopup {
    pub fn new(key: &str, transition: Transition, fields: Vec<ScreenField>) -> Self {
        let n = fields.len();
        FieldsPopup {
            key: key.to_string(),
            transition,
            fields,
            index: 0,
            inputs: vec![String::new(); n],
            choice: 0,
            values: vec![None; n],
            errors: vec![None; n],
            general: None,
            sending: false,
        }
    }

    /// The indexes of the permitted values of the focused field that match its text.
    pub fn choices(&self) -> Vec<usize> {
        let f = &self.fields[self.index];
        let t = self.inputs[self.index].to_lowercase();
        f.allowed.iter().enumerate().filter(|(_, c)| c.name.to_lowercase().contains(&t)).map(|(i, _)| i).collect()
    }
}

/// The `n` and `N` popup: the create form. It shows the summary, the required fields, and the
/// frequent fields of the create screen, and a box for the description. Ctrl-S creates the
/// Ticket; Ctrl-E opens the full Template in the editor with the values of the form.
/// The `TemplateJob` (with the `Session`) stays in `State::job` while the form is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePopup {
    /// The popup title, for example "Create a Task in BCE".
    pub title: String,
    /// The fields of the form.
    pub fields: Vec<ScreenField>,
    /// The Template key of each field (the keys of the errors).
    pub keys: Vec<String>,
    /// The text of each field. For a list field, the values have commas between them.
    pub inputs: Vec<String>,
    /// The description in Markdown. It can have more than one line.
    pub description: String,
    /// The focused field. `fields.len()` is the description.
    pub focus: usize,
    /// The selected permitted value (index in `choices()`).
    pub choice: usize,
    /// True after ↑ or ↓ in the permitted values: Enter takes the selected value.
    pub browsing: bool,
    /// The error below each field.
    pub errors: Vec<Option<String>>,
    /// An error that is not about one field.
    pub general: Option<String>,
    pub sending: bool,
}

impl CreatePopup {
    pub fn new(title: String, fields: Vec<ScreenField>, keys: Vec<String>, inputs: Vec<String>) -> Self {
        let n = fields.len();
        CreatePopup {
            title,
            fields,
            keys,
            inputs,
            description: String::new(),
            focus: 0,
            choice: 0,
            browsing: false,
            errors: vec![None; n],
            general: None,
            sending: false,
        }
    }

    /// True when the description box has the focus.
    pub fn on_description(&self) -> bool {
        self.focus >= self.fields.len()
    }

    /// The number of focus positions: the fields and the description.
    pub fn stops(&self) -> usize {
        self.fields.len() + 1
    }

    /// True when the focused field is a list (the values have commas between them).
    pub fn is_list(&self) -> bool {
        use crate::core::model::FieldKind;
        self.fields.get(self.focus).is_some_and(|f| matches!(f.kind, FieldKind::MultiChoice | FieldKind::Labels))
    }

    /// The text that filters the permitted values: the full input, or for a list, the text after
    /// the last comma.
    pub fn filter_text(&self) -> String {
        let Some(input) = self.inputs.get(self.focus) else { return String::new() };
        let t = if self.is_list() { input.rsplit(',').next().unwrap_or("") } else { input.as_str() };
        t.trim().to_lowercase()
    }

    /// The indexes of the permitted values of the focused field that match the text. A list
    /// field does not show the values that it already has.
    pub fn choices(&self) -> Vec<usize> {
        let Some(f) = self.fields.get(self.focus) else { return Vec::new() };
        let t = self.filter_text();
        let taken: Vec<String> = if self.is_list() {
            let input = &self.inputs[self.focus];
            let mut parts: Vec<&str> = input.split(',').collect();
            parts.pop();
            parts.iter().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
        } else {
            Vec::new()
        };
        f.allowed
            .iter()
            .enumerate()
            .filter(|(_, c)| c.name.to_lowercase().contains(&t) && !taken.contains(&c.name.to_lowercase()))
            .map(|(i, _)| i)
            .collect()
    }

    /// Move the focus, and reset the selection of the permitted values.
    pub fn move_focus(&mut self, to: usize) {
        self.focus = to % self.stops();
        self.choice = 0;
        self.browsing = false;
    }

    /// The values of the form for `Session::submit_form`: the field keys and the texts.
    pub fn preset(&self) -> crate::core::template::Preset {
        use crate::core::template::{FlagValue, Preset};
        let values = self
            .keys
            .iter()
            .zip(&self.inputs)
            .filter(|(_, v)| !v.trim().trim_end_matches(',').trim().is_empty())
            .map(|(k, v)| (k.clone(), FlagValue::Text(v.trim().trim_end_matches(',').trim().to_string())))
            .collect();
        let d = self.description.trim();
        Preset { values, description: (!d.is_empty()).then(|| d.to_string()) }
    }

    /// True when the user typed something (a Draft keeps it on Esc).
    pub fn has_text(&self) -> bool {
        !self.description.trim().is_empty() || self.fields.iter().zip(&self.inputs).any(|(f, v)| f.id == "summary" && !v.trim().is_empty())
    }
}

/// What a Template does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateKind {
    /// Create a Ticket in the default project, with the default type.
    Create,
    /// Create a Sub-task of this parent.
    Subtask { parent: String },
    /// Create a Ticket in the default project, with the default type, in this Epic
    /// (`n` in the zoom of an Epic).
    InEpic { epic: String },
    /// Edit a Ticket. `wiki`: the body is raw wiki markup (`W`).
    Edit { key: String, wiki: bool },
}

/// A create or edit that is running. It stays in the state while the editor is open and
/// while the request runs.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateJob {
    pub kind: TemplateKind,
    pub session: crate::core::template::Session,
    /// Edit: the Ticket data when the editor opened, to find changes in Jira.
    pub before: Option<crate::core::model::EditData>,
    /// Create: the editor opened from the create form (Ctrl-E), and the form had text. A save
    /// with no change keeps the text in a Draft.
    pub from_form: bool,
}

/// A comment add or edit in the editor. It stays in the state while the editor is open
/// and while the request runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentJob {
    pub session: crate::core::comment::CommentSession,
    /// The request that waits for the diff popup.
    pub plan: Option<crate::core::comment::CommentPlan>,
}

/// How a comment job starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentStart {
    /// A new comment. `text` is the start of the body (from the `c` popup).
    Add { text: String },
    /// Edit comment `id`. `wiki`: the body is raw wiki markup (`W`).
    Edit { id: String, wiki: bool },
}

/// The `c` popup: a one-line comment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommentPopup {
    pub key: String,
    pub input: String,
    pub sending: bool,
    pub error: Option<String>,
}

/// One row of the user picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickRow {
    Me,
    Nobody,
    User(User),
}

impl PickRow {
    pub fn assignee(&self) -> Assignee {
        match self {
            PickRow::Me => Assignee::Me,
            PickRow::Nobody => Assignee::Nobody,
            PickRow::User(u) => Assignee::User(u.clone()),
        }
    }
}

/// A search needs at least this number of characters. See ticket 12, part 4.
pub const USER_SEARCH_MIN: usize = 2;
/// jui sends the search this time after the last key.
pub const USER_SEARCH_DELAY_MS: u64 = 250;
/// The number of users that a picker search gets.
pub const USER_SEARCH_MAX: u32 = 50;
/// The number of recent users that the picker shows with no text.
pub const RECENT_USERS: usize = 5;

/// The `a` popup: a text input and a list of users below it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserPopup {
    pub key: String,
    pub input: String,
    /// The server results for the current text. `None` before the first result.
    pub results: Option<Vec<User>>,
    /// The number of the last search. A result with a different number is ignored.
    pub seq: u64,
    pub searching: bool,
    pub selected: usize,
    pub sending: bool,
    pub error: Option<String>,
}

impl UserPopup {
    /// True when the text is long enough for a search on the server.
    pub fn searches(&self) -> bool {
        self.input.trim().chars().count() >= USER_SEARCH_MIN
    }

    /// The rows. With a short text: `me`, `none`, and the recent users that match.
    /// With a longer text: the server results (and `me` or `none` when the text is that word).
    pub fn rows(&self, recent: &[User]) -> Vec<PickRow> {
        let t = self.input.trim().to_lowercase();
        if !self.searches() {
            let mut rows = vec![PickRow::Me, PickRow::Nobody];
            rows.retain(|r| match r {
                PickRow::Me => "me".starts_with(&t),
                PickRow::Nobody => "none".starts_with(&t),
                PickRow::User(_) => true,
            });
            let hit = |u: &&User| u.name.to_lowercase().contains(&t) || u.display_name.to_lowercase().contains(&t);
            rows.extend(recent.iter().filter(hit).cloned().map(PickRow::User));
            return rows;
        }
        let mut rows = Vec::new();
        match UserArg::parse(&t) {
            UserArg::Me => rows.push(PickRow::Me),
            UserArg::Nobody => rows.push(PickRow::Nobody),
            UserArg::Text(_) => {}
        }
        rows.extend(self.results.iter().flatten().cloned().map(PickRow::User));
        rows
    }
}

/// A selectable item in the zoom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoomItem {
    /// A Sub-task or a Ticket of an Epic: Enter opens the Ticket.
    Ticket(String),
    /// A Link (its ID, and the key of the other Ticket): Enter opens the Ticket, `d` deletes the Link.
    Link { id: String, key: String },
    /// An attachment (its ID): Enter opens it in the browser, `d` deletes it.
    Attachment(String),
    /// A Worklog (its ID): `e` edits it, `d` deletes it.
    Worklog(String),
    /// A comment (its ID): `e` edits it, `d` deletes it.
    Comment(String),
}

/// The kind of item that the delete popup deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteKind {
    Comment,
    Worklog,
    Link,
    Attachment,
}

impl DeleteKind {
    pub fn name(self) -> &'static str {
        match self {
            DeleteKind::Comment => "comment",
            DeleteKind::Worklog => "Worklog",
            DeleteKind::Link => "Link",
            DeleteKind::Attachment => "attachment",
        }
    }
}

/// The `l` popup: select a relation, then type the key of the other Ticket.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkPopup {
    pub key: String,
    /// The relation filter.
    pub filter: String,
    /// The selected row in the matching relations.
    pub selected: usize,
    /// The key of the other Ticket.
    pub target: String,
    /// False: the relation list has the focus. True: the key input has the focus.
    pub on_target: bool,
    pub sending: bool,
    pub error: Option<String>,
}

impl LinkPopup {
    /// The relations that match the filter.
    pub fn shown(&self, types: &[crate::core::model::LinkType]) -> Vec<crate::core::link::Relation> {
        crate::core::link::relations(types).into_iter().filter(|r| crate::core::link::relation_matches(r, &self.filter)).collect()
    }
}

/// The `u` popup: the path of a file to upload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UploadPopup {
    pub key: String,
    /// The path as the user typed or pasted it. The runner cleans it (`attachment::clean_path`).
    pub input: String,
    pub sending: bool,
    pub error: Option<String>,
}

/// The fields of the `L` popup.
pub const WORKLOG_FIELDS: [&str; 3] = ["Time spent", "Started", "Comment"];

/// The `L` popup: the time spent, the start, and a one-line comment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorklogPopup {
    pub key: String,
    /// The text of each field (see `WORKLOG_FIELDS`).
    pub inputs: [String; 3],
    pub focus: usize,
    /// An error for each field.
    pub errors: [Option<String>; 3],
    /// An error that is not about one field.
    pub general: Option<String>,
    pub sending: bool,
}

impl WorklogPopup {
    pub fn quick(&self) -> crate::core::worklog::Quick {
        crate::core::worklog::Quick {
            time_spent: self.inputs[0].clone(),
            started: self.inputs[1].clone(),
            comment: self.inputs[2].clone(),
            ..Default::default()
        }
    }
}

/// How a Worklog job in the editor starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorklogStart {
    /// A new Worklog, with the values of the `L` popup.
    Add(crate::core::worklog::Quick),
    /// Edit Worklog `id`.
    Edit { id: String },
}

/// A Worklog add or edit in the editor. It stays in the state while the editor is open
/// and while the request runs.
#[derive(Debug, Clone, PartialEq)]
pub struct WorklogJob {
    pub session: crate::core::worklog::WorklogSession,
    /// The request that waits for the recheck or for the diff popup.
    pub plan: Option<crate::core::worklog::WorklogPlan>,
}

/// The diff popup: it shows the change of the description (or of a comment) before jui sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPopup {
    pub key: String,
    /// What changes, for example "The description" or "Comment 7002".
    pub subject: String,
    pub lines: Vec<crate::core::template::DiffLine>,
    pub warnings: Vec<String>,
    /// The fields that also change.
    pub fields: Vec<String>,
    /// "Changed in Jira" warning, when the Ticket changed during the edit.
    pub conflict: Option<String>,
    pub scroll: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    Help { scroll: u16 },
    Errors { scroll: u16 },
    Transition(TransitionPopup),
    Fields(FieldsPopup),
    /// The create form (`n`, `N`): Ctrl-S creates, Ctrl-E opens the editor.
    Create(CreatePopup),
    /// The diff of a description change: `y` sends, `n` opens the editor again.
    Diff(DiffPopup),
    /// The Ticket changed in Jira during the edit, and only fields change: `y` sends, `n` keeps a Draft.
    Conflict { key: String, message: String },
    /// A one-line comment: Enter sends, `Ctrl-E` opens the editor.
    Comment(CommentPopup),
    /// The user picker for assign.
    User(UserPopup),
    /// The favourite filters (`Ctrl-F`).
    Filters(FilterPopup),
    /// The Board picker (`+` on the Boards tab).
    Boards(BoardPopup),
    /// Remove a Board from My Boards? `y` removes, `n` closes.
    ConfirmRemoveBoard { id: u64, name: String },
    /// Move the Ticket to a Sprint or to the backlog (`s` in the zoom).
    Sprint(SprintPopup),
    /// Set or remove the Epic (`E` in the zoom).
    Epic(EpicPopup),
    /// Add a Link (`l` in the zoom).
    Link(LinkPopup),
    /// Add a Worklog (`L` in the zoom).
    Worklog(WorklogPopup),
    /// Upload a file (`u` in the zoom).
    Upload(UploadPopup),
    /// Edit one Setting (Enter on the Settings page).
    Setting(SettingPopup),
    /// The config file is not valid after `E`.
    ConfigInvalid(ConfigInvalid),
    /// Delete a comment, a Worklog, a Link, or an attachment? `y` deletes, `n` closes.
    ConfirmDelete { what: DeleteKind, key: String, id: String, preview: String, sending: bool },
    /// An error that stops the TUI (no token, token not accepted). Only `q` works.
    Fatal { message: String, hint: Option<String> },
}

/// Full Tickets in memory, with the most recently used last.
#[derive(Debug, Default)]
pub struct TicketCache {
    map: HashMap<String, Ticket>,
    order: VecDeque<String>,
}

impl TicketCache {
    pub fn get(&self, key: &str) -> Option<&Ticket> {
        self.map.get(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn put(&mut self, t: Ticket) {
        self.touch(&t.key);
        self.map.insert(t.key.clone(), t);
        while self.order.len() > CACHE_SIZE {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    /// Mark a key as used now, so that it stays longer.
    pub fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }

    pub fn remove(&mut self, key: &str) {
        self.map.remove(key);
        self.order.retain(|k| k != key);
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Lines that the view made, kept until the input changes.
#[derive(Debug, Default)]
pub struct RenderCache {
    pub key: Option<(String, bool, Option<usize>, u16)>,
    pub lines: Vec<Line<'static>>,
    /// The first line of each selectable item.
    pub items: Vec<usize>,
}

#[derive(Debug)]
pub struct State {
    pub base_url: String,
    pub default_project: String,
    pub default_type: String,
    pub date_format: DateFormat,
    pub tabs: Vec<Tab>,
    pub tab: usize,
    pub cache: TicketCache,
    /// Keys of full Tickets that jui is getting now.
    pub pending: HashSet<String>,
    /// Keys whose last request failed, with the message.
    pub failed: HashMap<String, String>,
    pub status: Option<StatusMsg>,
    pub errors: VecDeque<LoggedError>,
    pub popup: Option<Popup>,
    pub width: u16,
    pub height: u16,
    /// The height of the body; the view writes it. Used for half-page scrolls.
    pub body_height: u16,
    pub spinner: usize,
    pub quit: bool,
    /// Tests set a fixed time, so that relative dates do not change.
    pub now: Option<DateTime<Utc>>,
    pub preview_render: RenderCache,
    pub zoom_render: RenderCache,
    /// The create or edit that is running.
    pub job: Option<TemplateJob>,
    /// A plan that waits for the diff popup or the conflict popup.
    pub pending_plan: Option<crate::core::template::Request>,
    /// The comment add or edit that is running in the editor.
    pub comment_job: Option<CommentJob>,
    /// The users of the last assign actions in this session, most recent first.
    pub recent_users: Vec<User>,
    /// Keys whose watch request runs now.
    pub watching: HashSet<String>,
    /// The JQL input of the Search tab.
    pub search: SearchBox,
    /// My Boards (the board list in the Settings).
    pub my_boards: Vec<crate::core::settings::BoardRef>,
    /// The config file, for the writes of My Boards.
    pub config_path: std::path::PathBuf,
    /// The Boards tab.
    pub boards: BoardsTab,
    /// The Sprint tab.
    pub sprint: SprintTab,
    /// The Epics tab.
    pub epics: EpicsTab,
    /// The open Sprints of each Board that the `s` popup got (this session only).
    pub open_sprints: HashMap<u64, Vec<crate::core::model::Sprint>>,
    /// The child Tickets of the Epics in the zoom.
    pub epic_children: HashMap<String, EpicChildren>,
    /// The Link types (this session only). `None` before the first `l`.
    pub link_types: Option<Vec<crate::core::model::LinkType>>,
    /// The Worklog add or edit that is running in the editor.
    pub worklog_job: Option<WorklogJob>,
    /// Tests set a fixed clock for the Worklog start times.
    pub clock: Option<crate::core::worklog::Clock>,
    /// All Settings with their sources, for the Settings page.
    pub settings: Settings,
    /// The Settings page.
    pub settings_page: SettingsPage,
    /// The config file is open in the editor (`E` on the Settings page).
    pub config_edit: bool,
    /// The Ticket types of the default project (this session only), for `default_type`.
    pub ticket_types: Option<(String, Vec<String>)>,
}

impl State {
    pub fn new(s: &Settings, width: u16, height: u16) -> Self {
        let mut tabs: Vec<Tab> = TAB_TITLES.iter().map(|_| Tab::default()).collect();
        tabs[MY_TICKETS].list = Some(TicketList { by_sprint: s.group_by_sprint, ..TicketList::new(&s.my_tickets_jql) });
        // The Search tab has no query at the start. jui does not run an empty JQL.
        tabs[SEARCH].list = Some(TicketList::new(""));
        // The Sprint tab gets its JQL when its Board arrives.
        tabs[SPRINT].list = Some(TicketList::new(""));
        // The Epics tab gets its JQL (the Epic keys) when the counts arrive.
        tabs[EPICS].list = Some(TicketList::new(""));
        let mut st = State {
            base_url: s.base_url.trim_end_matches('/').to_string(),
            default_project: s.default_project.clone(),
            default_type: s.default_type.clone(),
            date_format: s.date_format,
            tabs,
            tab: MY_TICKETS,
            cache: TicketCache::default(),
            pending: HashSet::new(),
            failed: HashMap::new(),
            status: None,
            errors: VecDeque::new(),
            popup: None,
            width,
            height,
            body_height: height.saturating_sub(2),
            spinner: 0,
            quit: false,
            now: None,
            preview_render: RenderCache::default(),
            zoom_render: RenderCache::default(),
            job: None,
            pending_plan: None,
            comment_job: None,
            recent_users: Vec::new(),
            watching: HashSet::new(),
            search: SearchBox::default(),
            my_boards: s.my_boards.clone(),
            config_path: s.path.clone(),
            boards: BoardsTab { only_mine: s.boards_only_mine, ..BoardsTab::default() },
            sprint: SprintTab { only_mine: s.sprint_only_mine, ..SprintTab::default() },
            epics: EpicsTab::default(),
            open_sprints: HashMap::new(),
            epic_children: HashMap::new(),
            link_types: None,
            worklog_job: None,
            clock: None,
            settings: s.clone(),
            settings_page: SettingsPage::default(),
            config_edit: false,
            ticket_types: None,
        };
        if let Some(w) = s.warnings.first() {
            st.set_status(MsgKind::Warning, format!("warning: {w}"));
        }
        st
    }

    /// The clock for the Worklog start times.
    pub fn clock(&self) -> crate::core::worklog::Clock {
        self.clock.unwrap_or_else(crate::core::worklog::Clock::system)
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.now.unwrap_or_else(Utc::now)
    }

    pub fn set_status(&mut self, kind: MsgKind, text: impl Into<String>) {
        self.status = Some(StatusMsg { text: text.into(), kind, ticks_left: STATUS_TICKS });
    }

    pub fn current(&self) -> &Tab {
        &self.tabs[self.tab]
    }

    pub fn current_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.tab]
    }

    /// True when any request runs (the status line shows a spinner).
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
            || self.boards.loading.is_some()
            || self.sprint.loading
            || self.epics.loading
            || self.epic_children.values().any(|c| *c == EpicChildren::Loading)
            || self.tabs.iter().any(|t| t.list.as_ref().is_some_and(|l| l.loading))
    }

    pub fn preview_visible(&self) -> bool {
        self.width >= NARROW
    }

    pub fn browse_url(&self, key: &str) -> String {
        format!("{}/browse/{key}", self.base_url)
    }

    /// The child Tickets of an Epic, when jui has them.
    pub fn children_of(&self, key: &str) -> &[crate::core::model::TicketSummary] {
        match self.epic_children.get(key) {
            Some(EpicChildren::Loaded { tickets, .. }) => tickets,
            _ => &[],
        }
    }

    /// The selectable items of the zoom: Sub-tasks, the Tickets of an Epic, Links, attachments,
    /// Worklogs, then comments (the order of the view).
    pub fn zoom_items(&self, key: &str) -> Vec<ZoomItem> {
        match self.cache.get(key) {
            Some(t) => t
                .subtasks
                .iter()
                .map(|s| ZoomItem::Ticket(s.key.clone()))
                .chain(self.children_of(key).iter().map(|c| ZoomItem::Ticket(c.key.clone())))
                .chain(t.links.iter().map(|l| ZoomItem::Link { id: l.id.clone(), key: l.ticket.key.clone() }))
                .chain(t.attachments.iter().map(|a| ZoomItem::Attachment(a.id.clone())))
                .chain(t.worklogs.iter().map(|w| ZoomItem::Worklog(w.id.clone())))
                .chain(t.comments.iter().map(|c| ZoomItem::Comment(c.id.clone())))
                .collect(),
            None => Vec::new(),
        }
    }

    /// The zoomed Ticket and the selected comment, when the selected zoom item is a comment.
    pub fn selected_comment(&self) -> Option<(String, String)> {
        let z = self.current().zoom.last()?;
        match self.zoom_items(&z.key).get(z.item) {
            Some(ZoomItem::Comment(id)) => Some((z.key.clone(), id.clone())),
            _ => None,
        }
    }

    /// The zoomed Ticket and the selected zoom item.
    /// The key of the Epic in the top zoom frame, if that Ticket is an Epic.
    pub fn zoom_epic(&self) -> Option<String> {
        let z = self.current().zoom.last()?;
        self.cache.get(&z.key).filter(|t| t.kind.eq_ignore_ascii_case("epic")).map(|_| z.key.clone())
    }

    pub fn zoom_is_epic(&self) -> bool {
        self.zoom_epic().is_some()
    }

    pub fn selected_item(&self) -> Option<(String, ZoomItem)> {
        let z = self.current().zoom.last()?;
        self.zoom_items(&z.key).get(z.item).map(|i| (z.key.clone(), i.clone()))
    }

    /// True when a create, an edit, a comment, or a Worklog runs in the editor.
    pub fn job_running(&self) -> bool {
        self.job.is_some() || self.comment_job.is_some() || self.worklog_job.is_some() || self.config_edit
    }

    /// The selectable rows of the Settings page: the Settings, then each Board of My Boards,
    /// then the row that opens the Board picker.
    pub fn settings_rows(&self) -> Vec<SettingsRow> {
        crate::core::settings::PAGE
            .iter()
            .map(SettingsRow::Value)
            .chain((0..self.my_boards.len()).map(SettingsRow::Board))
            .chain(std::iter::once(SettingsRow::AddBoards))
            .collect()
    }

    pub fn selected_setting(&self) -> Option<SettingsRow> {
        self.settings_rows().get(self.settings_page.selected).copied()
    }

    /// Use new Settings: copy the values that the TUI uses. Returns true when the My Tickets
    /// JQL changed (the caller loads the tab again).
    pub fn apply_settings(&mut self, s: Settings) -> bool {
        let jql_changed = s.my_tickets_jql != self.settings.my_tickets_jql;
        if s.default_project != self.default_project {
            self.ticket_types = None;
        }
        self.default_project = s.default_project.clone();
        self.default_type = s.default_type.clone();
        self.date_format = s.date_format;
        self.my_boards = s.my_boards.clone();
        self.config_path = s.path.clone();
        if let Some(list) = self.tabs[MY_TICKETS].list.as_mut() {
            if jql_changed {
                list.jql = s.my_tickets_jql.clone();
            }
            // The rows have the Sprint data: "on" groups them at once. "off" needs the query
            // order again, so `use_settings` loads the tab again.
            if list.by_sprint != s.group_by_sprint {
                list.by_sprint = s.group_by_sprint;
                list.regroup();
            }
        }
        self.settings = s;
        let n = self.settings_rows().len();
        self.settings_page.selected = self.settings_page.selected.min(n.saturating_sub(1));
        jql_changed
    }
}
