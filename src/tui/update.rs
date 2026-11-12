//! The pure `update(state, event) -> Vec<Action>` function. It does no I/O.
//! The runner sends the events and does the actions.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::keys::{self, Cmd, Ctx};
use super::state::*;
use super::state::{
    CommentJob, CommentPopup, CommentStart, DeleteKind, DiffPopup, LinkPopup, TemplateJob, TemplateKind, UploadPopup, UserPopup, WorklogJob,
    WorklogPopup, WorklogStart, ZoomItem,
};
use serde_json::{Map, Value};

use crate::core::Error;
use crate::core::board::{self as bd, Suggestion};
use crate::core::comment::{self as cm, CommentSession, CommentStep};
use crate::core::agile::{self as ag, Target};
use crate::core::link::{self as lk, LinkPlan};
use crate::core::worklog::{self as wl, WorklogPlan, WorklogSession, WorklogStep};
use crate::core::model::{
    Attachment, Board, BoardView, Comment, CommentContext, CreateMeta, EditData, Epic, FieldKind, Filter, LinkType, ListRow, Page, Sprint, SprintBoard,
    SprintRef, Ticket, TicketSummary, Transition, User, Worklog, WorklogContext,
};
use crate::core::settings::{self as cfg, BoardRef, Kind, Settings, Source};
use crate::core::user::{self as us, Assignee};
use crate::core::template::{self as tpl, Preset, Request, Session, Step};
use crate::core::transition as tr;

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,
    /// A page of a Ticket list arrived (or failed).
    Page { tab: usize, generation: u64, start_at: u64, result: Result<Page<ListRow>, Error> },
    /// A full Ticket arrived (or failed).
    Ticket { key: String, result: Box<Result<Ticket, Error>> },
    /// The preview delay for this key is over.
    PreviewDue { key: String },
    /// An action of the runner failed (for example, the browser did not start).
    Failed { context: String, error: Error },
    /// The Transitions of a Ticket arrived (or failed).
    Transitions { key: String, result: Result<Vec<Transition>, Error> },
    /// A Transition was done (or failed).
    Moved { key: String, to: String, result: Result<(), Error> },
    /// The answer to "does the Ticket still match the query of the list?".
    Matched { tab: usize, key: String, result: Result<bool, Error> },
    /// The create screen and the current user arrived (or failed).
    CreateReady { kind: TemplateKind, result: Box<Result<(CreateMeta, String), Error>> },
    /// The edit screen, the current values, and the current user arrived (or failed).
    EditReady { kind: TemplateKind, result: Box<Result<(EditData, String), Error>> },
    /// The editor closed. The text is the saved Template.
    Edited { result: Result<String, Error> },
    /// The Ticket data again, just before jui sends an edit.
    Rechecked { key: String, result: Box<Result<EditData, Error>> },
    /// A Ticket was created (or the request failed).
    Created { result: Result<String, Error> },
    /// An edit was sent (or failed).
    Saved { key: String, changed: Vec<String>, result: Result<(), Error> },
    /// A Draft was written (or not). `reason` tells why jui stopped. `command` is the
    /// `jui` command that uses the Draft again.
    DraftWritten { reason: String, command: String, result: Result<std::path::PathBuf, Error> },
    /// The summary and the comments of a Ticket arrived, for a comment Template.
    CommentReady { key: String, start: CommentStart, result: Box<Result<CommentContext, Error>> },
    /// A comment was added or changed (or the request failed). `id` is `None` for a new comment.
    CommentSent { key: String, id: Option<String>, result: Box<Result<Comment, Error>> },
    /// The comment again, just before jui sends an edit.
    CommentRechecked { key: String, id: String, result: Box<Result<Comment, Error>> },
    /// A comment was deleted (or the request failed).
    CommentDeleted { key: String, id: String, result: Result<(), Error> },
    /// The search delay of the user picker is over. `seq` is the number of the search.
    UserSearchDue { seq: u64 },
    /// The users of a picker search arrived (or failed).
    Users { key: String, seq: u64, result: Result<Vec<User>, Error> },
    /// An assign was done (or failed).
    Assigned { key: String, who: Assignee, result: Result<(), Error> },
    /// A watch or unwatch was done (or failed). `on`: watch.
    Watched { key: String, on: bool, result: Result<(), Error> },
    /// The key was copied to the clipboard (or not). `via` tells how.
    Copied { text: String, result: Result<&'static str, Error> },
    /// The favourite filters arrived (or failed).
    FavouriteFilters { result: Result<Vec<Filter>, Error> },
    /// One filter by its ID arrived (or failed).
    FilterLoaded { id: String, result: Result<Filter, Error> },
    /// A Board with its columns and Tickets arrived (or failed).
    BoardLoaded { id: u64, generation: u64, result: Box<Result<BoardView, Error>> },
    /// The suggestions of the Board picker arrived (or failed).
    BoardSuggestions { result: Result<Vec<Suggestion>, Error> },
    /// The search delay of the Board picker is over.
    BoardSearchDue { seq: u64 },
    /// The Boards of a picker search arrived (or failed).
    BoardsFound { seq: u64, result: Result<Vec<Board>, Error> },
    /// My Boards was saved to the Settings (or not).
    BoardsSaved { list: Vec<BoardRef>, result: Result<(), Error> },
    /// The Sprint tab data of a Board arrived (or failed).
    SprintBoardLoaded { id: u64, generation: u64, result: Box<Result<SprintBoard, Error>> },
    /// The Epics of the user with the number of open Tickets in each arrived (or failed).
    MyEpicCounts { generation: u64, result: Result<Vec<(String, usize)>, Error> },
    /// The open Sprints of a Board arrived (or failed), for the `s` popup of a Ticket.
    OpenSprints { board_id: u64, key: String, result: Result<Vec<Sprint>, Error> },
    /// A Sprint move was done (or failed).
    SprintMoved { key: String, target: Target, result: Result<(), Error> },
    /// The search delay of the Epic picker is over.
    EpicSearchDue { seq: u64 },
    /// The Epics of a picker search arrived (or failed). `seq` 0 is the list with no text.
    Epics { key: String, seq: u64, result: Result<Vec<Epic>, Error> },
    /// The Epic of a Ticket was set or removed (or the request failed). `old` is the Epic before.
    EpicSet { key: String, epic: Option<Epic>, old: Option<String>, result: Result<(), Error> },
    /// The child Tickets of an Epic arrived (or failed).
    EpicChildren { key: String, result: Result<Page<TicketSummary>, Error> },
    /// The Link types arrived (or failed).
    LinkTypes { result: Result<Vec<LinkType>, Error> },
    /// A Link was added (or the request failed).
    LinkAdded { plan: LinkPlan, result: Result<(), Error> },
    /// A Link was deleted (or the request failed).
    LinkDeleted { key: String, id: String, result: Result<(), Error> },
    /// The time tracking and the Worklogs of a Ticket arrived, for a Worklog Template.
    WorklogReady { key: String, start: WorklogStart, result: Box<Result<WorklogContext, Error>> },
    /// A Worklog was added or changed (or the request failed). `id` is `None` for a new Worklog.
    WorklogSent { key: String, id: Option<String>, result: Box<Result<Worklog, Error>> },
    /// The Worklog again, just before jui sends an edit.
    WorklogRechecked { key: String, id: String, result: Box<Result<Worklog, Error>> },
    /// A Worklog was deleted (or the request failed).
    WorklogDeleted { key: String, id: String, result: Result<(), Error> },
    /// A file was uploaded (or the check or the request failed).
    Uploaded { key: String, result: Result<Vec<Attachment>, Error> },
    /// An attachment was deleted (or the request failed).
    AttachmentDeleted { key: String, id: String, result: Result<(), Error> },
    /// The project check of the Settings page is done. `Ok` has the key as Jira writes it.
    ProjectChecked { input: String, result: Result<String, Error> },
    /// The Ticket types of a project arrived (or failed), for the `default_type` popup.
    TicketTypes { project: String, result: Result<Vec<String>, Error> },
    /// A Setting was saved to the file (or not). `settings`: the file read again.
    SettingSaved { key: String, result: Result<Settings, Error> },
    /// The config file closed in the editor (`E`). `original`: the file text before the edit.
    ConfigEdited { original: String, result: Result<String, Error> },
    /// The config file was written after `E` (or not). `settings`: the file read again.
    ConfigSaved { result: Result<Settings, Error> },
    /// The config file was read again (`r` on the Settings page).
    SettingsReloaded { result: Result<Settings, Error> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Get one page of a list. A new generation cancels the requests of older generations.
    FetchPage { tab: usize, jql: String, start_at: u64, generation: u64 },
    /// Get one full Ticket.
    FetchTicket { key: String },
    /// Send `PreviewDue` after the preview delay.
    SchedulePreview { key: String },
    OpenBrowser { url: String },
    /// Get the Transitions of a Ticket.
    FetchTransitions { key: String },
    /// Do a Transition. This changes the Ticket in Jira.
    DoTransition { key: String, transition: Transition, fields: Map<String, Value> },
    /// Find out if a Ticket still matches the query of a list.
    CheckMatch { tab: usize, key: String, jql: String },
    /// Get the create screen of a project and a type, and the current user.
    PrepareCreate { kind: TemplateKind, project: String, issue_type: String },
    /// Get the edit screen of a Ticket, and the current user.
    PrepareEdit { kind: TemplateKind, key: String },
    /// Stop the terminal mode and open the editor on the text. `label` names the file.
    OpenEditor { text: String, label: String },
    /// Read the Ticket again before an edit, to find changes in Jira.
    Recheck { key: String },
    /// Create a Ticket. This changes Jira.
    SendCreate { body: Value },
    /// Edit a Ticket. This changes Jira.
    SendEdit { key: String, body: Value, changed: Vec<String> },
    /// Write a Draft.
    WriteDraft { label: String, text: String, reason: String, command: String },
    /// Get the summary and the comments of a Ticket for a comment Template.
    PrepareComment { key: String, start: CommentStart },
    /// Add a comment (`id` is `None`) or change one. This changes Jira.
    SendComment { key: String, id: Option<String>, wiki: String },
    /// Read a comment again before an edit, to find changes in Jira.
    RecheckComment { key: String, id: String },
    /// Delete a comment. This changes Jira.
    DeleteComment { key: String, id: String },
    /// Send `UserSearchDue` after the search delay.
    ScheduleUserSearch { seq: u64 },
    /// Search the assignable users of a Ticket.
    SearchUsers { key: String, text: String, seq: u64 },
    /// Set the assignee. This changes Jira.
    Assign { key: String, who: Assignee },
    /// Watch (`on`) or unwatch the Ticket for the current user. This changes Jira.
    Watch { key: String, on: bool },
    /// Copy the text to the clipboard.
    Copy { text: String },
    /// Get the favourite filters of the user.
    FetchFavouriteFilters,
    /// Get one filter by its ID.
    FetchFilter { id: String },
    /// Get a Board with its columns and Tickets. A new generation cancels the old request.
    /// Get a Board. `only_mine` keeps only the Tickets of the current user.
    FetchBoard { id: u64, generation: u64, only_mine: bool },
    /// Get the suggestions of the Board picker for the project.
    FetchBoardSuggestions { project: String },
    /// Send `BoardSearchDue` after the search delay.
    ScheduleBoardSearch { seq: u64 },
    /// Search all Boards by name.
    SearchBoards { text: String, seq: u64 },
    /// Write My Boards to the config file. This changes only the local file.
    SaveBoards { path: std::path::PathBuf, list: Vec<BoardRef> },
    /// Get the Sprint tab data of a Board. `sprints`: the active Sprints when the Boards tab
    /// has them (the Sprint request is slow). A new generation cancels the old request.
    FetchSprintBoard { id: u64, generation: u64, sprints: Option<Vec<SprintRef>> },
    /// Get the Epics of the user (Epic keys and the number of open Tickets in each).
    FetchMyEpicCounts { generation: u64 },
    /// Get the open Sprints of a Board for the `s` popup.
    FetchOpenSprints { board_id: u64, key: String },
    /// Move a Ticket to a Sprint or to the backlog. This changes Jira.
    MoveSprint { key: String, target: Target },
    /// Send `EpicSearchDue` after the search delay.
    ScheduleEpicSearch { seq: u64 },
    /// Search the open Epics of a project. `seq` 0 is the list with no text.
    SearchEpics { key: String, project: String, text: String, seq: u64 },
    /// Set (or remove, with `None`) the Epic of a Ticket. This changes Jira.
    SetEpic { key: String, epic: Option<Epic>, old: Option<String> },
    /// Get the child Tickets of an Epic.
    FetchEpicChildren { key: String },
    /// Get the Link types.
    FetchLinkTypes,
    /// Add a Link. This changes Jira.
    AddLink { plan: LinkPlan },
    /// Delete a Link. This changes Jira.
    DeleteLink { key: String, id: String },
    /// Get the time tracking and the Worklogs of a Ticket for a Worklog Template.
    PrepareWorklog { key: String, start: WorklogStart },
    /// Add a Worklog (`plan.id` is `None`) or change one. This changes Jira.
    SendWorklog { key: String, plan: WorklogPlan },
    /// Read a Worklog again before an edit, to find changes in Jira.
    RecheckWorklog { key: String, id: String },
    /// Delete a Worklog (the remaining estimate goes up by its time). This changes Jira.
    DeleteWorklog { key: String, id: String },
    /// Check the file and upload it. `path` is the text of the popup. This changes Jira.
    Upload { key: String, path: String },
    /// Delete an attachment. This changes Jira.
    DeleteAttachment { key: String, id: String },
    /// Check that a project exists (`GET /project/<key>`). This does not change Jira.
    CheckProject { input: String },
    /// Get the Ticket types of a project.
    FetchTicketTypes { project: String },
    /// Write one Setting to the config file (`value` `None`: remove the key), then read the
    /// file again. This changes only the local file.
    SaveSetting { key: String, value: Option<String> },
    /// Open the config file in the editor (`E`). `original`: the file text before the first
    /// edit (`None`: the runner reads the file). `text`: the text to edit (`None`: `original`).
    EditConfig { original: Option<String>, text: Option<String> },
    /// Write the text from the editor to the config file, then read it again.
    SaveConfigText { text: String },
    /// Read the config file again (`r` on the Settings page).
    ReloadSettings,
    /// The base URL, the timeout, or the editor command changed: make a new Jira client and
    /// use the new editor command.
    UseConnection { base_url: String, timeout_secs: u64, editor_command: String },
    Quit,
}

/// The actions at the start: get the first page of My Tickets.
pub fn init(st: &mut State) -> Vec<Action> {
    let mut out = Vec::new();
    load_first_page(st, MY_TICKETS, false, &mut out);
    out
}

/// Show an error that stops the TUI (for example, no token). Only `q` works after this.
pub fn fatal(st: &mut State, e: &Error) {
    st.popup = Some(Popup::Fatal { message: e.to_string(), hint: e.hint() });
}

pub fn update(st: &mut State, ev: Event) -> Vec<Action> {
    let mut out = Vec::new();
    match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => key(st, k, &mut out),
        Event::Key(_) => {}
        Event::Resize(w, h) => {
            st.width = w;
            st.height = h;
        }
        Event::Tick => tick(st),
        Event::Page { tab, generation, start_at, result } => page(st, tab, generation, start_at, result, &mut out),
        Event::Ticket { key, result } => ticket(st, key, *result, &mut out),
        Event::Failed { context, error } => report(st, &context, &error),
        Event::Transitions { key, result } => transitions(st, key, result),
        Event::Moved { key, to, result } => moved(st, key, to, result, &mut out),
        Event::Matched { tab, key, result } => {
            if let (Ok(m), Some(list)) = (result, st.tabs.get_mut(tab).and_then(|t| t.list.as_mut())) {
                if m {
                    list.gone.remove(&key);
                } else {
                    list.gone.insert(key);
                }
            }
        }
        Event::CreateReady { kind, result } => template_ready(st, kind, result.map(|(m, me)| Session::create(m, &me, &Preset::default())), None, &mut out),
        Event::EditReady { kind, result } => {
            let wiki = matches!(kind, TemplateKind::Edit { wiki: true, .. });
            let before = result.as_ref().as_ref().ok().map(|(d, _)| d.clone());
            template_ready(st, kind, result.map(|(d, me)| Session::edit(d, &me, wiki, &Preset::default())), before, &mut out)
        }
        Event::Edited { result } => edited(st, result, &mut out),
        Event::Rechecked { key, result } => rechecked(st, key, *result, &mut out),
        Event::Created { result } => created(st, result, &mut out),
        Event::Saved { key, changed, result } => saved(st, key, changed, result, &mut out),
        Event::DraftWritten { reason, command, result } => match result {
            Ok(path) => {
                let text = format!("{reason} Draft: {}", path.display());
                log_error(st, "Keep your text", &Error::input_hint(reason.clone(), tpl::draft_hint(&path, &command)));
                st.set_status(MsgKind::Warning, text);
            }
            Err(e) => report(st, "Write the Draft", &e),
        },
        Event::CommentReady { key, start, result } => comment_ready(st, key, start, *result, &mut out),
        Event::CommentSent { key, id, result } => comment_sent(st, key, id, *result, &mut out),
        Event::CommentRechecked { key, id, result } => comment_rechecked(st, key, id, *result, &mut out),
        Event::CommentDeleted { key, id, result } => deleted(st, DeleteKind::Comment, key, id, result, &mut out),
        Event::LinkTypes { result } => link_types(st, result),
        Event::LinkAdded { plan, result } => link_added(st, plan, result, &mut out),
        Event::LinkDeleted { key, id, result } => deleted(st, DeleteKind::Link, key, id, result, &mut out),
        Event::WorklogReady { key, start, result } => worklog_ready(st, key, start, *result, &mut out),
        Event::WorklogSent { key, id, result } => worklog_sent(st, key, id, *result, &mut out),
        Event::WorklogRechecked { key, id, result } => worklog_rechecked(st, key, id, *result, &mut out),
        Event::WorklogDeleted { key, id, result } => deleted(st, DeleteKind::Worklog, key, id, result, &mut out),
        Event::Uploaded { key, result } => uploaded(st, key, result, &mut out),
        Event::AttachmentDeleted { key, id, result } => deleted(st, DeleteKind::Attachment, key, id, result, &mut out),
        Event::UserSearchDue { seq } => user_search_due(st, seq, &mut out),
        Event::Users { key, seq, result } => users(st, key, seq, result),
        Event::Assigned { key, who, result } => assigned(st, key, who, result, &mut out),
        Event::Watched { key, on, result } => watched(st, key, on, result, &mut out),
        Event::Copied { text, result } => match result {
            Ok(via) => st.set_status(MsgKind::Info, format!("Copied {text} ({via}).")),
            Err(e) => report(st, &format!("Copy {text}"), &e),
        },
        Event::FavouriteFilters { result } => favourite_filters(st, result),
        Event::FilterLoaded { id, result } => filter_loaded(st, id, result, &mut out),
        Event::BoardLoaded { id, generation, result } => board_loaded(st, id, generation, *result, &mut out),
        Event::BoardSuggestions { result } => board_suggestions(st, result),
        Event::BoardSearchDue { seq } => {
            if let Some(Popup::Boards(p)) = &st.popup
                && p.seq == seq
                && p.searches()
                && !p.saving
            {
                out.push(Action::SearchBoards { text: p.input.trim().to_string(), seq });
            }
        }
        Event::BoardsFound { seq, result } => boards_found(st, seq, result),
        Event::BoardsSaved { list, result } => boards_saved(st, list, result, &mut out),
        Event::SprintBoardLoaded { id, generation, result } => sprint_board_loaded(st, id, generation, *result, &mut out),
        Event::MyEpicCounts { generation, result } => my_epic_counts(st, generation, result, &mut out),
        Event::OpenSprints { board_id, key, result } => open_sprints(st, board_id, key, result),
        Event::SprintMoved { key, target, result } => sprint_moved(st, key, target, result, &mut out),
        Event::EpicSearchDue { seq } => {
            if let Some(Popup::Epic(p)) = &st.popup
                && p.seq == seq
                && p.searches()
                && !p.sending
            {
                out.push(Action::SearchEpics { key: p.key.clone(), project: p.project.clone(), text: p.input.trim().to_string(), seq });
            }
        }
        Event::Epics { key, seq, result } => epics(st, key, seq, result),
        Event::EpicSet { key, epic, old, result } => epic_set(st, key, epic, old, result, &mut out),
        Event::EpicChildren { key, result } => {
            st.zoom_render.key = None;
            let v = match result {
                Ok(p) => EpicChildren::Loaded { total: p.total.max(p.items.len() as u64), tickets: p.items },
                Err(e) => {
                    log_error(st, &format!("Get the Tickets of Epic {key}"), &e);
                    EpicChildren::Failed(e.to_string())
                }
            };
            st.epic_children.insert(key, v);
        }
        Event::PreviewDue { key } => {
            if st.selected_key_any().as_deref() == Some(key.as_str()) {
                fetch_ticket(st, &key, &mut out);
            }
        }
        Event::ProjectChecked { input, result } => project_checked(st, input, result, &mut out),
        Event::TicketTypes { project, result } => ticket_types(st, project, result),
        Event::SettingSaved { key, result } => setting_saved(st, key, result, &mut out),
        Event::ConfigEdited { original, result } => config_edited(st, original, result, &mut out),
        Event::ConfigSaved { result } => config_saved(st, result, &mut out),
        Event::SettingsReloaded { result } => match result {
            Ok(s) => {
                let warning = s.warnings.first().cloned();
                use_settings(st, s, &mut out);
                match warning {
                    Some(w) => st.set_status(MsgKind::Warning, format!("warning: {w}")),
                    None => st.set_status(MsgKind::Info, format!("Read {} again.", st.config_path.display())),
                }
            }
            Err(e) => report(st, "Read the config file", &e),
        },
    }
    out
}

impl State {
    /// The selected key of the current tab: the zoomed Ticket, or the selected row.
    pub fn selected_key_any(&self) -> Option<String> {
        let tab = self.current();
        if let Some(z) = tab.zoom.last() {
            return Some(z.key.clone());
        }
        if self.tab == BOARDS {
            return self.boards.selected_key().map(str::to_string);
        }
        tab.list.as_ref().and_then(|l| l.selected_key()).map(str::to_string)
    }
}

fn tick(st: &mut State) {
    st.spinner = st.spinner.wrapping_add(1);
    if let Some(m) = &mut st.status {
        m.ticks_left = m.ticks_left.saturating_sub(1);
        if m.ticks_left == 0 {
            st.status = None;
        }
    }
}

// ------------------------------------------------------------------ keys

fn key(st: &mut State, k: KeyEvent, out: &mut Vec<Action>) {
    let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);

    if matches!(
        st.popup,
        Some(Popup::Transition(_))
            | Some(Popup::Fields(_))
            | Some(Popup::Create(_))
            | Some(Popup::Diff(_))
            | Some(Popup::Conflict { .. })
            | Some(Popup::Comment(_))
            | Some(Popup::User(_))
            | Some(Popup::Filters(_))
            | Some(Popup::Boards(_))
            | Some(Popup::ConfirmRemoveBoard { .. })
            | Some(Popup::Sprint(_))
            | Some(Popup::Epic(_))
            | Some(Popup::Link(_))
            | Some(Popup::Worklog(_))
            | Some(Popup::Upload(_))
            | Some(Popup::Setting(_))
            | Some(Popup::ConfigInvalid(_))
            | Some(Popup::ConfirmDelete { .. })
    ) {
        if ctrl_c {
            quit(st, out);
            return;
        }
        match st.popup.take() {
            Some(Popup::Setting(p)) => setting_popup_key(st, p, k, out),
            Some(Popup::ConfigInvalid(p)) => config_invalid_key(st, p, k, out),
            Some(Popup::Filters(p)) => filters_key(st, p, k, out),
            Some(Popup::Boards(p)) => board_popup_key(st, p, k, out),
            Some(Popup::ConfirmRemoveBoard { id, name }) => remove_board_key(st, id, name, k, out),
            Some(Popup::Sprint(p)) => sprint_popup_key(st, p, k, out),
            Some(Popup::Epic(p)) => epic_popup_key(st, p, k, out),
            Some(Popup::Transition(p)) => transition_key(st, p, k, out),
            Some(Popup::Fields(p)) => fields_key(st, p, k, out),
            Some(Popup::Create(p)) => create_key(st, p, k, out),
            Some(Popup::Diff(p)) => diff_key(st, p, k, out),
            Some(p @ Popup::Conflict { .. }) => conflict_key(st, p, k, out),
            Some(Popup::Comment(p)) => comment_key(st, p, k, out),
            Some(Popup::User(p)) => user_key(st, p, k, out),
            Some(p @ Popup::ConfirmDelete { .. }) => delete_key(st, p, k, out),
            Some(Popup::Link(p)) => link_key(st, p, k, out),
            Some(Popup::Worklog(p)) => worklog_key(st, p, k, out),
            Some(Popup::Upload(p)) => upload_key(st, p, k, out),
            _ => unreachable!(),
        }
        return;
    }
    if let Some(p) = &mut st.popup {
        match p {
            Popup::Transition(_)
            | Popup::Fields(_)
            | Popup::Create(_)
            | Popup::Diff(_)
            | Popup::Conflict { .. }
            | Popup::Comment(_)
            | Popup::User(_)
            | Popup::Filters(_)
            | Popup::Boards(_)
            | Popup::ConfirmRemoveBoard { .. }
            | Popup::Sprint(_)
            | Popup::Epic(_)
            | Popup::Link(_)
            | Popup::Worklog(_)
            | Popup::Upload(_)
            | Popup::Setting(_)
            | Popup::ConfigInvalid(_)
            | Popup::ConfirmDelete { .. } => unreachable!(),
            Popup::Fatal { .. } => {
                if ctrl_c || matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                    quit(st, out);
                }
            }
            Popup::Help { scroll } | Popup::Errors { scroll } => match k.code {
                KeyCode::Char('j') | KeyCode::Down => *scroll = scroll.saturating_add(1),
                KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
                _ if ctrl_c => quit(st, out),
                _ => st.popup = None,
            },
        }
        return;
    }

    // An error stays until the next key.
    if st.status.as_ref().is_some_and(|m| m.kind != MsgKind::Info) {
        st.status = None;
    }

    // The JQL input of the Search tab gets all keys when it has the focus.
    if st.tab == SEARCH && st.search.focus && st.current().zoom.is_empty() {
        if ctrl_c {
            quit(st, out);
        } else {
            search_key(st, k, out);
        }
        return;
    }

    let ctx = match (st.current().zoom.is_empty(), st.tab) {
        (false, _) => Ctx::Zoom,
        (true, BOARDS) => Ctx::Board,
        (true, SPRINT) => Ctx::Sprint,
        (true, SETTINGS) => Ctx::Settings,
        (true, _) => Ctx::Main,
    };
    let Some(b) = keys::lookup(&k, ctx) else { return };
    if !b.built {
        st.set_status(MsgKind::Warning, format!("`{}` ({}) is not built yet.", b.label, b.help));
        return;
    }
    // The Settings page has no Ticket. Only its own keys and the global keys work.
    if ctx == Ctx::Settings {
        match b.cmd {
            Cmd::Exit | Cmd::Help | Cmd::Errors | Cmd::SearchFocus | Cmd::Filters | Cmd::NextTab | Cmd::PrevTab | Cmd::Tab(_) => {}
            Cmd::Create => {}
            cmd => {
                settings_cmd(st, cmd, out);
                return;
            }
        }
    }
    match b.cmd {
        Cmd::Exit => quit(st, out),
        Cmd::Help => st.popup = Some(Popup::Help { scroll: 0 }),
        Cmd::Errors => {
            if st.errors.is_empty() {
                st.set_status(MsgKind::Info, "No errors.");
            } else {
                st.popup = Some(Popup::Errors { scroll: 0 });
            }
        }
        Cmd::SearchFocus => {
            switch_tab(st, SEARCH, out);
            st.tabs[SEARCH].zoom.clear();
            st.search.focus = true;
            st.search.history_pos = None;
        }
        Cmd::Filters => {
            switch_tab(st, SEARCH, out);
            st.tabs[SEARCH].zoom.clear();
            open_filters(st, out);
        }
        Cmd::NextTab => switch_tab(st, (st.tab + 1) % TAB_TITLES.len(), out),
        Cmd::PrevTab => switch_tab(st, (st.tab + TAB_TITLES.len() - 1) % TAB_TITLES.len(), out),
        Cmd::Tab(i) => switch_tab(st, i, out),
        Cmd::Transition => match st.selected_key_any() {
            Some(key) => {
                st.popup = Some(Popup::Transition(TransitionPopup {
                    key: key.clone(),
                    list: None,
                    filter: String::new(),
                    selected: 0,
                    sending: false,
                    error: None,
                }));
                out.push(Action::FetchTransitions { key });
            }
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        // `e` and `W` on a selected comment in the zoom edit the comment.
        Cmd::Edit | Cmd::EditWiki if st.selected_comment().is_some() && ctx == Ctx::Zoom => {
            let (key, id) = st.selected_comment().expect("checked");
            start_comment(st, key, CommentStart::Edit { id, wiki: b.cmd == Cmd::EditWiki }, out);
        }
        // `e` on a selected Worklog edits the Worklog.
        Cmd::Edit | Cmd::EditWiki if ctx == Ctx::Zoom && matches!(st.selected_item(), Some((_, ZoomItem::Worklog(_)))) => {
            let Some((key, ZoomItem::Worklog(id))) = st.selected_item() else { return };
            start_worklog(st, key, WorklogStart::Edit { id }, out);
        }
        Cmd::AddLink => match st.current().zoom.last().map(|z| z.key.clone()) {
            Some(key) => {
                st.popup = Some(Popup::Link(LinkPopup { key, ..Default::default() }));
                if st.link_types.is_none() {
                    out.push(Action::FetchLinkTypes);
                }
            }
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::AddWorklog => match st.current().zoom.last().map(|z| z.key.clone()) {
            Some(_) if st.job_running() => st.set_status(MsgKind::Warning, "A create or edit is running. Wait until it is done."),
            Some(key) => st.popup = Some(Popup::Worklog(WorklogPopup { key, ..Default::default() })),
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::Upload => match st.current().zoom.last().map(|z| z.key.clone()) {
            Some(key) => st.popup = Some(Popup::Upload(UploadPopup { key, ..Default::default() })),
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::Edit | Cmd::EditWiki | Cmd::Create | Cmd::CreateSubtask => start_template(st, b.cmd, out),
        Cmd::Comment => match st.selected_key_any() {
            Some(key) if st.job_running() => {
                let _ = key;
                st.set_status(MsgKind::Warning, "A create or edit is running. Wait until it is done.");
            }
            Some(key) => st.popup = Some(Popup::Comment(CommentPopup { key, ..Default::default() })),
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::Delete => start_delete(st),
        Cmd::Assign => match st.selected_key_any() {
            Some(key) => st.popup = Some(Popup::User(UserPopup { key, ..Default::default() })),
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::Watch => match st.selected_key_any() {
            Some(key) => start_watch(st, key, out),
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::Copy => match st.selected_key_any() {
            Some(key) => out.push(Action::Copy { text: key }),
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::Browser => match st.selected_key_any() {
            Some(key) => {
                let url = st.browse_url(&key);
                st.set_status(MsgKind::Info, format!("Opened {url}"));
                out.push(Action::OpenBrowser { url });
            }
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::MoveSprint => match st.selected_key_any() {
            Some(key) => start_sprint_move(st, key, out),
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        Cmd::SetEpic => match st.selected_key_any() {
            Some(key) => start_epic_pick(st, key, out),
            None => st.set_status(MsgKind::Warning, "No Ticket is selected."),
        },
        cmd if ctx == Ctx::Zoom => zoom_cmd(st, cmd, out),
        cmd if ctx == Ctx::Board => board_cmd(st, cmd, out),
        Cmd::PrevSection | Cmd::NextSection => change_section(st, b.cmd == Cmd::NextSection, out),
        Cmd::OnlyMine if ctx == Ctx::Sprint => {
            let on = !st.sprint.only_mine;
            out.push(Action::SaveSetting { key: "sprint.only_mine".into(), value: Some((if on { "on" } else { "off" }).into()) });
            set_sprint_only_mine(st, on, out);
            st.set_status(MsgKind::Info, if on { "Sprint: only my Tickets. m shows all." } else { "Sprint: all Tickets." });
        }
        Cmd::Refresh if st.tab == SPRINT => refresh_sprint_tab(st, out),
        Cmd::Refresh if st.tab == EPICS && st.current().zoom.is_empty() => {
            if let Some(key) = st.selected_key_any() {
                st.cache.remove(&key);
                st.failed.remove(&key);
            }
            load_my_epics(st, out);
        }
        cmd => main_cmd(st, cmd, out),
    }
}

fn quit(st: &mut State, out: &mut Vec<Action>) {
    st.quit = true;
    out.push(Action::Quit);
}

fn switch_tab(st: &mut State, i: usize, out: &mut Vec<Action>) {
    if i >= st.tabs.len() || i == st.tab {
        return;
    }
    st.tab = i;
    if i == BOARDS {
        boards_first_time(st, out);
        return;
    }
    if i == SPRINT {
        sprint_tab_visit(st, out);
        return;
    }
    if i == EPICS {
        // The first visit gets the Epics. Later visits keep the state; r gets them again.
        let list = st.tabs[EPICS].list.as_ref();
        if !st.epics.loading && list.is_some_and(|l| l.total.is_none() && !l.loading) {
            load_my_epics(st, out);
        }
        return;
    }
    // A tab keeps its state. It gets data only the first time.
    // After a JQL error, the Search tab waits for a corrected query.
    let bad_jql = i == SEARCH && st.search.error.is_some();
    let needs_load = !bad_jql && st.tabs[i].list.as_ref().is_some_and(|l| l.total.is_none() && !l.loading);
    if needs_load {
        load_first_page(st, i, false, out);
    }
}

fn half_page(st: &State) -> u16 {
    (st.body_height / 2).max(1)
}

fn main_cmd(st: &mut State, cmd: Cmd, out: &mut Vec<Action>) {
    let half = half_page(st);
    let tab = st.tab;
    match cmd {
        Cmd::Down => move_selection(st, |sel, _| sel + 1, out),
        Cmd::Up => move_selection(st, |sel, _| sel.saturating_sub(1), out),
        Cmd::First => move_selection(st, |_, _| 0, out),
        Cmd::Last => move_selection(st, |_, len| len.saturating_sub(1), out),
        Cmd::ScrollDown => st.current_mut().preview_scroll = st.current().preview_scroll.saturating_add(1),
        Cmd::ScrollUp => st.current_mut().preview_scroll = st.current().preview_scroll.saturating_sub(1),
        Cmd::PageDown => st.current_mut().preview_scroll = st.current().preview_scroll.saturating_add(half),
        Cmd::PageUp => st.current_mut().preview_scroll = st.current().preview_scroll.saturating_sub(half),
        Cmd::Open => {
            let Some(key) = st.current().list.as_ref().and_then(|l| l.selected_key()).map(str::to_string) else {
                return;
            };
            push_zoom(st, &key, out);
        }
        Cmd::Back => {}
        Cmd::Quit => quit(st, out),
        Cmd::Refresh => {
            if st.tabs[tab].list.is_none() {
                return;
            }
            if let Some(key) = st.selected_key_any() {
                st.cache.remove(&key);
                st.failed.remove(&key);
            }
            load_first_page(st, tab, true, out);
        }
        _ => {}
    }
}

fn zoom_cmd(st: &mut State, cmd: Cmd, out: &mut Vec<Action>) {
    let half = half_page(st);
    let Some(frame) = st.current().zoom.last().cloned() else { return };
    let items = st.zoom_items(&frame.key);
    let set = |st: &mut State, f: &dyn Fn(&mut ZoomFrame)| {
        if let Some(z) = st.current_mut().zoom.last_mut() {
            f(z);
        }
    };
    match cmd {
        Cmd::Down if !items.is_empty() => set(st, &|z| {
            z.item = (z.item + 1).min(items.len() - 1);
            z.follow = true;
        }),
        Cmd::Up if !items.is_empty() => set(st, &|z| {
            z.item = z.item.saturating_sub(1);
            z.follow = true;
        }),
        // With no Sub-tasks or Links, j and k scroll.
        Cmd::Down | Cmd::ScrollDown => set(st, &|z| {
            z.scroll = z.scroll.saturating_add(1);
            z.follow = false;
        }),
        Cmd::Up | Cmd::ScrollUp => set(st, &|z| {
            z.scroll = z.scroll.saturating_sub(1);
            z.follow = false;
        }),
        Cmd::PageDown => set(st, &|z| {
            z.scroll = z.scroll.saturating_add(half);
            z.follow = false;
        }),
        Cmd::PageUp => set(st, &|z| {
            z.scroll = z.scroll.saturating_sub(half);
            z.follow = false;
        }),
        Cmd::First => set(st, &|z| {
            z.scroll = 0;
            z.item = 0;
            z.follow = false;
        }),
        Cmd::Last => set(st, &|z| {
            // The view limits the scroll to the last page.
            z.scroll = u16::MAX;
            z.follow = false;
        }),
        Cmd::Open => match items.get(frame.item) {
            Some(ZoomItem::Ticket(key)) => {
                let key = key.clone();
                push_zoom(st, &key, out);
            }
            Some(ZoomItem::Link { key, .. }) => {
                let key = key.clone();
                push_zoom(st, &key, out);
            }
            Some(ZoomItem::Attachment(id)) => {
                let url = st.cache.get(&frame.key).and_then(|t| t.attachments.iter().find(|a| a.id == *id)).and_then(|a| a.url.clone());
                match url {
                    Some(url) => {
                        st.set_status(MsgKind::Info, format!("Opened {url}"));
                        out.push(Action::OpenBrowser { url });
                    }
                    None => st.set_status(MsgKind::Warning, "Jira gave no URL for this attachment."),
                }
            }
            Some(ZoomItem::Worklog(_)) => st.set_status(MsgKind::Info, "e edits the Worklog, d deletes it."),
            Some(ZoomItem::Comment(_)) => st.set_status(MsgKind::Info, "e edits the comment, W edits its wiki markup, d deletes it."),
            None => st.set_status(MsgKind::Info, "Select a Sub-task or a Link to open it."),
        },
        Cmd::Back | Cmd::Quit => {
            st.current_mut().zoom.pop();
            if let Some(key) = st.selected_key_any() {
                need_ticket(st, &key, false, out);
            }
        }
        Cmd::Refresh => {
            st.cache.remove(&frame.key);
            st.failed.remove(&frame.key);
            st.epic_children.remove(&frame.key);
            fetch_ticket(st, &frame.key, out);
        }
        _ => {}
    }
}

fn push_zoom(st: &mut State, key: &str, out: &mut Vec<Action>) {
    st.current_mut().zoom.push(ZoomFrame::new(key));
    need_ticket(st, key, true, out);
    need_epic_children(st, key, out);
}

/// Make sure that the full Ticket will be in the cache. `now` skips the preview delay.
fn need_ticket(st: &mut State, key: &str, now: bool, out: &mut Vec<Action>) {
    if st.cache.contains(key) {
        st.cache.touch(key);
        return;
    }
    if st.pending.contains(key) {
        return;
    }
    if now {
        fetch_ticket(st, key, out);
    } else {
        out.push(Action::SchedulePreview { key: key.to_string() });
    }
}

fn fetch_ticket(st: &mut State, key: &str, out: &mut Vec<Action>) {
    if st.cache.contains(key) || st.pending.contains(key) {
        return;
    }
    st.failed.remove(key);
    st.pending.insert(key.to_string());
    out.push(Action::FetchTicket { key: key.to_string() });
}

fn move_selection(st: &mut State, f: impl Fn(usize, usize) -> usize, out: &mut Vec<Action>) {
    let tab = st.tab;
    let Some(list) = st.tabs[tab].list.as_mut() else { return };
    if list.rows.is_empty() {
        return;
    }
    let old = list.selected;
    list.selected = f(old, list.rows.len()).min(list.rows.len() - 1);
    let moved = list.selected != old;
    maybe_next_page(st, tab, out);
    if moved {
        st.tabs[tab].preview_scroll = 0;
        if let Some(key) = st.selected_key_any() {
            need_ticket(st, &key, false, out);
        }
    }
}

/// Infinite scroll: get the next page when the selection is near the end of the loaded rows.
fn maybe_next_page(st: &mut State, tab: usize, out: &mut Vec<Action>) {
    let Some(list) = st.tabs[tab].list.as_mut() else { return };
    let near_end = list.rows.len().saturating_sub(list.selected + 1) < NEXT_PAGE_DISTANCE;
    if near_end && list.has_more() && !list.loading && !list.stale {
        list.loading = true;
        out.push(Action::FetchPage {
            tab,
            jql: list.jql.clone(),
            start_at: list.rows.len() as u64,
            generation: list.generation,
        });
    }
}

/// Start a new query on a list: a new generation, so that old responses are ignored.
/// The old rows stay on the screen (dim) until the first page arrives.
fn load_first_page(st: &mut State, tab: usize, keep_selection: bool, out: &mut Vec<Action>) {
    let Some(list) = st.tabs[tab].list.as_mut() else { return };
    // The Search tab has no query until the user runs one.
    if list.jql.trim().is_empty() {
        return;
    }
    list.generation += 1;
    list.loading = true;
    list.stale = !list.rows.is_empty();
    list.keep_key = if keep_selection { list.selected_key().map(str::to_string) } else { None };
    out.push(Action::FetchPage { tab, jql: list.jql.clone(), start_at: 0, generation: list.generation });
}

// ------------------------------------------------------------------ responses

fn page(
    st: &mut State,
    tab: usize,
    generation: u64,
    start_at: u64,
    result: Result<Page<ListRow>, Error>,
    out: &mut Vec<Action>,
) {
    let Some(list) = st.tabs.get_mut(tab).and_then(|t| t.list.as_mut()) else { return };
    if generation != list.generation {
        return; // A late response of an old query.
    }
    list.loading = false;
    if tab == SEARCH && start_at == 0 {
        st.search.error = None;
    }
    let Some(list) = st.tabs.get_mut(tab).and_then(|t| t.list.as_mut()) else { return };
    match result {
        Ok(p) => {
            list.offline = false;
            list.total = Some(p.total);
            if start_at == 0 {
                list.gone.clear();
                list.rows = p.items;
                list.stale = false;
                list.selected = 0;
                list.regroup();
                list.selected = match list.keep_key.take() {
                    Some(k) => list.rows.iter().position(|r| r.ticket.key == k).unwrap_or(0),
                    None => 0,
                };
                list.table = Default::default();
                st.tabs[tab].preview_scroll = 0;
                // The first row shows at once; there is no scroll to wait for.
                if tab == st.tab
                    && st.tabs[tab].zoom.is_empty()
                    && let Some(key) = st.selected_key_any()
                {
                    need_ticket(st, &key, true, out);
                }
            } else if start_at as usize == list.rows.len() {
                list.rows.extend(p.items);
                // A new page can add rows to a group above the selection: the selected
                // Ticket stays selected.
                list.regroup();
                // If the server sent fewer rows than the total said, stop the scroll.
                if list.rows.len() as u64 > p.total {
                    list.total = Some(list.rows.len() as u64);
                }
            }
            maybe_next_page(st, tab, out);
        }
        Err(e) => {
            list.stale = false;
            list.keep_key = None;
            if matches!(e, Error::Network { .. } | Error::Timeout(_)) {
                list.offline = true;
            }
            // A JQL error shows below the input, and the input gets the focus again.
            if tab == SEARCH
                && start_at == 0
                && let Error::Validation { fields, messages } = &e
            {
                let mut all = messages.clone();
                all.extend(fields.values().cloned());
                list.rows.clear();
                list.total = None;
                st.search.error = Some(if all.is_empty() { "Jira did not accept the JQL.".into() } else { all.join(" ") });
                st.search.focus = st.tab == SEARCH;
                log_error(st, "Search with JQL", &e);
                return;
            }
            let page_no = start_at / PAGE_SIZE + 1;
            let ctx = format!("Get page {page_no} of {}", TAB_TITLES[tab]);
            report(st, &ctx, &e);
        }
    }
}

fn ticket(st: &mut State, key: String, result: Result<Ticket, Error>, out: &mut Vec<Action>) {
    if let Ok(t) = &result {
        update_card(st, t);
    }
    let zoomed = st.tabs.iter().any(|t| t.zoom.last().is_some_and(|z| z.key == key));
    st.pending.remove(&key);
    // The lines of this Ticket must be made again.
    st.preview_render.key = None;
    st.zoom_render.key = None;
    match result {
        Ok(t) => {
            st.failed.remove(&key);
            update_rows(st, &t);
            st.cache.put(t);
            if zoomed {
                need_epic_children(st, &key, out);
            }
        }
        Err(Error::Cancelled) => {}
        Err(e) => {
            st.failed.insert(key.clone(), e.to_string());
            report(st, &format!("Get Ticket {key}"), &e);
        }
    }
}

/// The zoom of an Epic shows its child Tickets: get them when jui does not have them.
fn need_epic_children(st: &mut State, key: &str, out: &mut Vec<Action>) {
    let is_epic = st.cache.get(key).is_some_and(|t| t.kind.eq_ignore_ascii_case("epic"));
    if is_epic && !st.epic_children.contains_key(key) {
        st.epic_children.insert(key.to_string(), EpicChildren::Loading);
        out.push(Action::FetchEpicChildren { key: key.to_string() });
    }
}

/// Show an error in the status line and keep it for the `!` popup.
fn report(st: &mut State, context: &str, e: &Error) {
    match e {
        Error::Cancelled => return,
        Error::NoToken | Error::Unauthorized => fatal(st, e),
        _ => {}
    }
    log_error(st, context, e);
    st.set_status(MsgKind::Error, format!("{e} (! for details)"));
}

/// Keep an error for the `!` popup.
fn log_error(st: &mut State, context: &str, e: &Error) {
    let mut message = e.to_string();
    if let Error::Validation { fields, .. } = e {
        for (f, m) in fields {
            message.push_str(&format!(" {f}: {m}"));
        }
    }
    st.errors.push_front(LoggedError { context: context.to_string(), kind: e.kind(), message, hint: e.hint() });
    st.errors.truncate(ERROR_LOG_SIZE);
}

/// Put the new data of a Ticket in the rows of all lists that show it.
fn update_rows(st: &mut State, t: &Ticket) {
    for tab in &mut st.tabs {
        let Some(list) = tab.list.as_mut() else { continue };
        let mut moved = false;
        for r in list.rows.iter_mut().filter(|r| r.ticket.key == t.key) {
            let new: TicketSummary = t.into();
            // The full Ticket gets its Sprint from the Agile endpoint. When that request
            // failed, keep the Sprint of the row.
            let sprint = if t.sprint.is_some() { new.sprint.clone() } else { r.ticket.sprint.clone() };
            moved |= sprint != r.ticket.sprint;
            r.ticket = TicketSummary { sprint, ..new };
            r.description = t.description.as_ref().map(|d| d.wiki.clone());
        }
        if moved {
            list.regroup();
        }
    }
}

// ------------------------------------------------------------------ Transitions

fn transitions(st: &mut State, key: String, result: Result<Vec<Transition>, Error>) {
    let Some(Popup::Transition(p)) = &mut st.popup else { return };
    if p.key != key {
        return;
    }
    match result {
        Ok(ts) if ts.is_empty() => {
            st.popup = None;
            st.set_status(MsgKind::Warning, format!("{key} has no Transitions that you can do."));
        }
        Ok(ts) => p.list = Some(ts),
        Err(e) => {
            st.popup = None;
            report(st, &format!("Get the Transitions of {key}"), &e);
        }
    }
}

fn transition_key(st: &mut State, mut p: TransitionPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        // Wait for the answer. Esc hides the popup; the request continues.
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Transition(p));
        }
        return;
    }
    let shown = p.filtered();
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Down => p.selected = (p.selected + 1).min(shown.len().saturating_sub(1)),
        KeyCode::Up => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char('n') if k.modifiers.contains(KeyModifiers::CONTROL) => {
            p.selected = (p.selected + 1).min(shown.len().saturating_sub(1))
        }
        KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => p.selected = p.selected.saturating_sub(1),
        KeyCode::Backspace => {
            p.filter.pop();
            p.selected = 0;
        }
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
            p.filter.push(c);
            p.selected = 0;
        }
        KeyCode::Enter => {
            let (Some(ts), Some(&i)) = (&p.list, shown.get(p.selected)) else {
                st.popup = Some(Popup::Transition(p));
                return;
            };
            let t = ts[i].clone();
            let missing: Vec<_> = tr::missing_required(&t, &Map::new()).into_iter().cloned().collect();
            if missing.is_empty() {
                p.sending = true;
                p.error = None;
                out.push(Action::DoTransition { key: p.key.clone(), transition: t, fields: Map::new() });
            } else {
                st.popup = Some(Popup::Fields(FieldsPopup::new(&p.key, t, missing)));
                return;
            }
        }
        _ => {}
    }
    st.popup = Some(Popup::Transition(p));
}

fn fields_key(st: &mut State, mut p: FieldsPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Fields(p));
        }
        return;
    }
    let i = p.index;
    let has_choices = !p.fields[i].allowed.is_empty();
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Tab => {
            p.index = (i + 1) % p.fields.len();
            p.choice = 0;
        }
        KeyCode::BackTab => {
            p.index = (i + p.fields.len() - 1) % p.fields.len();
            p.choice = 0;
        }
        KeyCode::Down if has_choices => p.choice = (p.choice + 1).min(p.choices().len().saturating_sub(1)),
        KeyCode::Up if has_choices => p.choice = p.choice.saturating_sub(1),
        KeyCode::Backspace => {
            p.inputs[i].pop();
            p.values[i] = None;
            p.choice = 0;
        }
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
            p.inputs[i].push(c);
            p.values[i] = None;
            p.choice = 0;
        }
        KeyCode::Enter => {
            let f = &p.fields[i];
            let value = if has_choices {
                match p.choices().get(p.choice) {
                    Some(&c) => {
                        let choice = &f.allowed[c];
                        let v = match f.kind {
                            FieldKind::MultiChoice => serde_json::json!([{ "id": choice.id }]),
                            _ => serde_json::json!({ "id": choice.id }),
                        };
                        Ok((choice.name.clone(), v))
                    }
                    None => Err("No match.".to_string()),
                }
            } else if p.inputs[i].trim().is_empty() {
                Err("A value is required.".to_string())
            } else {
                tr::field_value(f, &p.inputs[i]).map(|v| (p.inputs[i].trim().to_string(), v)).map_err(|e| e.to_string())
            };
            match value {
                Ok((text, v)) => {
                    p.inputs[i] = text.clone();
                    p.values[i] = Some((text, v));
                    p.errors[i] = None;
                    match p.values.iter().position(Option::is_none) {
                        Some(next) => {
                            p.index = next;
                            p.choice = 0;
                        }
                        None => {
                            let mut fields = Map::new();
                            for (f, v) in p.fields.iter().zip(&p.values) {
                                if let Some((_, v)) = v {
                                    fields.insert(f.id.clone(), v.clone());
                                }
                            }
                            p.sending = true;
                            p.general = None;
                            out.push(Action::DoTransition { key: p.key.clone(), transition: p.transition.clone(), fields });
                        }
                    }
                }
                Err(e) => p.errors[i] = Some(e),
            }
        }
        _ => {}
    }
    st.popup = Some(Popup::Fields(p));
}

fn moved(st: &mut State, key: String, to: String, result: Result<(), Error>, out: &mut Vec<Action>) {
    // The popup of this request, if it is still open.
    let open = match &st.popup {
        Some(Popup::Transition(p)) => p.key == key,
        Some(Popup::Fields(p)) => p.key == key,
        _ => false,
    };
    match result {
        Ok(()) => {
            if open {
                st.popup = None;
            }
            st.set_status(MsgKind::Info, format!("{key} moved to {to}."));
            st.cache.remove(&key);
            st.failed.remove(&key);
            st.pending.remove(&key);
            fetch_ticket(st, &key, out);
            for (tab, t) in st.tabs.iter().enumerate() {
                if let Some(list) = &t.list
                    && list.rows.iter().any(|r| r.ticket.key == key)
                {
                    let jql = crate::core::model::jql_with_key(&list.jql, &key);
                    out.push(Action::CheckMatch { tab, key: key.clone(), jql });
                }
            }
        }
        Err(Error::Validation { fields, messages }) if open => {
            // The popup stays open with the values of the user, and shows the errors.
            match st.popup.as_mut() {
                Some(Popup::Fields(p)) => {
                    p.sending = false;
                    let mut rest = messages.clone();
                    for (name, msg) in &fields {
                        match p.fields.iter().position(|f| &f.name == name || &f.id == name) {
                            Some(i) => p.errors[i] = Some(msg.clone()),
                            None => rest.push(format!("{name}: {msg}")),
                        }
                    }
                    if let Some(first) = p.errors.iter().position(Option::is_some) {
                        p.index = first;
                    }
                    p.general = (!rest.is_empty()).then(|| rest.join(" "));
                }
                Some(Popup::Transition(p)) => {
                    p.sending = false;
                    let mut all = messages.clone();
                    all.extend(fields.iter().map(|(n, m)| format!("{n}: {m}")));
                    p.error = Some(if all.is_empty() { "Jira did not accept the Transition.".into() } else { all.join(" ") });
                }
                _ => {}
            }
            let e = Error::Validation { fields, messages };
            log_error(st, &format!("Move {key} to {to}"), &e);
        }
        Err(e) => {
            if open {
                st.popup = None;
            }
            report(st, &format!("Move {key} to {to}"), &e);
        }
    }
}

// ------------------------------------------------------------------ create and edit

/// `e`, `W`, `n`, `N`: start a create or an edit. Only one runs at a time.
fn start_template(st: &mut State, cmd: Cmd, out: &mut Vec<Action>) {
    if st.job_running() {
        st.set_status(MsgKind::Warning, "A create or edit is running. Wait until it is done.");
        return;
    }
    let selected = st.selected_key_any();
    // `n` in the zoom of an Epic puts the new Ticket in this Epic.
    let zoom_epic = st.zoom_epic();
    let (kind, action) = match (cmd, selected) {
        (Cmd::Create, _) if let Some(epic) = zoom_epic => {
            let kind = TemplateKind::InEpic { epic };
            let a = Action::PrepareCreate { kind: kind.clone(), project: st.default_project.clone(), issue_type: st.default_type.clone() };
            (kind, a)
        }
        (Cmd::Create, _) => {
            let kind = TemplateKind::Create;
            let a = Action::PrepareCreate { kind: kind.clone(), project: st.default_project.clone(), issue_type: st.default_type.clone() };
            (kind, a)
        }
        (Cmd::CreateSubtask, Some(parent)) => {
            let project = parent.split('-').next().unwrap_or(&st.default_project).to_string();
            let kind = TemplateKind::Subtask { parent };
            (kind.clone(), Action::PrepareCreate { kind, project, issue_type: "subtask".into() })
        }
        (Cmd::Edit | Cmd::EditWiki, Some(key)) => {
            let kind = TemplateKind::Edit { key: key.clone(), wiki: cmd == Cmd::EditWiki };
            (kind.clone(), Action::PrepareEdit { kind, key })
        }
        _ => {
            st.set_status(MsgKind::Warning, "No Ticket is selected.");
            return;
        }
    };
    let what = match &kind {
        TemplateKind::Create => "Getting the create screen…".to_string(),
        TemplateKind::Subtask { parent } => format!("Getting the Sub-task screen for {parent}…"),
        TemplateKind::InEpic { epic } => format!("Getting the create screen for a Ticket in {epic}…"),
        TemplateKind::Edit { key, .. } => format!("Getting {key} for the edit…"),
    };
    st.set_status(MsgKind::Info, what);
    out.push(action);
}

fn kind_context(kind: &TemplateKind) -> String {
    match kind {
        TemplateKind::Create => "Create a Ticket".into(),
        TemplateKind::Subtask { parent } => format!("Create a Sub-task of {parent}"),
        TemplateKind::InEpic { epic } => format!("Create a Ticket in {epic}"),
        TemplateKind::Edit { key, .. } => format!("Edit {key}"),
    }
}

fn template_ready(
    st: &mut State,
    kind: TemplateKind,
    session: Result<Session, Error>,
    before: Option<EditData>,
    out: &mut Vec<Action>,
) {
    match session {
        Ok(mut session) => {
            // The parent or the Epic goes into the Template, as `jui new --parent` and
            // `jui new --epic` do.
            let preset = match &kind {
                TemplateKind::Subtask { parent } => Some(("parent", parent.clone())),
                TemplateKind::InEpic { epic } => Some(("epic", epic.clone())),
                _ => None,
            };
            let mut p = Preset::default();
            if let Some((field, value)) = preset {
                p.values.push((field.into(), tpl::FlagValue::Text(value)));
                if let tpl::Mode::Create(m) = &session.mode {
                    session = Session::create(m.clone(), session_me(&session), &p);
                }
            }
            st.status = None;
            // A create opens the form. An edit opens the editor.
            if let tpl::Mode::Create(meta) = &session.mode {
                st.popup = Some(Popup::Create(create_popup(&kind, meta, &p)));
                st.job = Some(TemplateJob { kind, session, before, from_form: false });
                return;
            }
            out.push(Action::OpenEditor { text: session.text().to_string(), label: session.label().to_string() });
            st.job = Some(TemplateJob { kind, session, before, from_form: false });
        }
        Err(e) => report(st, &kind_context(&kind), &e),
    }
}

/// Make the create form for a create screen. The preset (the parent or the Epic) fills fields.
fn create_popup(kind: &TemplateKind, meta: &CreateMeta, preset: &Preset) -> CreatePopup {
    let all_keys = tpl::keys(&meta.fields);
    let shown = tpl::form_fields(meta, preset);
    let fields: Vec<_> = shown.iter().map(|&i| meta.fields[i].clone()).collect();
    let keys: Vec<String> = shown.iter().map(|&i| all_keys[i].clone()).collect();
    let inputs = shown
        .iter()
        .map(|&i| {
            preset
                .values
                .iter()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case(&all_keys[i]) || k.eq_ignore_ascii_case(&meta.fields[i].id))
                .map(|(_, v)| match v {
                    tpl::FlagValue::Text(t) => t.clone(),
                    tpl::FlagValue::List(l) => l.join(", "),
                })
                .unwrap_or_default()
        })
        .collect();
    let title = match kind {
        TemplateKind::Subtask { parent } => format!("Create a {} of {parent}", meta.issue_type.name),
        TemplateKind::InEpic { epic } => format!("Create a {} in {} (Epic {epic})", meta.issue_type.name, meta.project),
        _ => format!("Create a {} in {}", meta.issue_type.name, meta.project),
    };
    CreatePopup::new(title, fields, keys, inputs)
}

/// Show the errors of the create form below their fields. Errors of fields that the form does
/// not show go to the general error.
fn show_form_errors(p: &mut CreatePopup, errs: &[tpl::TemplateError]) {
    p.errors = vec![None; p.fields.len()];
    let mut rest = Vec::new();
    for e in errs {
        match e.key.as_deref().and_then(|k| p.keys.iter().position(|x| x == k)) {
            Some(i) => p.errors[i] = Some(e.message.clone()),
            None => rest.push(e.text()),
        }
    }
    p.general = (!rest.is_empty()).then(|| rest.join(" "));
    if let Some(first) = p.errors.iter().position(Option::is_some) {
        p.move_focus(first);
    }
}

/// Take the selected permitted value into the focused field. A list field gets the value at
/// the end, after the values that it has. Returns false when no value matches.
fn take_choice(p: &mut CreatePopup) -> bool {
    let Some(&c) = p.choices().get(p.choice) else { return false };
    let name = p.fields[p.focus].allowed[c].name.clone();
    if p.is_list() {
        let input = &p.inputs[p.focus];
        let mut parts: Vec<String> = input.split(',').map(|s| s.trim().to_string()).collect();
        parts.pop();
        parts.retain(|s| !s.is_empty());
        parts.push(name);
        p.inputs[p.focus] = format!("{}, ", parts.join(", "));
    } else {
        p.inputs[p.focus] = name;
    }
    p.errors[p.focus] = None;
    p.choice = 0;
    p.browsing = false;
    true
}

fn create_key(st: &mut State, mut p: CreatePopup, k: KeyEvent, out: &mut Vec<Action>) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    if p.sending {
        st.popup = Some(Popup::Create(p));
        return;
    }
    let has_choices = p.fields.get(p.focus).is_some_and(|f| !f.allowed.is_empty());
    match k.code {
        KeyCode::Esc if p.browsing => p.browsing = false,
        KeyCode::Esc => {
            let preset = p.preset();
            if p.has_text() {
                if let Some(job) = st.job.as_mut() {
                    job.session.use_form(&preset);
                }
                draft(st, "Cancelled. No Ticket created.".into(), out);
            } else {
                st.job = None;
                st.set_status(MsgKind::Info, "Cancelled. No Ticket created.");
            }
            return;
        }
        KeyCode::Char('s') if ctrl => {
            let preset = p.preset();
            let Some(job) = st.job.as_mut() else { return };
            match job.session.submit_form(&preset) {
                Ok(body) => {
                    p.sending = true;
                    p.general = None;
                    p.errors = vec![None; p.fields.len()];
                    st.set_status(MsgKind::Info, "Creating the Ticket…");
                    out.push(Action::SendCreate { body });
                }
                Err(errs) => show_form_errors(&mut p, &errs),
            }
        }
        KeyCode::Char('e') if ctrl => {
            let preset = p.preset();
            let Some(job) = st.job.as_mut() else { return };
            job.session.use_form(&preset);
            job.from_form = p.has_text();
            out.push(Action::OpenEditor { text: job.session.text().to_string(), label: job.session.label().to_string() });
            return;
        }
        KeyCode::Char('u') if ctrl => {
            if p.on_description() {
                p.description.clear();
            } else {
                p.inputs[p.focus].clear();
                p.errors[p.focus] = None;
            }
            p.choice = 0;
            p.browsing = false;
        }
        KeyCode::Tab => p.move_focus(p.focus + 1),
        KeyCode::BackTab => p.move_focus(p.focus + p.stops() - 1),
        KeyCode::Down if has_choices => {
            if p.browsing {
                p.choice = (p.choice + 1).min(p.choices().len().saturating_sub(1));
            }
            p.browsing = true;
        }
        KeyCode::Up if has_choices && p.browsing && p.choice > 0 => p.choice -= 1,
        KeyCode::Up if has_choices && p.browsing => p.browsing = false,
        KeyCode::Down if !p.on_description() => p.move_focus(p.focus + 1),
        KeyCode::Up => p.move_focus(p.focus + p.stops() - 1),
        KeyCode::Enter if p.on_description() => p.description.push('\n'),
        KeyCode::Enter => {
            let filtering = !p.filter_text().is_empty();
            if has_choices && (p.browsing || filtering) {
                if !take_choice(&mut p) {
                    p.errors[p.focus] = Some("No permitted value matches.".into());
                } else if !p.is_list() {
                    p.move_focus(p.focus + 1);
                }
            } else {
                p.move_focus(p.focus + 1);
            }
        }
        KeyCode::Backspace => {
            if p.on_description() {
                p.description.pop();
            } else {
                p.inputs[p.focus].pop();
                p.errors[p.focus] = None;
                p.choice = 0;
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if p.on_description() {
                p.description.push(c);
            } else {
                p.inputs[p.focus].push(c);
                p.errors[p.focus] = None;
                p.choice = 0;
                p.browsing = false;
            }
        }
        _ => {}
    }
    st.popup = Some(Popup::Create(p));
}

/// The current user of a session. The session keeps it private; the Template has it as `me`.
fn session_me(s: &Session) -> &str {
    s.me()
}

fn edited(st: &mut State, result: Result<String, Error>, out: &mut Vec<Action>) {
    if st.worklog_job.is_some() {
        worklog_edited(st, result, out);
        return;
    }
    if st.comment_job.is_some() {
        comment_edited(st, result, out);
        return;
    }
    let Some(job) = st.job.as_mut() else { return };
    let text = match result {
        Ok(t) => t,
        Err(e) => {
            let ctx = kind_context(&job.kind);
            draft(st, format!("The editor failed: {e}"), out);
            log_error(st, &ctx, &e);
            return;
        }
    };
    let step = job.session.submit(&text);
    next_step(st, step, out);
}

fn next_step(st: &mut State, step: Step, out: &mut Vec<Action>) {
    let Some(job) = st.job.as_mut() else { return };
    match step {
        // The editor came from the create form, and the form had text: keep it in a Draft.
        Step::Cancel(m) if job.from_form => draft(st, m, out),
        Step::Cancel(m) => {
            st.job = None;
            st.set_status(MsgKind::Info, m);
        }
        Step::Reopen => {
            out.push(Action::OpenEditor { text: job.session.text().to_string(), label: job.session.label().to_string() });
        }
        Step::Stop(m) => draft(st, m, out),
        Step::Send(Request::Create(body)) => {
            st.set_status(MsgKind::Info, "Creating the Ticket…");
            out.push(Action::SendCreate { body });
        }
        Step::Send(r @ Request::Edit(_)) => {
            let TemplateKind::Edit { key, .. } = &job.kind else { return };
            let key = key.clone();
            st.pending_plan = Some(r);
            st.set_status(MsgKind::Info, format!("Checking {key} before the change…"));
            out.push(Action::Recheck { key });
        }
    }
}

/// Stop the job and write a Draft.
fn draft(st: &mut State, reason: String, out: &mut Vec<Action>) {
    st.pending_plan = None;
    if let Some(job) = st.worklog_job.take() {
        let s = &job.session;
        out.push(Action::WriteDraft { label: s.label(), text: s.draft_text().to_string(), reason, command: s.command() });
        return;
    }
    if let Some(job) = st.comment_job.take() {
        let s = &job.session;
        out.push(Action::WriteDraft { label: s.label(), text: s.draft_text().to_string(), reason, command: s.command() });
        return;
    }
    if let Some(job) = st.job.take() {
        let command = match &job.kind {
            TemplateKind::Edit { key, wiki: true } => format!("edit {key} --wiki"),
            TemplateKind::Edit { key, .. } => format!("edit {key}"),
            _ => "new".to_string(),
        };
        out.push(Action::WriteDraft { label: job.session.label().to_string(), text: job.session.draft_text().to_string(), reason, command });
    }
}

fn rechecked(st: &mut State, key: String, result: Result<EditData, Error>, out: &mut Vec<Action>) {
    let Some(job) = st.job.as_ref() else { return };
    let now = match result {
        Ok(d) => d,
        Err(e) => {
            draft(st, format!("jui could not read {key} again."), out);
            report(st, &format!("Edit {key}"), &e);
            return;
        }
    };
    let conflict = job.before.as_ref().filter(|b| b.updated != now.updated).map(|b| {
        let fields = tpl::changed_in_jira(b, &now);
        let what = if fields.is_empty() { String::new() } else { format!(" Changed: {}.", fields.join(", ")) };
        format!("{key} changed in Jira while you edited it.{what}")
    });
    let Some(Request::Edit(plan)) = st.pending_plan.clone() else { return };
    let fields = plan.changed.clone();
    match &plan.description {
        Some(d) if d.needs_diff() => {
            st.status = None;
            st.popup = Some(Popup::Diff(DiffPopup {
                key,
                subject: "The description".into(),
                lines: tpl::diff(&d.old, &d.new),
                warnings: d.warnings.clone(),
                fields,
                conflict,
                scroll: 0,
            }));
        }
        _ => match conflict {
            Some(message) => {
                st.status = None;
                st.popup = Some(Popup::Conflict { key, message });
            }
            None => send_plan(st, out),
        },
    }
}

fn send_plan(st: &mut State, out: &mut Vec<Action>) {
    let (Some(Request::Edit(plan)), Some(job)) = (st.pending_plan.take(), st.job.as_ref()) else { return };
    let TemplateKind::Edit { key, .. } = &job.kind else { return };
    let key = key.clone();
    let mut changed = plan.changed.clone();
    if plan.description.is_some() {
        changed.push("description".into());
    }
    st.set_status(MsgKind::Info, format!("Saving {key}…"));
    out.push(Action::SendEdit { key, body: plan.body(), changed });
}

fn diff_key(st: &mut State, mut p: DiffPopup, k: KeyEvent, out: &mut Vec<Action>) {
    let comment = st.comment_job.is_some();
    let worklog = st.worklog_job.is_some();
    match k.code {
        KeyCode::Char('y') if worklog => send_worklog_plan(st, out),
        KeyCode::Char('n') if worklog => {
            if let Some(job) = st.worklog_job.as_mut() {
                job.plan = None;
                job.session.reopen_note("You did not send the change. Edit the Template, or save it with no changes to cancel.");
                out.push(Action::OpenEditor { text: job.session.text().to_string(), label: job.session.label() });
            }
        }
        KeyCode::Char('y') if comment => send_comment_plan(st, out),
        KeyCode::Char('n') if comment => {
            if let Some(job) = st.comment_job.as_mut() {
                job.plan = None;
                job.session.reopen_note("You did not send the change. Edit the text, or empty the file to cancel.");
                out.push(Action::OpenEditor { text: job.session.text().to_string(), label: job.session.label() });
            }
        }
        KeyCode::Char('y') => send_plan(st, out),
        KeyCode::Char('n') => {
            // Open the editor again with the text of the user.
            st.pending_plan = None;
            if let Some(job) = st.job.as_mut() {
                let note = "You did not send the change. Edit the text, or empty the file to cancel.";
                job.session.reopen_note(note);
                out.push(Action::OpenEditor { text: job.session.text().to_string(), label: job.session.label().to_string() });
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => draft(st, format!("You did not send the change to {}.", p.key), out),
        KeyCode::Char('j') | KeyCode::Down => {
            p.scroll = p.scroll.saturating_add(1);
            st.popup = Some(Popup::Diff(p));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            p.scroll = p.scroll.saturating_sub(1);
            st.popup = Some(Popup::Diff(p));
        }
        _ => st.popup = Some(Popup::Diff(p)),
    }
}

fn conflict_key(st: &mut State, p: Popup, k: KeyEvent, out: &mut Vec<Action>) {
    let Popup::Conflict { key, .. } = &p else { return };
    match k.code {
        KeyCode::Char('y') => send_plan(st, out),
        KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => draft(st, format!("You did not send the change to {key}."), out),
        _ => st.popup = Some(p),
    }
}

/// A write request failed. A validation error opens the Template again with error lines.
fn write_failed(st: &mut State, e: Error, out: &mut Vec<Action>) {
    let Some(job) = st.job.as_mut() else { return };
    let ctx = kind_context(&job.kind);
    // The create form is open: it stays open with the values, and shows the errors.
    if let Some(Popup::Create(p)) = st.popup.as_mut() {
        p.sending = false;
        match e {
            Error::Validation { fields, messages } => {
                let mut errs: Vec<tpl::TemplateError> =
                    messages.iter().map(|m| tpl::TemplateError { key: None, message: m.clone() }).collect();
                for (id, m) in &fields {
                    let key = p
                        .fields
                        .iter()
                        .position(|f| &f.id == id || &f.name == id)
                        .map(|i| p.keys[i].clone())
                        .unwrap_or_else(|| id.clone());
                    errs.push(tpl::TemplateError { key: Some(key), message: m.clone() });
                }
                if errs.is_empty() {
                    errs.push(tpl::TemplateError { key: None, message: "Jira did not accept the request.".into() });
                }
                show_form_errors(p, &errs);
                st.status = None;
                let e = Error::Validation { fields, messages };
                log_error(st, &ctx, &e);
            }
            e => {
                p.general = Some(e.to_string());
                st.status = None;
                log_error(st, &ctx, &e);
            }
        }
        return;
    }
    match job.session.rejected(e) {
        Ok(step) => {
            log_error(st, &ctx, &Error::input("Jira did not accept the Template. See the # ERROR: lines."));
            next_step(st, step, out);
        }
        Err(e) => {
            draft(st, format!("{ctx} failed."), out);
            report(st, &ctx, &e);
        }
    }
}

fn created(st: &mut State, result: Result<String, Error>, out: &mut Vec<Action>) {
    match result {
        Ok(key) => {
            if matches!(st.popup, Some(Popup::Create(_))) {
                st.popup = None;
            }
            let kind = st.job.take().map(|j| j.kind);
            let parent = match &kind {
                Some(TemplateKind::Subtask { parent }) => Some(parent.clone()),
                _ => None,
            };
            match &kind {
                Some(TemplateKind::InEpic { epic }) => {
                    st.set_status(MsgKind::Info, format!("Created {key} in {epic}."));
                    // The zoom of the Epic shows the new Ticket.
                    st.epic_children.remove(epic);
                    need_epic_children(st, &epic.clone(), out);
                }
                _ => st.set_status(MsgKind::Info, format!("Created {key}.")),
            }
            // The new Ticket can be in the lists. The parent has a new Sub-task.
            let tabs: Vec<usize> = (0..st.tabs.len()).filter(|&i| st.tabs[i].list.as_ref().is_some_and(|l| l.total.is_some())).collect();
            for t in tabs {
                if t == EPICS {
                    // The new Ticket can be in an Epic: get the counts again.
                    load_my_epics(st, out);
                } else {
                    load_first_page(st, t, true, out);
                }
            }
            if let Some(p) = parent {
                st.cache.remove(&p);
                st.pending.remove(&p);
                fetch_ticket(st, &p, out);
            }
        }
        Err(e) => write_failed(st, e, out),
    }
}

fn saved(st: &mut State, key: String, changed: Vec<String>, result: Result<(), Error>, out: &mut Vec<Action>) {
    match result {
        Ok(()) => {
            st.job = None;
            st.set_status(MsgKind::Info, format!("Saved {key}. Changed: {}.", changed.join(", ")));
            st.cache.remove(&key);
            st.failed.remove(&key);
            st.pending.remove(&key);
            fetch_ticket(st, &key, out);
            for (tab, t) in st.tabs.iter().enumerate() {
                if let Some(list) = &t.list
                    && list.rows.iter().any(|r| r.ticket.key == key)
                {
                    let jql = crate::core::model::jql_with_key(&list.jql, &key);
                    out.push(Action::CheckMatch { tab, key: key.clone(), jql });
                }
            }
        }
        Err(e) => write_failed(st, e, out),
    }
}

// ------------------------------------------------------------------ comments

/// Start a comment add or edit in the editor. Only one job runs at a time.
fn start_comment(st: &mut State, key: String, start: CommentStart, out: &mut Vec<Action>) {
    if st.job_running() {
        st.set_status(MsgKind::Warning, "A create or edit is running. Wait until it is done.");
        return;
    }
    let what = match &start {
        CommentStart::Add { .. } => format!("Getting the comments of {key}…"),
        CommentStart::Edit { id, .. } => format!("Getting comment {id} for the edit…"),
    };
    st.set_status(MsgKind::Info, what);
    out.push(Action::PrepareComment { key, start });
}

/// Keys in the `c` popup: type the text, Enter sends, `Ctrl-E` opens the editor, Esc closes.
fn comment_key(st: &mut State, mut p: CommentPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        // Wait for the answer. Esc hides the popup; the request continues.
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Comment(p));
        }
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Char('e') if ctrl => {
            let text = p.input.clone();
            start_comment(st, p.key, CommentStart::Add { text }, out);
            return;
        }
        KeyCode::Backspace => {
            p.input.pop();
        }
        KeyCode::Char(c) if !ctrl => p.input.push(c),
        KeyCode::Enter => match cm::to_plan(&p.input, false) {
            Some(plan) => {
                p.sending = true;
                p.error = None;
                out.push(Action::SendComment { key: p.key.clone(), id: None, wiki: plan.wiki });
            }
            None => {
                st.set_status(MsgKind::Info, "Cancelled. No comment added.");
                return;
            }
        },
        _ => {}
    }
    st.popup = Some(Popup::Comment(p));
}

fn comment_ready(st: &mut State, key: String, start: CommentStart, result: Result<CommentContext, Error>, out: &mut Vec<Action>) {
    let ctx = match result {
        Ok(c) => c,
        Err(e) => {
            // The text from the popup must not go away.
            if let CommentStart::Add { text } = &start
                && !text.trim().is_empty()
            {
                out.push(Action::WriteDraft {
                    label: format!("{key}-comment"),
                    text: text.clone(),
                    reason: format!("jui could not get the comments of {key}."),
                    command: format!("comment add {key}"),
                });
            }
            report(st, &format!("Comment on {key}"), &e);
            return;
        }
    };
    let session = match &start {
        CommentStart::Add { text } => CommentSession::add(&ctx, text, false),
        CommentStart::Edit { id, wiki } => match CommentSession::edit(&ctx, id, *wiki) {
            Ok(s) => s,
            Err(e) => {
                report(st, &format!("Edit comment {id} on {key}"), &e);
                return;
            }
        },
    };
    st.status = None;
    out.push(Action::OpenEditor { text: session.text().to_string(), label: session.label() });
    st.comment_job = Some(CommentJob { session, plan: None });
}

fn comment_edited(st: &mut State, result: Result<String, Error>, out: &mut Vec<Action>) {
    let Some(job) = st.comment_job.as_mut() else { return };
    let text = match result {
        Ok(t) => t,
        Err(e) => {
            draft(st, format!("The editor failed: {e}"), out);
            log_error(st, "Comment", &e);
            return;
        }
    };
    match job.session.submit(&text) {
        CommentStep::Cancel(m) => {
            st.comment_job = None;
            st.set_status(MsgKind::Info, m);
        }
        CommentStep::Reopen => out.push(Action::OpenEditor { text: job.session.text().to_string(), label: job.session.label() }),
        CommentStep::Stop(m) => draft(st, m, out),
        CommentStep::Send(plan) => {
            let key = job.session.key.clone();
            match plan.id.clone() {
                // An edit: read the comment again, then show the diff.
                Some(id) => {
                    job.plan = Some(plan);
                    st.set_status(MsgKind::Info, format!("Checking comment {id} before the change…"));
                    out.push(Action::RecheckComment { key, id });
                }
                None => {
                    job.plan = Some(plan);
                    send_comment_plan(st, out);
                }
            }
        }
    }
}

fn comment_rechecked(st: &mut State, key: String, id: String, result: Result<Comment, Error>, out: &mut Vec<Action>) {
    let Some(plan) = st.comment_job.as_ref().and_then(|j| j.plan.clone()) else { return };
    let now = match result {
        Ok(c) => c,
        Err(e) => {
            draft(st, format!("jui could not read comment {id} again."), out);
            report(st, &format!("Edit comment {id} on {key}"), &e);
            return;
        }
    };
    st.status = None;
    st.popup = Some(Popup::Diff(DiffPopup {
        conflict: cm::changed_in_jira(&key, &plan, &now),
        key,
        subject: format!("Comment {id}"),
        lines: tpl::diff(plan.old.as_deref().unwrap_or(""), &plan.wiki),
        warnings: plan.warnings.clone(),
        fields: vec![],
        scroll: 0,
    }));
}

fn send_comment_plan(st: &mut State, out: &mut Vec<Action>) {
    // The plan stays in the job, for the warnings in the result message.
    let Some(job) = st.comment_job.as_ref() else { return };
    let Some(plan) = job.plan.clone() else { return };
    let key = job.session.key.clone();
    let what = match &plan.id {
        Some(id) => format!("Saving comment {id}…"),
        None => format!("Adding the comment to {key}…"),
    };
    st.set_status(MsgKind::Info, what);
    out.push(Action::SendComment { key, id: plan.id, wiki: plan.wiki });
}

/// The Ticket has a new or changed comment: get it again.
fn refresh_ticket(st: &mut State, key: &str, out: &mut Vec<Action>) {
    st.cache.remove(key);
    st.failed.remove(key);
    st.pending.remove(key);
    fetch_ticket(st, key, out);
}

fn comment_sent(st: &mut State, key: String, id: Option<String>, result: Result<Comment, Error>, out: &mut Vec<Action>) {
    let popup_open = matches!(&st.popup, Some(Popup::Comment(p)) if p.key == key && p.sending);
    let from_job = !popup_open && st.comment_job.is_some();
    match result {
        Ok(c) => {
            let warnings = st.comment_job.take().and_then(|j| j.plan).map(|p| p.warnings).unwrap_or_default();
            if popup_open {
                st.popup = None;
            }
            let msg = match &id {
                Some(id) => format!("Saved comment {id} on {key}."),
                None => format!("Added comment {} to {key}.", c.id),
            };
            if warnings.is_empty() {
                st.set_status(MsgKind::Info, msg);
            } else {
                st.set_status(MsgKind::Warning, format!("{msg} warning: {}", warnings.join("; ")));
            }
            refresh_ticket(st, &key, out);
        }
        Err(e) if popup_open => {
            let Some(Popup::Comment(p)) = st.popup.as_mut() else { return };
            p.sending = false;
            match &e {
                // The popup stays open with the text of the user, and shows the error.
                Error::Validation { fields, messages } => {
                    let mut all = messages.clone();
                    all.extend(fields.iter().map(|(f, m)| format!("{f}: {m}")));
                    p.error = Some(if all.is_empty() { "Jira did not accept the comment.".into() } else { all.join(" ") });
                    log_error(st, &format!("Comment on {key}"), &e);
                }
                _ => {
                    let text = p.input.clone();
                    st.popup = None;
                    out.push(Action::WriteDraft {
                        label: format!("{key}-comment"),
                        text,
                        reason: format!("jui could not add the comment to {key}."),
                        command: format!("comment add {key}"),
                    });
                    report(st, &format!("Comment on {key}"), &e);
                }
            }
        }
        Err(e) if from_job => {
            let Some(job) = st.comment_job.as_mut() else { return };
            job.plan = None;
            match job.session.rejected(e) {
                Ok(CommentStep::Reopen) => {
                    out.push(Action::OpenEditor { text: job.session.text().to_string(), label: job.session.label() });
                    log_error(st, "Comment", &Error::input("Jira did not accept the comment. See the # ERROR: lines."));
                }
                Ok(_) => {}
                Err(e) => {
                    draft(st, format!("jui could not send the comment to {key}."), out);
                    report(st, &format!("Comment on {key}"), &e);
                }
            }
        }
        // The popup was hidden with Esc during the request.
        Err(e) => report(st, &format!("Comment on {key}"), &e),
    }
}

const DELETE_HINT: &str = "Select a comment, a Worklog, a Link, or an attachment to delete it.";

/// `d` in the zoom: ask before jui deletes the selected comment, Worklog, Link, or attachment.
fn start_delete(st: &mut State) {
    let Some((key, item)) = st.selected_item() else {
        st.set_status(MsgKind::Info, DELETE_HINT);
        return;
    };
    let t = st.cache.get(&key);
    let (what, id, preview) = match item {
        ZoomItem::Comment(id) => {
            let preview = t.and_then(|t| t.comments.iter().find(|c| c.id == id)).map(|c| format!("@{}: {}", cm::author(c), cm::first_line(c, 60)));
            (DeleteKind::Comment, id, preview)
        }
        ZoomItem::Worklog(id) => {
            let preview = t.and_then(|t| t.worklogs.iter().find(|w| w.id == id)).map(|w| wl::one_line(w, 50));
            (DeleteKind::Worklog, id, preview)
        }
        ZoomItem::Link { id, .. } => {
            let preview = t.and_then(|t| t.links.iter().find(|l| l.id == id)).map(|l| lk::describe(&key, l));
            (DeleteKind::Link, id, preview)
        }
        ZoomItem::Attachment(id) => {
            let preview = t.and_then(|t| t.attachments.iter().find(|a| a.id == id)).map(crate::core::attachment::one_line);
            (DeleteKind::Attachment, id, preview)
        }
        ZoomItem::Ticket(_) => {
            st.set_status(MsgKind::Info, DELETE_HINT);
            return;
        }
    };
    st.popup = Some(Popup::ConfirmDelete { what, key, id, preview: preview.unwrap_or_default(), sending: false });
}

fn delete_key(st: &mut State, p: Popup, k: KeyEvent, out: &mut Vec<Action>) {
    let Popup::ConfirmDelete { what, key, id, preview, sending } = p else { return };
    if sending {
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::ConfirmDelete { what, key, id, preview, sending });
        }
        return;
    }
    match k.code {
        KeyCode::Char('y') => {
            let (key2, id2) = (key.clone(), id.clone());
            out.push(match what {
                DeleteKind::Comment => Action::DeleteComment { key: key2, id: id2 },
                DeleteKind::Worklog => Action::DeleteWorklog { key: key2, id: id2 },
                DeleteKind::Link => Action::DeleteLink { key: key2, id: id2 },
                DeleteKind::Attachment => Action::DeleteAttachment { key: key2, id: id2 },
            });
            st.popup = Some(Popup::ConfirmDelete { what, key, id, preview, sending: true });
        }
        KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {}
        _ => st.popup = Some(Popup::ConfirmDelete { what, key, id, preview, sending }),
    }
}

fn deleted(st: &mut State, what: DeleteKind, key: String, id: String, result: Result<(), Error>, out: &mut Vec<Action>) {
    if matches!(&st.popup, Some(Popup::ConfirmDelete { id: i, what: w, .. }) if *i == id && *w == what) {
        st.popup = None;
    }
    let name = what.name();
    match result {
        Ok(()) => {
            let msg = match what {
                DeleteKind::Link => {
                    // The other Ticket also loses the Link.
                    let other = st.cache.get(&key).and_then(|t| t.links.iter().find(|l| l.id == id)).map(|l| l.ticket.key.clone());
                    if let Some(o) = other {
                        st.cache.remove(&o);
                    }
                    format!("Deleted Link {id} on {key}.")
                }
                _ => format!("Deleted {name} {id} on {key}."),
            };
            st.set_status(MsgKind::Info, msg);
            // Keep the selection on the item before the deleted comment.
            if let Some(z) = st.current_mut().zoom.last_mut()
                && z.key == key
            {
                z.item = z.item.saturating_sub(1);
            }
            refresh_ticket(st, &key, out);
        }
        Err(e) => report(st, &format!("Delete {name} {id} on {key}"), &e),
    }
}

// ------------------------------------------------------------------ assign and watch

/// Keys in the user picker: type to search, ↑↓ select, Enter assign, Esc close.
fn user_key(st: &mut State, mut p: UserPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        // Wait for the answer. Esc hides the popup; the request continues.
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::User(p));
        }
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let rows = p.rows(&st.recent_users);
    let mut changed = false;
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Down => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Up => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char('n') if ctrl => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Char('p') if ctrl => p.selected = p.selected.saturating_sub(1),
        KeyCode::Backspace => {
            p.input.pop();
            changed = true;
        }
        KeyCode::Char(c) if !ctrl => {
            p.input.push(c);
            changed = true;
        }
        KeyCode::Enter => match rows.get(p.selected) {
            Some(row) => {
                p.sending = true;
                p.error = None;
                out.push(Action::Assign { key: p.key.clone(), who: row.assignee() });
            }
            None if p.searching => p.error = Some("Wait for the search.".into()),
            None => p.error = Some("No match.".into()),
        },
        _ => {}
    }
    if changed {
        p.selected = 0;
        p.error = None;
        // A new key makes the old search out of date. The search starts after the delay.
        p.seq += 1;
        p.results = None;
        p.searching = p.searches();
        if p.searching {
            out.push(Action::ScheduleUserSearch { seq: p.seq });
        }
    }
    st.popup = Some(Popup::User(p));
}

fn user_search_due(st: &mut State, seq: u64, out: &mut Vec<Action>) {
    let Some(Popup::User(p)) = &st.popup else { return };
    if p.seq == seq && p.searches() && !p.sending {
        out.push(Action::SearchUsers { key: p.key.clone(), text: p.input.trim().to_string(), seq });
    }
}

fn users(st: &mut State, key: String, seq: u64, result: Result<Vec<User>, Error>) {
    let Some(Popup::User(p)) = &mut st.popup else { return };
    if p.key != key || p.seq != seq {
        return; // A late result of an old text.
    }
    p.searching = false;
    match result {
        Ok(u) => p.results = Some(u),
        Err(e) => {
            p.results = Some(Vec::new());
            p.error = Some(e.to_string());
            log_error(st, &format!("Search users for {key}"), &e);
        }
    }
}

fn assigned(st: &mut State, key: String, who: Assignee, result: Result<(), Error>, out: &mut Vec<Action>) {
    let open = matches!(&st.popup, Some(Popup::User(p)) if p.key == key);
    match result {
        Ok(()) => {
            if open {
                st.popup = None;
            }
            if let Assignee::User(u) = &who {
                us::remember(&mut st.recent_users, u.clone(), RECENT_USERS);
            }
            let msg = match &who {
                Assignee::Nobody => format!("{key} is now unassigned."),
                w => format!("Assigned {key} to {}.", w.label()),
            };
            st.set_status(MsgKind::Info, msg);
            changed_ticket(st, &key, out);
        }
        Err(e) if open => {
            // The popup stays open with the text, and shows the error.
            if let Some(Popup::User(p)) = st.popup.as_mut() {
                p.sending = false;
                p.error = Some(match &e {
                    Error::Validation { fields, messages } => {
                        let mut all = messages.clone();
                        all.extend(fields.values().cloned());
                        if all.is_empty() { "Jira did not accept the assignee.".into() } else { all.join(" ") }
                    }
                    e => e.to_string(),
                });
            }
            log_error(st, &format!("Assign {key}"), &e);
        }
        Err(e) => report(st, &format!("Assign {key}"), &e),
    }
}

/// `w`: watch or unwatch. jui reads the watch state from the full Ticket.
fn start_watch(st: &mut State, key: String, out: &mut Vec<Action>) {
    if st.watching.contains(&key) {
        st.set_status(MsgKind::Info, format!("The watch request for {key} runs. Wait."));
        return;
    }
    let Some(t) = st.cache.get(&key) else {
        st.set_status(MsgKind::Info, format!("Getting {key}. Push w again when it shows."));
        fetch_ticket(st, &key, out);
        return;
    };
    let on = !t.watching;
    st.watching.insert(key.clone());
    st.set_status(MsgKind::Info, if on { format!("Watching {key}…") } else { format!("Stopping the watch of {key}…") });
    out.push(Action::Watch { key, on });
}

fn watched(st: &mut State, key: String, on: bool, result: Result<(), Error>, out: &mut Vec<Action>) {
    st.watching.remove(&key);
    match result {
        Ok(()) => {
            st.set_status(MsgKind::Info, if on { format!("You now watch {key}.") } else { format!("You no longer watch {key}.") });
            refresh_ticket(st, &key, out);
        }
        Err(e) => report(st, &format!("{} {key}", if on { "Watch" } else { "Unwatch" }), &e),
    }
}

/// A field of the Ticket changed: get it again, and check if it still matches each list.
fn changed_ticket(st: &mut State, key: &str, out: &mut Vec<Action>) {
    refresh_ticket(st, key, out);
    for (tab, t) in st.tabs.iter().enumerate() {
        if let Some(list) = &t.list
            && list.rows.iter().any(|r| r.ticket.key == key)
        {
            let jql = crate::core::model::jql_with_key(&list.jql, key);
            out.push(Action::CheckMatch { tab, key: key.to_string(), jql });
        }
    }
}

// ------------------------------------------------------------------ search and filters

/// Run a JQL query in the Search tab. `filter` is the filter name, when the query comes from a filter.
fn run_search(st: &mut State, jql: &str, filter: Option<String>, out: &mut Vec<Action>) {
    let jql = jql.trim().to_string();
    st.search.set_text(&jql);
    st.search.remember(&jql);
    st.search.history_pos = None;
    st.search.filter = filter;
    st.search.error = None;
    st.search.focus = false;
    st.tabs[SEARCH].zoom.clear();
    let Some(list) = st.tabs[SEARCH].list.as_mut() else { return };
    if list.jql != jql {
        // A different query: the old selection has no meaning.
        list.selected = 0;
    }
    list.jql = jql;
    list.offline = false;
    load_first_page(st, SEARCH, false, out);
}

/// Keys in the JQL input: type, ←→ Home End move the cursor, ↑↓ go through the history,
/// Enter runs the query, Esc and Tab give the focus to the list, Ctrl-F shows the filters.
fn search_key(st: &mut State, k: KeyEvent, out: &mut Vec<Action>) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let s = &mut st.search;
    match k.code {
        KeyCode::Enter => {
            let jql = s.input.clone();
            if jql.trim().is_empty() {
                s.error = Some("Type a JQL query, for example: project = BCE AND status = \"In Progress\"".into());
                return;
            }
            run_search(st, &jql, None, out);
        }
        KeyCode::Esc | KeyCode::Tab => {
            s.focus = false;
            s.history_pos = None;
        }
        KeyCode::Char('f') if ctrl => open_filters(st, out),
        KeyCode::Char('a') if ctrl => s.cursor = 0,
        KeyCode::Char('e') if ctrl => s.cursor = s.input.chars().count(),
        KeyCode::Char('u') if ctrl => {
            s.input.clear();
            s.cursor = 0;
            s.history_pos = None;
        }
        KeyCode::Char('w') if ctrl => {
            // Delete the word before the cursor.
            while s.cursor > 0 && s.input.chars().nth(s.cursor - 1) == Some(' ') {
                s.backspace();
            }
            while s.cursor > 0 && s.input.chars().nth(s.cursor - 1) != Some(' ') {
                s.backspace();
            }
        }
        KeyCode::Char(c) if !ctrl => {
            s.insert(c);
            s.history_pos = None;
        }
        KeyCode::Backspace => {
            s.backspace();
            s.history_pos = None;
        }
        KeyCode::Delete => s.delete(),
        KeyCode::Left => s.cursor = s.cursor.saturating_sub(1),
        KeyCode::Right => s.cursor = (s.cursor + 1).min(s.input.chars().count()),
        KeyCode::Home => s.cursor = 0,
        KeyCode::End => s.cursor = s.input.chars().count(),
        KeyCode::Up if !s.history.is_empty() => {
            let next = match s.history_pos {
                None => {
                    s.draft = s.input.clone();
                    0
                }
                Some(i) => (i + 1).min(s.history.len() - 1),
            };
            s.history_pos = Some(next);
            let text = s.history[next].clone();
            s.set_text(&text);
        }
        KeyCode::Down => match s.history_pos {
            Some(0) => {
                s.history_pos = None;
                let text = std::mem::take(&mut s.draft);
                s.set_text(&text);
            }
            Some(i) => {
                s.history_pos = Some(i - 1);
                let text = s.history[i - 1].clone();
                s.set_text(&text);
            }
            None => {}
        },
        _ => {}
    }
}

/// Open the `Ctrl-F` popup and get the favourite filters. jui gets them each time, so a
/// new favourite in the Jira web UI shows at once.
fn open_filters(st: &mut State, out: &mut Vec<Action>) {
    st.search.focus = false;
    st.popup = Some(Popup::Filters(FilterPopup::default()));
    out.push(Action::FetchFavouriteFilters);
}

fn favourite_filters(st: &mut State, result: Result<Vec<Filter>, Error>) {
    let Some(Popup::Filters(p)) = &mut st.popup else { return };
    match result {
        Ok(fs) => p.list = Some(fs),
        Err(e) => {
            p.list = Some(Vec::new());
            p.error = Some(e.to_string());
            log_error(st, "Get the favourite filters", &e);
        }
    }
}

/// Keys in the filter popup: type to filter (or type a filter ID), ↑↓ select, Enter runs, Esc closes.
fn filters_key(st: &mut State, mut p: FilterPopup, k: KeyEvent, out: &mut Vec<Action>) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shown = p.shown();
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Down => p.selected = (p.selected + 1).min(shown.len().saturating_sub(1)),
        KeyCode::Up => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char('n') if ctrl => p.selected = (p.selected + 1).min(shown.len().saturating_sub(1)),
        KeyCode::Char('p') if ctrl => p.selected = p.selected.saturating_sub(1),
        KeyCode::Backspace => {
            p.input.pop();
            p.selected = 0;
            p.error = None;
        }
        KeyCode::Char(c) if !ctrl => {
            p.input.push(c);
            p.selected = 0;
            p.error = None;
        }
        KeyCode::Enter => {
            let chosen = p.list.as_ref().and_then(|fs| shown.get(p.selected).map(|&i| fs[i].clone()));
            match (chosen, p.typed_id()) {
                (Some(f), _) => {
                    let name = f.name.clone();
                    run_search(st, &f.jql, Some(name), out);
                    return;
                }
                // A filter ID that is not a favourite: get it from Jira.
                (None, Some(id)) if p.loading_id.is_none() => {
                    p.loading_id = Some(id.clone());
                    p.error = None;
                    out.push(Action::FetchFilter { id });
                }
                (None, _) if p.list.is_none() => p.error = Some("Wait for the filters.".into()),
                (None, _) if p.list.as_ref().is_some_and(Vec::is_empty) && p.input.is_empty() => {
                    p.error = Some("You have no favourite filters. Type a filter ID.".into())
                }
                (None, _) => p.error = Some("No match. Type a filter ID to run any filter.".into()),
            }
        }
        _ => {}
    }
    st.popup = Some(Popup::Filters(p));
}

// ------------------------------------------------------------------ Boards

/// The first visit of the Boards tab: show the first Board of My Boards, or the Board picker
/// when My Boards is empty. Later visits keep the state of the tab.
fn boards_first_time(st: &mut State, out: &mut Vec<Action>) {
    let b = &st.boards;
    if b.view.is_some() || b.loading.is_some() || b.error.is_some() {
        return;
    }
    if st.my_boards.is_empty() {
        open_board_picker(st, out);
    } else {
        let i = st.boards.index.min(st.my_boards.len() - 1);
        load_board(st, i, false, out);
    }
}

/// Show Board `index` of My Boards. A Board in the memory cache shows at once, and jui gets
/// it again only with `refresh`. Else, the old Board stays (dim) until the new one arrives.
fn load_board(st: &mut State, index: usize, refresh: bool, out: &mut Vec<Action>) {
    let Some(r) = st.my_boards.get(index).cloned() else { return };
    let b = &mut st.boards;
    b.index = index;
    b.error = None;
    if !refresh && let Some(v) = b.cached(r.id).cloned() {
        b.loading = None;
        b.generation += 1;
        b.show(v, None);
        st.tabs[BOARDS].preview_scroll = 0;
        if let Some(key) = st.selected_key_any() {
            need_ticket(st, &key, false, out);
        }
        return;
    }
    b.keep_key = if refresh { b.selected_key().map(str::to_string) } else { None };
    b.generation += 1;
    b.loading = Some(r.id);
    b.stale = b.view.as_ref().is_some_and(|v| v.board.id != r.id) || (b.view.is_some() && !refresh);
    if refresh {
        b.stale = false;
    }
    out.push(Action::FetchBoard { id: r.id, generation: b.generation, only_mine: b.only_mine });
}

fn board_loaded(st: &mut State, id: u64, generation: u64, result: Result<BoardView, Error>, out: &mut Vec<Action>) {
    if generation != st.boards.generation {
        return; // A late response of an old Board.
    }
    st.boards.loading = None;
    match result {
        Ok(v) => {
            st.boards.missing.remove(&id);
            // jui updates the stored name when the name in Jira changes.
            if let Some(list) = bd::renamed(&st.my_boards, std::slice::from_ref(&v.board)) {
                out.push(Action::SaveBoards { path: st.config_path.clone(), list: list.clone() });
                st.my_boards = list;
            }
            if v.truncated() {
                st.set_status(MsgKind::Warning, "Board has more Tickets. Use the Search tab with JQL.");
            }
            let keep = st.boards.keep_key.take();
            st.boards.show(v, keep.as_deref());
            st.tabs[BOARDS].preview_scroll = 0;
            if st.tab == BOARDS
                && st.tabs[BOARDS].zoom.is_empty()
                && let Some(key) = st.selected_key_any()
            {
                need_ticket(st, &key, true, out);
            }
        }
        Err(e) => {
            st.boards.stale = false;
            st.boards.keep_key = None;
            if matches!(e, Error::NotFound(_)) {
                // jui shows the Board as "not found" and does not remove it. The user removes it.
                st.boards.missing.insert(id, e.to_string());
                st.boards.view = None;
            }
            st.boards.error = Some((id, e.to_string()));
            report(st, &format!("Get Board {id}"), &e);
        }
    }
}

/// Keys on the Boards tab (the columns).
fn board_cmd(st: &mut State, cmd: Cmd, out: &mut Vec<Action>) {
    let half = half_page(st);
    match cmd {
        Cmd::Down => move_card(st, |r, _| r + 1, out),
        Cmd::Up => move_card(st, |r, _| r.saturating_sub(1), out),
        Cmd::First => move_card(st, |_, _| 0, out),
        Cmd::Last => move_card(st, |_, n| n.saturating_sub(1), out),
        Cmd::Left => move_column(st, -1, out),
        Cmd::Right => move_column(st, 1, out),
        Cmd::ScrollDown => st.tabs[BOARDS].preview_scroll = st.tabs[BOARDS].preview_scroll.saturating_add(1),
        Cmd::ScrollUp => st.tabs[BOARDS].preview_scroll = st.tabs[BOARDS].preview_scroll.saturating_sub(1),
        Cmd::PageDown => st.tabs[BOARDS].preview_scroll = st.tabs[BOARDS].preview_scroll.saturating_add(half),
        Cmd::PageUp => st.tabs[BOARDS].preview_scroll = st.tabs[BOARDS].preview_scroll.saturating_sub(half),
        Cmd::Open => {
            if let Some(key) = st.boards.selected_key().map(str::to_string) {
                push_zoom(st, &key, out);
            }
        }
        Cmd::Quit => quit(st, out),
        Cmd::Refresh => {
            if st.my_boards.is_empty() {
                open_board_picker(st, out);
                return;
            }
            if let Some(key) = st.selected_key_any() {
                st.cache.remove(&key);
                st.failed.remove(&key);
            }
            let i = st.boards.index.min(st.my_boards.len() - 1);
            load_board(st, i, true, out);
        }
        Cmd::PrevBoard | Cmd::NextBoard => {
            let n = st.my_boards.len();
            if n == 0 {
                st.set_status(MsgKind::Info, "My Boards is empty. Push + to add Boards.");
                return;
            }
            if n == 1 {
                st.set_status(MsgKind::Info, "My Boards has one Board. Push + to add Boards.");
                return;
            }
            let i = if cmd == Cmd::NextBoard { (st.boards.index + 1) % n } else { (st.boards.index + n - 1) % n };
            load_board(st, i, false, out);
        }
        Cmd::PickBoards => open_board_picker(st, out),
        Cmd::OnlyMine => {
            let on = !st.boards.only_mine;
            // The Board loads again at once. The save of the Setting runs at the same time.
            out.push(Action::SaveSetting { key: "boards.only_mine".into(), value: Some((if on { "on" } else { "off" }).into()) });
            set_only_mine(st, on, out);
            st.set_status(MsgKind::Info, if on { "Board: only my Tickets. m shows all." } else { "Board: all Tickets." });
        }
        Cmd::RemoveBoard => match st.my_boards.get(st.boards.index) {
            Some(r) => {
                st.popup = Some(Popup::ConfirmRemoveBoard { id: r.id, name: bd::ref_label(r) });
            }
            None => st.set_status(MsgKind::Info, "My Boards is empty. Push + to add Boards."),
        },
        _ => {}
    }
}

/// Show all Tickets or only my Tickets on the Boards tab. The shown Board loads again with the
/// new filter, and the selected card stays selected when it is in the new view.
fn set_only_mine(st: &mut State, on: bool, out: &mut Vec<Action>) {
    if st.boards.only_mine == on {
        return;
    }
    st.boards.only_mine = on;
    if st.my_boards.is_empty() || (st.boards.view.is_none() && st.boards.loading.is_none()) {
        return; // The first visit of the tab uses the new filter.
    }
    let i = st.boards.index.min(st.my_boards.len() - 1);
    let keep = st.boards.selected_key().map(str::to_string);
    let cached = st.boards.cached(st.my_boards[i].id).is_some();
    load_board(st, i, false, out);
    if cached {
        // A cached view shows at once: select the same card again.
        if let (Some(k), Some(v)) = (keep, st.boards.view.clone()) {
            st.boards.show(v, Some(&k));
        }
    } else {
        st.boards.keep_key = keep;
        st.boards.stale = true;
    }
}

fn move_card(st: &mut State, f: impl Fn(usize, usize) -> usize, out: &mut Vec<Action>) {
    let b = &mut st.boards;
    if b.stale {
        return;
    }
    let Some(v) = &b.view else { return };
    let Some(col) = v.columns.get(b.col) else { return };
    if col.tickets.is_empty() {
        return;
    }
    let row = b.rows.get(b.col).copied().unwrap_or(0);
    let new = f(row, col.tickets.len()).min(col.tickets.len() - 1);
    if new != row {
        b.rows[b.col] = new;
        st.tabs[BOARDS].preview_scroll = 0;
        if let Some(key) = st.selected_key_any() {
            need_ticket(st, &key, false, out);
        }
    }
}

/// Go to the next column with cards in this direction. Empty columns are skipped.
fn move_column(st: &mut State, dir: isize, out: &mut Vec<Action>) {
    let b = &mut st.boards;
    if b.stale {
        return;
    }
    let Some(v) = &b.view else { return };
    let n = v.columns.len() as isize;
    let mut c = b.col as isize + dir;
    while c >= 0 && c < n && v.columns[c as usize].tickets.is_empty() {
        c += dir;
    }
    if c < 0 || c >= n || c as usize == b.col {
        return;
    }
    b.col = c as usize;
    st.tabs[BOARDS].preview_scroll = 0;
    if let Some(key) = st.selected_key_any() {
        need_ticket(st, &key, false, out);
    }
}

/// A full Ticket arrived: update its card on the shown Board. A new status moves the card
/// to the column of that status. A status in no column keeps the card, in a dim color.
fn update_card(st: &mut State, t: &Ticket) {
    let b = &mut st.boards;
    let (sel_col, sel_row) = (b.col, b.rows.get(b.col).copied());
    let Some(v) = b.view.as_mut() else { return };
    let Some((ci, ri)) =
        v.columns.iter().enumerate().find_map(|(ci, c)| c.tickets.iter().position(|x| x.key == t.key).map(|ri| (ci, ri)))
    else {
        return;
    };
    let summary: crate::core::model::TicketSummary = t.into();
    let target = t.status_id.as_deref().and_then(|id| v.columns.iter().position(|c| c.status_ids.iter().any(|s| s == id)));
    match target {
        Some(to) if to != ci => {
            let selected = sel_col == ci && sel_row == Some(ri);
            v.columns[ci].tickets.remove(ri);
            v.columns[to].tickets.insert(0, summary);
            if b.rows.get(ci).is_some_and(|r| *r >= v.columns[ci].tickets.len()) {
                b.rows[ci] = v.columns[ci].tickets.len().saturating_sub(1);
            }
            if selected {
                // The selection follows the card.
                b.col = to;
                b.rows[to] = 0;
            } else if b.col == to {
                b.rows[to] += 1;
            }
        }
        _ => v.columns[ci].tickets[ri] = summary,
    }
    // The memory cache keeps the old position; the next `r` gets the Board again.
    let id = v.board.id;
    b.cache.retain(|c| c.board.id != id);
}

/// Open the Board picker. jui gets the suggestions each time.
fn open_board_picker(st: &mut State, out: &mut Vec<Action>) {
    st.popup = Some(Popup::Boards(BoardPopup::new(&st.my_boards)));
    out.push(Action::FetchBoardSuggestions { project: st.default_project.clone() });
}

fn board_suggestions(st: &mut State, result: Result<Vec<Suggestion>, Error>) {
    let Some(Popup::Boards(p)) = &mut st.popup else { return };
    match result {
        Ok(s) => p.suggestions = Some(s),
        Err(e) => {
            p.suggestions = Some(Vec::new());
            p.error = Some(e.to_string());
            log_error(st, "Get the Board suggestions", &e);
        }
    }
}

fn boards_found(st: &mut State, seq: u64, result: Result<Vec<Board>, Error>) {
    let Some(Popup::Boards(p)) = &mut st.popup else { return };
    if p.seq != seq {
        return; // A late result of an old text.
    }
    p.searching = false;
    match result {
        Ok(b) => p.results = Some(b),
        Err(e) => {
            p.results = Some(Vec::new());
            p.error = Some(e.to_string());
            log_error(st, "Search Boards", &e);
        }
    }
}

/// Keys in the Board picker: type to search, ↑↓ select, Space marks, Enter saves, Esc cancels.
fn board_popup_key(st: &mut State, mut p: BoardPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.saving {
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Boards(p));
        }
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let rows = p.rows();
    let mut changed = false;
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Down => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Up => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char('n') if ctrl => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Char('p') if ctrl => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char(' ') => match rows.get(p.selected) {
            Some(r) => {
                p.toggle(&r.board);
                p.error = None;
            }
            None => p.error = Some("No Board is selected.".into()),
        },
        KeyCode::Backspace => {
            p.input.pop();
            changed = true;
        }
        KeyCode::Char(c) if !ctrl => {
            p.input.push(c);
            changed = true;
        }
        KeyCode::Enter => {
            if !p.changed() {
                st.set_status(MsgKind::Info, "My Boards did not change.");
                if st.my_boards.is_empty() {
                    st.boards.error = None;
                }
                return;
            }
            p.saving = true;
            p.error = None;
            out.push(Action::SaveBoards { path: st.config_path.clone(), list: p.marked.clone() });
        }
        _ => {}
    }
    if changed {
        p.selected = 0;
        p.error = None;
        // A new key makes the old search out of date. The search starts after the delay.
        p.seq += 1;
        p.results = None;
        p.searching = p.searches();
        if p.searching {
            out.push(Action::ScheduleBoardSearch { seq: p.seq });
        }
    }
    st.popup = Some(Popup::Boards(p));
}

fn remove_board_key(st: &mut State, id: u64, name: String, k: KeyEvent, out: &mut Vec<Action>) {
    match k.code {
        KeyCode::Char('y') => {
            let list = bd::remove(&st.my_boards, &[id]).list;
            out.push(Action::SaveBoards { path: st.config_path.clone(), list });
        }
        KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {}
        _ => st.popup = Some(Popup::ConfirmRemoveBoard { id, name }),
    }
}

/// My Boards was saved. Show the selected Board (or the first one) of the new list.
fn boards_saved(st: &mut State, list: Vec<BoardRef>, result: Result<(), Error>, out: &mut Vec<Action>) {
    let popup = matches!(&st.popup, Some(Popup::Boards(p)) if p.saving);
    if let Err(e) = result {
        if let Some(Popup::Boards(p)) = st.popup.as_mut() {
            p.saving = false;
            p.error = Some(e.to_string());
            log_error(st, "Save My Boards", &e);
        } else {
            report(st, "Save My Boards", &e);
        }
        return;
    }
    if popup {
        st.popup = None;
    }
    // The Settings page shows My Boards from the Settings too.
    st.settings.my_boards = list.clone();
    if !list.is_empty() || st.settings.source("my_boards") == Source::File {
        st.settings.sources.retain(|(k, _)| *k != "my_boards");
        st.settings.sources.push(("my_boards", if list.is_empty() { Source::Default } else { Source::File }));
    }
    // A save of a new name only (after a Board loads) changes nothing on the screen.
    let same_ids = list.iter().map(|r| r.id).eq(st.my_boards.iter().map(|r| r.id));
    if same_ids && !popup {
        st.my_boards = list;
        return;
    }
    let shown = st.boards.view.as_ref().map(|v| v.board.id);
    let added = list.iter().filter(|r| !st.my_boards.iter().any(|m| m.id == r.id)).count();
    let removed = st.my_boards.iter().filter(|m| !list.iter().any(|r| r.id == m.id)).count();
    st.my_boards = list;
    let n = st.settings_rows().len();
    st.settings_page.selected = st.settings_page.selected.min(n - 1);
    let msg = match (added, removed) {
        (0, 0) => "Saved the order of My Boards.".to_string(),
        (a, 0) => format!("Added {a} Board{} to My Boards.", if a == 1 { "" } else { "s" }),
        (0, r) => format!("Removed {r} Board{} from My Boards.", if r == 1 { "" } else { "s" }),
        (a, r) => format!("My Boards: added {a}, removed {r}."),
    };
    st.set_status(MsgKind::Info, msg);
    if st.my_boards.is_empty() {
        st.boards.view = None;
        st.boards.loading = None;
        st.boards.generation += 1;
        st.boards.index = 0;
        return;
    }
    // Keep the shown Board when it stays in the list. Else, show the first new Board, or the first Board.
    let index = shown
        .and_then(|id| st.my_boards.iter().position(|r| r.id == id))
        .or_else(|| (added > 0).then(|| st.my_boards.len() - added))
        .unwrap_or(0);
    if shown.is_some_and(|id| st.my_boards[index].id == id) {
        st.boards.index = index;
    } else if st.tab == BOARDS || shown.is_some() || st.boards.loading.is_some() {
        load_board(st, index, false, out);
    } else {
        // The Boards tab was not opened yet: it shows this Board on its first visit.
        st.boards.index = index;
        st.boards.error = None;
    }
}

fn filter_loaded(st: &mut State, id: String, result: Result<Filter, Error>, out: &mut Vec<Action>) {
    let Some(Popup::Filters(p)) = &mut st.popup else { return };
    if p.loading_id.as_deref() != Some(id.as_str()) {
        return; // The popup closed or the user asked for a different ID.
    }
    p.loading_id = None;
    match result {
        Ok(f) => {
            st.popup = None;
            run_search(st, &f.jql, Some(f.name), out);
        }
        Err(e) => {
            p.error = Some(e.to_string());
            log_error(st, &format!("Get filter {id}"), &e);
        }
    }
}

// ------------------------------------------------------------------ Sprint tab

/// The Board of the Sprint tab: the Board on the Boards tab, else the first Board of My Boards.
fn sprint_board_ref(st: &State) -> Option<(u64, Option<Vec<SprintRef>>)> {
    if let Some(v) = st.boards.view.as_ref().filter(|_| !st.boards.stale) {
        let sprints = v.board.is_scrum().then(|| v.sprints.clone());
        return Some((v.board.id, sprints));
    }
    let i = st.boards.index.min(st.my_boards.len().checked_sub(1)?);
    let id = st.my_boards[i].id;
    // The memory cache of the Boards tab can have the active Sprints.
    let sprints = st.boards.cached(id).filter(|v| v.board.is_scrum()).map(|v| v.sprints.clone());
    Some((id, sprints))
}

/// A visit of the Sprint tab. The tab follows the Board of the Boards tab: when that Board
/// changed, jui gets the new Board. Else the tab keeps its state.
fn sprint_tab_visit(st: &mut State, out: &mut Vec<Action>) {
    let Some((id, sprints)) = sprint_board_ref(st) else {
        st.sprint = SprintTab { only_mine: st.sprint.only_mine, ..SprintTab::default() };
        return;
    };
    if st.sprint.board_id == Some(id) && (st.sprint.data.is_some() || st.sprint.loading) {
        return;
    }
    load_sprint_board(st, id, sprints, None, out);
}

fn load_sprint_board(st: &mut State, id: u64, sprints: Option<Vec<SprintRef>>, refresh: Option<ag::Section>, out: &mut Vec<Action>) {
    let sp = &mut st.sprint;
    if sp.board_id != Some(id) {
        sp.data = None;
        sp.section = 0;
        if let Some(list) = st.tabs[SPRINT].list.as_mut() {
            *list = TicketList::new("");
        }
    }
    let sp = &mut st.sprint;
    sp.board_id = Some(id);
    sp.loading = true;
    sp.error = None;
    sp.generation += 1;
    sp.refresh = refresh;
    out.push(Action::FetchSprintBoard { id, generation: sp.generation, sprints });
}

fn sprint_board_loaded(st: &mut State, id: u64, generation: u64, result: Result<SprintBoard, Error>, out: &mut Vec<Action>) {
    if generation != st.sprint.generation {
        return; // A late response of an old Board.
    }
    st.sprint.loading = false;
    match result {
        Ok(b) => {
            let keep = st.sprint.refresh.take();
            st.sprint.data = Some(b);
            let sections = st.sprint.sections();
            let refresh = keep.is_some();
            st.sprint.section = keep.and_then(|k| sections.iter().position(|s| *s == k)).unwrap_or(0);
            show_section(st, refresh, out);
        }
        Err(e) => {
            st.sprint.refresh = None;
            st.sprint.error = Some(e.to_string());
            report(st, &format!("Get the Sprints of Board {id}"), &e);
        }
    }
}

/// Show the Tickets of the selected part. `keep`: keep the selected Ticket (a refresh).
fn show_section(st: &mut State, keep: bool, out: &mut Vec<Action>) {
    let jql = match (&st.sprint.data, st.sprint.current()) {
        (Some(b), Some(s)) => match ag::section_jql(b, &s) {
            Some(q) if st.sprint.only_mine => ag::only_mine(&q),
            Some(q) => q,
            None => String::new(),
        },
        _ => String::new(),
    };
    let Some(list) = st.tabs[SPRINT].list.as_mut() else { return };
    if list.jql != jql {
        list.selected = 0;
        list.gone.clear();
    }
    list.jql = jql;
    list.offline = false;
    if list.jql.is_empty() {
        list.rows.clear();
        list.total = None;
        list.loading = false;
        list.generation += 1;
        return;
    }
    st.tabs[SPRINT].preview_scroll = 0;
    load_first_page(st, SPRINT, keep, out);
}

/// Show all Tickets or only my Tickets on the Sprint tab. The same part loads again with the
/// new filter, and the selected Ticket stays selected when it is in the new list.
fn set_sprint_only_mine(st: &mut State, on: bool, out: &mut Vec<Action>) {
    if st.sprint.only_mine == on {
        return;
    }
    st.sprint.only_mine = on;
    if st.sprint.data.is_some() {
        show_section(st, true, out);
    }
}

/// `[` and `]` on the Sprint tab: the previous or next active Sprint, or the backlog.
fn change_section(st: &mut State, next: bool, out: &mut Vec<Action>) {
    let n = st.sprint.sections().len();
    if n < 2 {
        let what = if n == 1 { "The Board has only one part to show." } else { "The Board has no Sprints to show." };
        st.set_status(MsgKind::Info, what);
        return;
    }
    let i = st.sprint.section;
    st.sprint.section = if next { (i + 1) % n } else { (i + n - 1) % n };
    show_section(st, false, out);
}

/// `r` on the Sprint tab: get the Sprints again (a Sprint can start or end), then the Tickets.
fn refresh_sprint_tab(st: &mut State, out: &mut Vec<Action>) {
    if let Some(key) = st.selected_key_any() {
        st.cache.remove(&key);
        st.failed.remove(&key);
    }
    let Some(id) = st.sprint.board_id.or_else(|| sprint_board_ref(st).map(|(id, _)| id)) else {
        st.set_status(MsgKind::Info, "My Boards is empty. Push 2, then + to add Boards.");
        return;
    };
    let keep = st.sprint.current().or(Some(ag::Section::Backlog));
    load_sprint_board(st, id, None, keep, out);
}

// ------------------------------------------------------------------ Epics tab

/// Get the Epics of the user again: first the counts (the user's open Tickets in Epics), then
/// the Epics as a Ticket list. The old rows stay (dim) until the new list arrives.
fn load_my_epics(st: &mut State, out: &mut Vec<Action>) {
    st.epics.generation += 1;
    st.epics.loading = true;
    if let Some(list) = st.tabs[EPICS].list.as_mut() {
        list.stale = !list.rows.is_empty();
        list.offline = false;
    }
    out.push(Action::FetchMyEpicCounts { generation: st.epics.generation });
}

fn my_epic_counts(st: &mut State, generation: u64, result: Result<Vec<(String, usize)>, Error>, out: &mut Vec<Action>) {
    if generation != st.epics.generation {
        return; // A late response of an old request.
    }
    st.epics.loading = false;
    match result {
        Ok(counts) => {
            let jql = ag::my_epics_jql(&counts).unwrap_or_default();
            st.epics.counts = counts.into_iter().collect();
            let Some(list) = st.tabs[EPICS].list.as_mut() else { return };
            if jql.is_empty() {
                // No open Ticket of the user is in an Epic.
                list.jql.clear();
                list.rows.clear();
                list.total = Some(0);
                list.selected = 0;
                list.stale = false;
                list.loading = false;
                list.generation += 1;
                return;
            }
            list.jql = jql;
            load_first_page(st, EPICS, true, out);
        }
        Err(e) => {
            if let Some(list) = st.tabs[EPICS].list.as_mut() {
                list.stale = false;
                if matches!(e, Error::Network { .. } | Error::Timeout(_)) {
                    list.offline = true;
                }
            }
            report(st, "Get my Epics", &e);
        }
    }
}

// ------------------------------------------------------------------ Sprint move (`s`)

/// The Board for the `s` popup: the Board of the Sprint tab, else of the Boards tab, else the
/// first Board of My Boards.
fn move_board(st: &State) -> Option<(u64, String)> {
    if let Some(b) = st.sprint.data.as_ref().filter(|b| b.board.is_scrum()) {
        return Some((b.board.id, b.board.name.clone()));
    }
    if let Some(v) = st.boards.view.as_ref().filter(|v| v.board.is_scrum()) {
        return Some((v.board.id, v.board.name.clone()));
    }
    st.my_boards.first().map(|r| (r.id, r.name.clone().unwrap_or_else(|| format!("Board {}", r.id))))
}

fn start_sprint_move(st: &mut State, key: String, out: &mut Vec<Action>) {
    let Some((board_id, board_name)) = move_board(st) else {
        st.set_status(MsgKind::Warning, "My Boards is empty. Add a Scrum Board (2, then +) to move Tickets to its Sprints.");
        return;
    };
    let list = st.open_sprints.get(&board_id).cloned();
    if list.is_none() {
        out.push(Action::FetchOpenSprints { board_id, key: key.clone() });
    }
    st.popup = Some(Popup::Sprint(SprintPopup { key, board_id, board_name, list, filter: String::new(), selected: 0, sending: false, error: None }));
}

fn open_sprints(st: &mut State, board_id: u64, key: String, result: Result<Vec<Sprint>, Error>) {
    match result {
        Ok(list) => {
            let list = ag::open_sprints(list);
            st.open_sprints.insert(board_id, list.clone());
            if let Some(Popup::Sprint(p)) = &mut st.popup
                && p.key == key
                && p.board_id == board_id
            {
                p.list = Some(list);
            }
        }
        Err(e) => {
            if matches!(&st.popup, Some(Popup::Sprint(p)) if p.key == key) {
                st.popup = None;
            }
            report(st, &format!("Get the Sprints of Board {board_id}"), &e);
        }
    }
}

/// Keys in the `s` popup: type to filter, ↑↓ select, Enter move, Esc close.
fn sprint_popup_key(st: &mut State, mut p: SprintPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        // Wait for the answer. Esc hides the popup; the request continues.
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Sprint(p));
        }
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let rows = p.rows();
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Down => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Up => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char('n') if ctrl => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Char('p') if ctrl => p.selected = p.selected.saturating_sub(1),
        KeyCode::Backspace => {
            p.filter.pop();
            p.selected = 0;
            p.error = None;
        }
        KeyCode::Char(c) if !ctrl => {
            p.filter.push(c);
            p.selected = 0;
            p.error = None;
        }
        KeyCode::Enter => match rows.get(p.selected) {
            Some(t) => {
                let current = st.cache.get(&p.key).and_then(|t| t.sprint.as_ref()).filter(|s| !s.state.eq_ignore_ascii_case("closed"));
                let same = match (t, current) {
                    (Target::Sprint(s), Some(c)) => s.id == c.id,
                    (Target::Backlog, None) => st.cache.contains(&p.key),
                    _ => false,
                };
                if same {
                    p.error = Some(format!("{} is already in {}.", p.key, t.label()));
                } else {
                    p.sending = true;
                    p.error = None;
                    out.push(Action::MoveSprint { key: p.key.clone(), target: t.clone() });
                }
            }
            None if p.list.is_none() => p.error = Some("Wait for the Sprints.".into()),
            None => p.error = Some("No match.".into()),
        },
        _ => {}
    }
    st.popup = Some(Popup::Sprint(p));
}

fn sprint_moved(st: &mut State, key: String, target: Target, result: Result<(), Error>, out: &mut Vec<Action>) {
    let open = matches!(&st.popup, Some(Popup::Sprint(p)) if p.key == key);
    match result {
        Ok(()) => {
            if open {
                st.popup = None;
            }
            st.set_status(MsgKind::Info, format!("Moved {key} to {}.", target.label()));
            changed_ticket(st, &key, out);
            // The Boards tab shows the active Sprints. A move in or out changes the Board.
            if let Some(v) = &st.boards.view {
                let id = v.board.id;
                st.boards.cache.retain(|c| c.board.id != id);
            }
        }
        Err(e) if open => {
            if let Some(Popup::Sprint(p)) = st.popup.as_mut() {
                p.sending = false;
                p.error = Some(match &e {
                    Error::Validation { fields, messages } => {
                        let mut all = messages.clone();
                        all.extend(fields.values().cloned());
                        if all.is_empty() { "Jira did not accept the move.".into() } else { all.join(" ") }
                    }
                    e => e.to_string(),
                });
            }
            log_error(st, &format!("Move {key} to {}", target.label()), &e);
        }
        Err(e) => report(st, &format!("Move {key} to {}", target.label()), &e),
    }
}

// ------------------------------------------------------------------ Epic picker (`E`)

fn start_epic_pick(st: &mut State, key: String, out: &mut Vec<Action>) {
    if st.cache.get(&key).is_some_and(|t| t.kind.eq_ignore_ascii_case("epic")) {
        st.set_status(MsgKind::Warning, format!("{key} is an Epic. An Epic cannot be in an Epic."));
        return;
    }
    let project = key.split('-').next().unwrap_or(&st.default_project).to_string();
    out.push(Action::SearchEpics { key: key.clone(), project: project.clone(), text: String::new(), seq: 0 });
    st.popup = Some(Popup::Epic(EpicPopup { key, project, ..Default::default() }));
}

fn epics(st: &mut State, key: String, seq: u64, result: Result<Vec<Epic>, Error>) {
    let Some(Popup::Epic(p)) = &mut st.popup else { return };
    if p.key != key {
        return;
    }
    if seq == 0 {
        match result {
            Ok(list) => p.base = Some(list),
            Err(e) => {
                p.base = Some(Vec::new());
                p.error = Some(e.to_string());
                log_error(st, &format!("Get the Epics of {key}"), &e);
            }
        }
        return;
    }
    if p.seq != seq {
        return; // A late result of an old text.
    }
    p.searching = false;
    match result {
        Ok(list) => p.results = Some(list),
        Err(e) => {
            p.results = Some(Vec::new());
            p.error = Some(e.to_string());
            log_error(st, "Search Epics", &e);
        }
    }
}

/// Keys in the Epic picker: type to search, ↑↓ select, Enter set, Esc close.
fn epic_popup_key(st: &mut State, mut p: EpicPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Epic(p));
        }
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let rows = p.rows();
    let mut changed = false;
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Down => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Up => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char('n') if ctrl => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Char('p') if ctrl => p.selected = p.selected.saturating_sub(1),
        KeyCode::Backspace => {
            p.input.pop();
            changed = true;
        }
        KeyCode::Char(c) if !ctrl => {
            p.input.push(c);
            changed = true;
        }
        KeyCode::Enter => {
            let current = st.cache.get(&p.key).and_then(|t| t.epic.as_ref()).map(|e| e.key.clone());
            match rows.get(p.selected) {
                Some(EpicRow::Nobody) if current.is_none() && st.cache.contains(&p.key) => {
                    p.error = Some(format!("{} has no Epic.", p.key));
                }
                Some(EpicRow::Epic(e)) if current.as_deref() == Some(e.key.as_str()) => {
                    p.error = Some(format!("{} is already in {}.", p.key, e.key));
                }
                Some(EpicRow::Epic(e)) if e.key == p.key => p.error = Some("A Ticket cannot be its own Epic.".into()),
                Some(row) => {
                    let epic = match row {
                        EpicRow::Nobody => None,
                        EpicRow::Epic(e) => Some(e.clone()),
                    };
                    p.sending = true;
                    p.error = None;
                    out.push(Action::SetEpic { key: p.key.clone(), epic, old: current });
                }
                None if p.searching => p.error = Some("Wait for the search.".into()),
                None => p.error = Some("No match.".into()),
            }
        }
        _ => {}
    }
    if changed {
        p.selected = 0;
        p.error = None;
        p.seq += 1;
        p.results = None;
        p.searching = p.searches();
        if p.searching {
            out.push(Action::ScheduleEpicSearch { seq: p.seq });
        }
    }
    st.popup = Some(Popup::Epic(p));
}

fn epic_set(st: &mut State, key: String, epic: Option<Epic>, old: Option<String>, result: Result<(), Error>, out: &mut Vec<Action>) {
    let open = matches!(&st.popup, Some(Popup::Epic(p)) if p.key == key);
    match result {
        Ok(()) => {
            if open {
                st.popup = None;
            }
            let msg = match &epic {
                Some(e) => format!("Set the Epic of {key} to {} ({}).", e.key, e.name),
                None => format!("Removed the Epic of {key}."),
            };
            st.set_status(MsgKind::Info, msg);
            changed_ticket(st, &key, out);
            // The old and the new Epic have a different list of child Tickets now.
            for k in old.into_iter().chain(epic.map(|e| e.key)) {
                if st.epic_children.remove(&k).is_some() {
                    st.zoom_render.key = None;
                    if st.tabs.iter().any(|t| t.zoom.last().is_some_and(|z| z.key == k)) {
                        need_epic_children(st, &k, out);
                    }
                }
            }
        }
        Err(e) if open => {
            if let Some(Popup::Epic(p)) = st.popup.as_mut() {
                p.sending = false;
                p.error = Some(match &e {
                    Error::Validation { fields, messages } => {
                        let mut all = messages.clone();
                        all.extend(fields.values().cloned());
                        if all.is_empty() { "Jira did not accept the Epic.".into() } else { all.join(" ") }
                    }
                    e => e.to_string(),
                });
            }
            log_error(st, &format!("Set the Epic of {key}"), &e);
        }
        Err(e) => report(st, &format!("Set the Epic of {key}"), &e),
    }
}

// ------------------------------------------------------------------ Links

fn link_types(st: &mut State, result: Result<Vec<LinkType>, Error>) {
    match result {
        Ok(t) => st.link_types = Some(t),
        Err(e) => {
            if let Some(Popup::Link(p)) = st.popup.as_mut() {
                p.error = Some(format!("Cannot get the Link types: {e}"));
            }
            log_error(st, "Get the Link types", &e);
        }
    }
}

/// Keys in the `l` popup. First select the relation (type to filter, ↑↓, Enter or Tab),
/// then type the key of the other Ticket (Enter adds the Link). Esc closes.
fn link_key(st: &mut State, mut p: LinkPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Link(p));
        }
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shown = st.link_types.as_deref().map(|t| p.shown(t)).unwrap_or_default();
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Tab | KeyCode::BackTab => {
            p.on_target = !p.on_target && !shown.is_empty();
        }
        KeyCode::Up if !p.on_target => p.selected = p.selected.saturating_sub(1),
        KeyCode::Down if !p.on_target => p.selected = (p.selected + 1).min(shown.len().saturating_sub(1)),
        KeyCode::Backspace if p.on_target && p.target.is_empty() => p.on_target = false,
        KeyCode::Backspace if p.on_target => {
            p.target.pop();
        }
        KeyCode::Backspace => {
            p.filter.pop();
            p.selected = 0;
        }
        KeyCode::Char(c) if !ctrl && p.on_target => p.target.push(c),
        KeyCode::Char(c) if !ctrl => {
            p.filter.push(c);
            p.selected = 0;
        }
        KeyCode::Enter if !p.on_target => {
            if !shown.is_empty() {
                p.on_target = true;
            }
        }
        KeyCode::Enter => {
            p.error = None;
            let Some(rel) = shown.get(p.selected).cloned() else {
                p.on_target = false;
                st.popup = Some(Popup::Link(p));
                return;
            };
            let to = crate::core::model::normalize_key(p.target.trim(), &st.default_project);
            if !crate::core::model::looks_like_key(&to) {
                p.error = Some(format!("`{}` is not a Ticket key.", p.target.trim()));
                st.popup = Some(Popup::Link(p));
                return;
            }
            match LinkPlan::new(&p.key, rel, &to, None) {
                Err(e) => p.error = Some(e.to_string()),
                Ok(plan) => {
                    let exists = st.cache.get(&p.key).is_some_and(|t| plan.exists_in(&t.links));
                    if exists {
                        st.set_status(MsgKind::Info, format!("The Link exists: {}. Nothing changed.", plan.describe()));
                        return;
                    }
                    p.sending = true;
                    out.push(Action::AddLink { plan });
                }
            }
        }
        _ => {}
    }
    st.popup = Some(Popup::Link(p));
}

fn link_added(st: &mut State, plan: LinkPlan, result: Result<(), Error>, out: &mut Vec<Action>) {
    let open = matches!(&st.popup, Some(Popup::Link(p)) if p.key == plan.from && p.sending);
    match result {
        Ok(()) => {
            if open {
                st.popup = None;
            }
            st.set_status(MsgKind::Info, format!("Added the Link: {}.", plan.describe()));
            // Both Tickets have the new Link.
            st.cache.remove(&plan.to);
            refresh_ticket(st, &plan.from, out);
        }
        Err(e) if open => {
            if let Some(Popup::Link(p)) = st.popup.as_mut() {
                p.sending = false;
                p.error = Some(e.to_string());
            }
            log_error(st, &format!("Add the Link {}", plan.describe()), &e);
        }
        Err(e) => report(st, &format!("Add the Link {}", plan.describe()), &e),
    }
}

// ------------------------------------------------------------------ attachments

/// Keys in the `u` popup: type or paste the path, Enter uploads, Esc closes.
fn upload_key(st: &mut State, mut p: UploadPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        // Esc hides the popup; the upload continues.
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Upload(p));
        }
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Char('u') if ctrl => {
            p.input.clear();
            p.error = None;
        }
        KeyCode::Backspace => {
            p.input.pop();
            p.error = None;
        }
        KeyCode::Char(c) if !ctrl => {
            p.input.push(c);
            p.error = None;
        }
        KeyCode::Enter if p.input.trim().is_empty() => p.error = Some("Type or paste the path of a file.".into()),
        KeyCode::Enter => {
            p.error = None;
            p.sending = true;
            out.push(Action::Upload { key: p.key.clone(), path: p.input.clone() });
        }
        _ => {}
    }
    st.popup = Some(Popup::Upload(p));
}

fn uploaded(st: &mut State, key: String, result: Result<Vec<Attachment>, Error>, out: &mut Vec<Action>) {
    let open = matches!(&st.popup, Some(Popup::Upload(p)) if p.key == key && p.sending);
    match result {
        Ok(added) => {
            if open {
                st.popup = None;
            }
            let names = added.iter().map(|a| a.filename.as_str()).collect::<Vec<_>>().join(", ");
            st.set_status(MsgKind::Info, format!("Uploaded {names} to {key}."));
            refresh_ticket(st, &key, out);
        }
        // A bad path or a big file: the user corrects the path in the popup.
        Err(e) if open => {
            if let Some(Popup::Upload(p)) = st.popup.as_mut() {
                p.sending = false;
                p.error = Some(e.to_string());
            }
            log_error(st, &format!("Upload a file to {key}"), &e);
        }
        Err(e) => report(st, &format!("Upload a file to {key}"), &e),
    }
}

// ------------------------------------------------------------------ Worklogs

/// Keys in the `L` popup: Tab and ↑↓ move between the fields, Enter adds the Worklog,
/// `Ctrl-E` opens the Worklog Template with the values, Esc closes.
fn worklog_key(st: &mut State, mut p: WorklogPopup, k: KeyEvent, out: &mut Vec<Action>) {
    if p.sending {
        if k.code != KeyCode::Esc {
            st.popup = Some(Popup::Worklog(p));
        }
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let n = p.inputs.len();
    match k.code {
        KeyCode::Esc => return,
        KeyCode::Char('e') if ctrl => {
            let q = p.quick();
            start_worklog(st, p.key, WorklogStart::Add(q), out);
            return;
        }
        KeyCode::Tab | KeyCode::Down => p.focus = (p.focus + 1) % n,
        KeyCode::BackTab | KeyCode::Up => p.focus = (p.focus + n - 1) % n,
        KeyCode::Backspace => {
            p.inputs[p.focus].pop();
            p.errors[p.focus] = None;
        }
        KeyCode::Char(c) if !ctrl => {
            p.inputs[p.focus].push(c);
            p.errors[p.focus] = None;
        }
        KeyCode::Enter => {
            p.errors = Default::default();
            p.general = None;
            match wl::quick_plan(&p.quick(), &st.clock()) {
                Ok(plan) => {
                    p.sending = true;
                    out.push(Action::SendWorklog { key: p.key.clone(), plan });
                }
                Err((field, msg)) => match field.as_str() {
                    "time_spent" => {
                        p.errors[0] = Some(msg);
                        p.focus = 0;
                    }
                    "started" => {
                        p.errors[1] = Some(msg);
                        p.focus = 1;
                    }
                    _ => p.general = Some(msg),
                },
            }
        }
        _ => {}
    }
    st.popup = Some(Popup::Worklog(p));
}

/// Start a Worklog add or edit in the editor. Only one job runs at a time.
fn start_worklog(st: &mut State, key: String, start: WorklogStart, out: &mut Vec<Action>) {
    if st.job_running() {
        st.set_status(MsgKind::Warning, "A create or edit is running. Wait until it is done.");
        return;
    }
    let what = match &start {
        WorklogStart::Add(_) => format!("Getting the time tracking of {key}…"),
        WorklogStart::Edit { id } => format!("Getting Worklog {id} for the edit…"),
    };
    st.set_status(MsgKind::Info, what);
    out.push(Action::PrepareWorklog { key, start });
}

fn worklog_ready(st: &mut State, key: String, start: WorklogStart, result: Result<WorklogContext, Error>, out: &mut Vec<Action>) {
    let ctx = match result {
        Ok(c) => c,
        Err(e) => {
            report(st, &format!("Worklog on {key}"), &e);
            return;
        }
    };
    let session = match &start {
        WorklogStart::Add(q) => WorklogSession::add(&ctx, q, st.clock()),
        WorklogStart::Edit { id } => match WorklogSession::edit(&ctx, id, st.clock()) {
            Ok(s) => s,
            Err(e) => {
                report(st, &format!("Edit Worklog {id} on {key}"), &e);
                return;
            }
        },
    };
    st.status = None;
    out.push(Action::OpenEditor { text: session.text().to_string(), label: session.label() });
    st.worklog_job = Some(WorklogJob { session, plan: None });
}

fn worklog_edited(st: &mut State, result: Result<String, Error>, out: &mut Vec<Action>) {
    let Some(job) = st.worklog_job.as_mut() else { return };
    let text = match result {
        Ok(t) => t,
        Err(e) => {
            draft(st, format!("The editor failed: {e}"), out);
            log_error(st, "Worklog", &e);
            return;
        }
    };
    let step = job.session.submit(&text);
    worklog_step(st, step, out);
}

fn worklog_step(st: &mut State, step: WorklogStep, out: &mut Vec<Action>) {
    let Some(job) = st.worklog_job.as_mut() else { return };
    match step {
        WorklogStep::Cancel(m) => {
            st.worklog_job = None;
            st.set_status(MsgKind::Info, m);
        }
        WorklogStep::Reopen => out.push(Action::OpenEditor { text: job.session.text().to_string(), label: job.session.label() }),
        WorklogStep::Stop(m) => draft(st, m, out),
        WorklogStep::Send(plan) => {
            let key = job.session.key.clone();
            let id = plan.id.clone();
            job.plan = Some(plan);
            match id {
                // An edit: read the Worklog again, then show the diff when the comment changes.
                Some(id) => {
                    st.set_status(MsgKind::Info, format!("Checking Worklog {id} before the change…"));
                    out.push(Action::RecheckWorklog { key, id });
                }
                None => send_worklog_plan(st, out),
            }
        }
    }
}

fn worklog_rechecked(st: &mut State, key: String, id: String, result: Result<Worklog, Error>, out: &mut Vec<Action>) {
    let Some(job) = st.worklog_job.as_ref() else { return };
    let Some(plan) = job.plan.clone() else { return };
    let now = match result {
        Ok(w) => w,
        Err(e) => {
            draft(st, format!("jui could not read Worklog {id} again."), out);
            report(st, &format!("Edit Worklog {id} on {key}"), &e);
            return;
        }
    };
    let conflict = job.session.old().and_then(|before| wl::changed_in_jira(&key, before, &now));
    if !plan.needs_diff() && conflict.is_none() {
        send_worklog_plan(st, out);
        return;
    }
    st.status = None;
    let lines = if plan.needs_diff() { tpl::diff(plan.old_comment.as_deref().unwrap_or(""), &plan.comment) } else { vec![] };
    st.popup = Some(Popup::Diff(DiffPopup {
        conflict,
        key,
        subject: format!("The comment of Worklog {id}"),
        lines,
        warnings: plan.warnings.clone(),
        fields: vec![],
        scroll: 0,
    }));
}

fn send_worklog_plan(st: &mut State, out: &mut Vec<Action>) {
    let Some(job) = st.worklog_job.as_ref() else { return };
    let Some(plan) = job.plan.clone() else { return };
    let key = job.session.key.clone();
    let what = match &plan.id {
        Some(id) => format!("Saving Worklog {id}…"),
        None => format!("Adding the Worklog to {key}…"),
    };
    st.set_status(MsgKind::Info, what);
    out.push(Action::SendWorklog { key, plan });
}

fn worklog_sent(st: &mut State, key: String, id: Option<String>, result: Result<Worklog, Error>, out: &mut Vec<Action>) {
    let popup_open = id.is_none() && matches!(&st.popup, Some(Popup::Worklog(p)) if p.key == key && p.sending);
    let from_job = !popup_open && st.worklog_job.is_some();
    match result {
        Ok(w) => {
            let plan = if popup_open { None } else { st.worklog_job.take().and_then(|j| j.plan) };
            if popup_open {
                st.popup = None;
            }
            let what = plan.as_ref().map(WorklogPlan::describe).unwrap_or_else(|| human(&w));
            let msg = match &id {
                Some(id) => format!("Saved Worklog {id} on {key}: {what}."),
                None => format!("Added Worklog {} to {key}: {what}.", w.id),
            };
            let warnings = plan.map(|p| p.warnings).unwrap_or_default();
            if warnings.is_empty() {
                st.set_status(MsgKind::Info, msg);
            } else {
                st.set_status(MsgKind::Warning, format!("{msg} warning: {}", warnings.join("; ")));
            }
            refresh_ticket(st, &key, out);
        }
        Err(e) if popup_open => {
            let Some(Popup::Worklog(p)) = st.popup.as_mut() else { return };
            p.sending = false;
            // The popup stays open with the values of the user, and shows the error.
            match &e {
                Error::Validation { fields, messages } => {
                    let mut general = messages.clone();
                    for (f, m) in fields {
                        match f.as_str() {
                            "timeLogged" | "timeSpent" | "timeSpentSeconds" => p.errors[0] = Some(m.clone()),
                            "started" | "startDate" => p.errors[1] = Some(m.clone()),
                            _ => general.push(format!("{f}: {m}")),
                        }
                    }
                    if general.is_empty() && p.errors.iter().all(Option::is_none) {
                        general.push("Jira did not accept the Worklog.".into());
                    }
                    p.general = (!general.is_empty()).then(|| general.join(" "));
                }
                e => p.general = Some(e.to_string()),
            }
            log_error(st, &format!("Worklog on {key}"), &e);
        }
        Err(e) if from_job => {
            let Some(job) = st.worklog_job.as_mut() else { return };
            job.plan = None;
            match job.session.rejected(e) {
                Ok(step) => {
                    log_error(st, "Worklog", &Error::input("Jira did not accept the Worklog. See the # ERROR: lines."));
                    worklog_step(st, step, out);
                }
                Err(e) => {
                    draft(st, format!("jui could not send the Worklog to {key}."), out);
                    report(st, &format!("Worklog on {key}"), &e);
                }
            }
        }
        // The popup was hidden with Esc during the request.
        Err(e) => report(st, &format!("Worklog on {key}"), &e),
    }
}

/// "1h 30m on 2026-10-19 10:00" for a Worklog that Jira returned.
fn human(w: &Worklog) -> String {
    let t = crate::core::format::human_duration(w.time_spent_seconds);
    match w.started {
        Some(d) => format!("{t} on {}", d.format(wl::STARTED_FORMAT)),
        None => t,
    }
}

// ------------------------------------------------------------------ Settings page

/// Keys on the Settings page.
fn settings_cmd(st: &mut State, cmd: Cmd, out: &mut Vec<Action>) {
    let n = st.settings_rows().len();
    let page = &mut st.settings_page;
    let half = (st.body_height / 2).max(1) as usize;
    match cmd {
        Cmd::Down => page.selected = (page.selected + 1).min(n - 1),
        Cmd::Up => page.selected = page.selected.saturating_sub(1),
        Cmd::First => page.selected = 0,
        Cmd::Last => page.selected = n - 1,
        Cmd::PageDown => page.selected = (page.selected + half).min(n - 1),
        Cmd::PageUp => page.selected = page.selected.saturating_sub(half),
        Cmd::Open => edit_setting(st, out),
        Cmd::ResetSetting => reset_setting(st, out),
        Cmd::ScrollUp => move_board_row(st, -1, out),
        Cmd::ScrollDown => move_board_row(st, 1, out),
        Cmd::EditConfig => {
            if st.job_running() {
                st.set_status(MsgKind::Warning, "A different edit is running. Wait until it is done.");
                return;
            }
            st.config_edit = true;
            out.push(Action::EditConfig { original: None, text: None });
        }
        Cmd::Refresh => out.push(Action::ReloadSettings),
        Cmd::Quit => quit(st, out),
        _ => {}
    }
}

/// Enter on the Settings page: open the edit popup, or the Board picker on the My Boards rows.
fn edit_setting(st: &mut State, out: &mut Vec<Action>) {
    let meta = match st.selected_setting() {
        Some(SettingsRow::Value(m)) => m,
        Some(SettingsRow::Board(_) | SettingsRow::AddBoards) => {
            open_board_picker(st, out);
            return;
        }
        None => return,
    };
    let current = st.settings.page_value(meta.key).unwrap_or_default();
    let mut p =
        SettingPopup { meta, input: String::new(), choices: None, selected: 0, checking: false, saving: false, error: None };
    match meta.kind {
        Kind::Choice(values) => {
            p.choices = Some(values.iter().map(|v| v.to_string()).collect());
            p.selected = values.iter().position(|v| *v == current).unwrap_or(0);
        }
        Kind::TicketType => match &st.ticket_types {
            Some((project, types)) if *project == st.default_project => {
                p.selected = types.iter().position(|t| t.eq_ignore_ascii_case(&current)).unwrap_or(0);
                p.choices = Some(types.clone());
            }
            _ => out.push(Action::FetchTicketTypes { project: st.default_project.clone() }),
        },
        _ => p.input = current,
    }
    st.popup = Some(Popup::Setting(p));
}

/// Keys in the edit popup of one Setting.
fn setting_popup_key(st: &mut State, mut p: SettingPopup, k: KeyEvent, out: &mut Vec<Action>) {
    // Esc closes the popup also during a check. A late result is ignored, and a late save
    // shows in the status line.
    if k.code == KeyCode::Esc {
        return;
    }
    if p.busy() {
        st.popup = Some(Popup::Setting(p));
        return;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let list = p.is_list();
    let shown = p.shown();
    match k.code {
        KeyCode::Down if list => p.selected = (p.selected + 1).min(shown.len().saturating_sub(1)),
        KeyCode::Up if list => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char('n') if ctrl && list => p.selected = (p.selected + 1).min(shown.len().saturating_sub(1)),
        KeyCode::Char('p') if ctrl && list => p.selected = p.selected.saturating_sub(1),
        KeyCode::Char('u') if ctrl => {
            p.input.clear();
            p.selected = 0;
            p.error = None;
        }
        KeyCode::Backspace => {
            p.input.pop();
            p.selected = 0;
            p.error = None;
        }
        KeyCode::Char(c) if !ctrl => {
            if p.meta.kind == Kind::Number && !c.is_ascii_digit() {
                p.error = Some("Type only digits.".into());
            } else {
                p.input.push(c);
                p.selected = 0;
                p.error = None;
            }
        }
        KeyCode::Enter => {
            let value = if list {
                match shown.get(p.selected) {
                    Some(v) => v.clone(),
                    // The Ticket types are not available: the typed text is the value.
                    None if p.meta.kind == Kind::TicketType && p.choices.as_ref().is_some_and(Vec::is_empty) => p.input.trim().to_string(),
                    None => {
                        p.error = Some("No value matches. Push Ctrl-U to see all values.".into());
                        st.popup = Some(Popup::Setting(p));
                        return;
                    }
                }
            } else {
                p.input.trim().to_string()
            };
            submit_setting(st, p, value, out);
            return;
        }
        _ => {}
    }
    st.popup = Some(Popup::Setting(p));
}

/// Check a value and save it. A project key is checked on the server first.
fn submit_setting(st: &mut State, mut p: SettingPopup, value: String, out: &mut Vec<Action>) {
    let key = p.meta.key;
    let item = match cfg::validate(key, &value) {
        Ok(item) => item,
        Err(e) => {
            p.error = Some(match e.hint() {
                Some(h) => format!("{e} {h}"),
                None => e.to_string(),
            });
            st.popup = Some(Popup::Setting(p));
            return;
        }
    };
    // The value as jui writes it (for example, an upper-case project key).
    let value = item.as_str().map(str::to_string).unwrap_or_else(|| item.to_string().trim().to_string());
    let current = st.settings.page_value(key).unwrap_or_default();
    if value == current && st.settings.source(key) != Source::Default {
        st.set_status(MsgKind::Info, format!("{} did not change.", p.meta.label));
        return;
    }
    p.error = None;
    if p.meta.kind == Kind::Project {
        p.checking = true;
        p.input = value.clone();
        out.push(Action::CheckProject { input: value });
    } else {
        p.saving = true;
        out.push(Action::SaveSetting { key: key.to_string(), value: Some(value) });
    }
    st.popup = Some(Popup::Setting(p));
}

fn project_checked(st: &mut State, input: String, result: Result<String, Error>, out: &mut Vec<Action>) {
    let Some(Popup::Setting(p)) = &mut st.popup else { return };
    if !p.checking || p.input != input {
        return; // The popup closed, or the text changed.
    }
    p.checking = false;
    match result {
        Ok(key) => {
            p.saving = true;
            out.push(Action::SaveSetting { key: p.meta.key.to_string(), value: Some(key) });
        }
        Err(Error::NotFound(_)) => p.error = Some(format!("Project {input} does not exist, or you cannot see it.")),
        Err(e) => {
            p.error = Some(e.to_string());
            log_error(st, &format!("Check project {input}"), &e);
        }
    }
}

fn ticket_types(st: &mut State, project: String, result: Result<Vec<String>, Error>) {
    match result {
        Ok(types) => {
            st.ticket_types = Some((project.clone(), types.clone()));
            if let Some(Popup::Setting(p)) = &mut st.popup
                && p.meta.kind == Kind::TicketType
                && p.choices.is_none()
            {
                p.selected = types.iter().position(|t| t.eq_ignore_ascii_case(&st.default_type)).unwrap_or(0);
                p.choices = Some(types);
            }
        }
        Err(e) => {
            if let Some(Popup::Setting(p)) = &mut st.popup
                && p.meta.kind == Kind::TicketType
            {
                p.choices = Some(Vec::new());
                p.error = Some(format!("{e} Type the name of the type."));
            }
            log_error(st, &format!("Get the Ticket types of {project}"), &e);
        }
    }
}

/// `d` on the Settings page: remove the key from the file (the default value), or remove
/// the selected Board from My Boards.
fn reset_setting(st: &mut State, out: &mut Vec<Action>) {
    match st.selected_setting() {
        Some(SettingsRow::Value(m)) => {
            if st.settings.source(m.key) == Source::Default {
                st.set_status(MsgKind::Info, format!("{} already uses the default value.", m.label));
                return;
            }
            // An env value that is not in the file: nothing to remove.
            if st.settings.source(m.key) == Source::Env && st.settings.file_base_url.is_none() {
                let env = cfg::env_name(m.key).unwrap_or("An environment variable");
                st.set_status(MsgKind::Info, format!("The file has no value. {env} replaces the default value."));
                return;
            }
            out.push(Action::SaveSetting { key: m.key.to_string(), value: None });
        }
        Some(SettingsRow::Board(i)) => {
            let Some(r) = st.my_boards.get(i) else { return };
            st.popup = Some(Popup::ConfirmRemoveBoard { id: r.id, name: bd::ref_label(r) });
        }
        Some(SettingsRow::AddBoards) => st.set_status(MsgKind::Info, "Select a Board to remove it, or push Enter for the Board picker."),
        None => {}
    }
}

/// `K` and `J` on a Board row: move the Board up or down in My Boards.
fn move_board_row(st: &mut State, dir: isize, out: &mut Vec<Action>) {
    let Some(SettingsRow::Board(i)) = st.selected_setting() else {
        st.set_status(MsgKind::Info, "K and J change the order of My Boards. Select a Board first.");
        return;
    };
    let j = i as isize + dir;
    if j < 0 || j as usize >= st.my_boards.len() {
        return;
    }
    let mut list = st.my_boards.clone();
    list.swap(i, j as usize);
    st.settings_page.selected = (st.settings_page.selected as isize + dir) as usize;
    out.push(Action::SaveBoards { path: st.config_path.clone(), list });
}

fn setting_saved(st: &mut State, key: String, result: Result<Settings, Error>, out: &mut Vec<Action>) {
    let label = cfg::meta(&key).map(|m| m.label).unwrap_or(key.as_str()).to_string();
    match result {
        Ok(s) => {
            if matches!(&st.popup, Some(Popup::Setting(p)) if p.meta.key == key) {
                st.popup = None;
            }
            let reset = s.source(&key) == Source::Default
                || (s.source(&key) == Source::Env && s.file_base_url.is_none());
            let path = s.path.display().to_string();
            let warning = s.warnings.first().cloned();
            use_settings(st, s, out);
            let msg = if reset {
                format!("{label} uses the default value. Saved to {path}")
            } else {
                format!("Saved {label} to {path}")
            };
            match warning {
                Some(w) => st.set_status(MsgKind::Warning, format!("{msg}. warning: {w}")),
                None => st.set_status(MsgKind::Info, msg),
            }
        }
        Err(e) => match &mut st.popup {
            Some(Popup::Setting(p)) if p.meta.key == key => {
                p.saving = false;
                p.error = Some(e.to_string());
                log_error(st, &format!("Save {label}"), &e);
            }
            _ => report(st, &format!("Save {label}"), &e),
        },
    }
}

/// Use Settings that jui read again from the file. The new values have an effect at once.
fn use_settings(st: &mut State, s: Settings, out: &mut Vec<Action>) {
    let old = st.settings.clone();
    let old_ids: Vec<u64> = st.my_boards.iter().map(|r| r.id).collect();
    let jql_changed = st.apply_settings(s);
    let new = &st.settings;
    let connection = new.base_url != old.base_url || new.timeout_secs != old.timeout_secs || new.editor_command != old.editor_command;
    if connection {
        out.push(Action::UseConnection {
            base_url: new.base_url.clone(),
            timeout_secs: new.timeout_secs,
            editor_command: new.editor_command.clone(),
        });
    }
    let server_changed = new.base_url != old.base_url;
    if server_changed {
        st.base_url = st.settings.base_url.clone();
        st.cache.clear();
        st.failed.clear();
    }
    if new.date_format != old.date_format {
        st.preview_render.key = None;
        st.zoom_render.key = None;
    }
    if jql_changed || server_changed {
        // A new query: the old rows stay (dim) until the first page of the new query arrives.
        st.tabs[MY_TICKETS].zoom.clear();
        load_first_page(st, MY_TICKETS, false, out);
    } else if old.group_by_sprint && !st.settings.group_by_sprint {
        // The rows are in Sprint order. Get them again in the order of the query.
        load_first_page(st, MY_TICKETS, true, out);
    }
    // A change on the Settings page or in the file. After `m`, the tab already has the value,
    // so `set_only_mine` does nothing. Only a change of the value is used: after two quick
    // pushes of `m`, the last save gives the last value.
    if st.settings.boards_only_mine != old.boards_only_mine {
        let only_mine = st.settings.boards_only_mine;
        set_only_mine(st, only_mine, out);
    }
    if st.settings.sprint_only_mine != old.sprint_only_mine {
        let only_mine = st.settings.sprint_only_mine;
        set_sprint_only_mine(st, only_mine, out);
    }
    // My Boards changed in the file: the Boards tab starts again on its next visit.
    let ids: Vec<u64> = st.my_boards.iter().map(|r| r.id).collect();
    if ids != old_ids {
        let shown = st.boards.view.as_ref().map(|v| v.board.id);
        match shown.and_then(|id| ids.iter().position(|x| *x == id)) {
            Some(i) => st.boards.index = i,
            None => {
                st.boards.view = None;
                st.boards.loading = None;
                st.boards.error = None;
                st.boards.generation += 1;
                st.boards.index = 0;
            }
        }
    }
}

/// The editor closed after `E`. jui checks the text before it writes the file.
fn config_edited(st: &mut State, original: String, result: Result<String, Error>, out: &mut Vec<Action>) {
    let text = match result {
        Ok(t) => t,
        Err(e) => {
            st.config_edit = false;
            report(st, "Edit the config file", &e);
            return;
        }
    };
    if text == original {
        st.config_edit = false;
        st.set_status(MsgKind::Info, "The config file did not change.");
        return;
    }
    match cfg::check_text(&text, &st.config_path) {
        cfg::FileCheck::Valid(_) => out.push(Action::SaveConfigText { text }),
        cfg::FileCheck::Invalid(message) => st.popup = Some(Popup::ConfigInvalid(ConfigInvalid { original, text, message })),
    }
}

/// The config file text is not valid: `e` edits it again, `q` discards the changes.
fn config_invalid_key(st: &mut State, p: ConfigInvalid, k: KeyEvent, out: &mut Vec<Action>) {
    match k.code {
        KeyCode::Char('e') | KeyCode::Enter => out.push(Action::EditConfig { original: Some(p.original), text: Some(p.text) }),
        KeyCode::Char('q') | KeyCode::Esc => {
            st.config_edit = false;
            st.set_status(MsgKind::Info, "The config file did not change.");
        }
        _ => st.popup = Some(Popup::ConfigInvalid(p)),
    }
}

fn config_saved(st: &mut State, result: Result<Settings, Error>, out: &mut Vec<Action>) {
    st.config_edit = false;
    match result {
        Ok(s) => {
            let path = s.path.display().to_string();
            let warning = s.warnings.first().cloned();
            use_settings(st, s, out);
            match warning {
                Some(w) => st.set_status(MsgKind::Warning, format!("Saved {path}. warning: {w}")),
                None => st.set_status(MsgKind::Info, format!("Saved {path}")),
            }
        }
        Err(e) => report(st, "Save the config file", &e),
    }
}
