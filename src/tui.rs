//! The terminal UI. See ticket 04 "jui architecture" (async model) and ticket 06
//! "jui TUI layout and navigation".
//!
//! The event-driven async pattern:
//! - One task reads the terminal events and a tick timer, and sends them on an `mpsc` channel.
//! - Core calls run in `tokio::spawn` tasks. They send their results on the same channel.
//! - The main loop waits only for the channel. It calls the pure `update` function,
//!   draws the state, and does the actions that `update` returns.
//! - A `CancellationToken` for each list cancels the old requests when a new query starts.

pub mod keys;
pub mod state;
pub mod update;
pub mod view;

use std::collections::HashMap;
use std::io::stdout;
use std::time::Duration;

use crossterm::event::{Event as CEvent, EventStream};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::core::settings::{self, Settings};
use crate::core::{Error, Jira, Result};
use state::{PAGE_SIZE, PREVIEW_DELAY_MS, State, TICK_MS};
use state::CommentStart as update_start;
use update::{Action, Event};

/// Start the TUI and wait until the user quits.
pub async fn run(s: Settings) -> Result<()> {
    let jira = Jira::new(&s, std::env::var(settings::ENV_TOKEN).ok());
    let (w, h) = crossterm::terminal::size().unwrap_or((120, 30));
    let mut st = State::new(&s, w, h);
    let first = match &jira {
        Ok(_) => update::init(&mut st),
        Err(e) => {
            // The TUI does not start without a token. It shows the error, and `q` quits.
            update::fatal(&mut st, e);
            Vec::new()
        }
    };

    let terminal = ratatui::try_init().map_err(|e| Error::input(format!("Cannot start the terminal mode: {e}")))?;
    let (tx, rx) = unbounded_channel();
    let mut app = App {
        terminal,
        jira: jira.ok(),
        tx: tx.clone(),
        root: CancellationToken::new(),
        lists: HashMap::new(),
        events: None,
        editor: crate::editor::editor_from_env(&s.editor_command),
        user_search: None,
        board_load: None,
        board_search: None,
        sprint_load: None,
        epic_search: None,
        settings_env: s.env.clone(),
    };
    app.start_events();
    let result = app.run_loop(&mut st, rx, first).await;
    app.root.cancel();
    app.stop_events().await;
    ratatui::restore();
    result
}

struct App {
    terminal: DefaultTerminal,
    jira: Option<Jira>,
    tx: UnboundedSender<Event>,
    /// Cancels all requests when the TUI stops.
    root: CancellationToken,
    /// The token of the current query of each list.
    lists: HashMap<usize, CancellationToken>,
    /// The event task and its stop token.
    events: Option<(CancellationToken, JoinHandle<()>)>,
    /// The editor command (the Setting, then `$VISUAL`, then `$EDITOR`, then `vi`).
    editor: String,
    /// The token of the running user search of the picker.
    user_search: Option<CancellationToken>,
    /// The token of the running Board request (the Boards tab).
    board_load: Option<CancellationToken>,
    /// The token of the running Board search of the Board picker.
    board_search: Option<CancellationToken>,
    /// The token of the running request of the Sprint tab.
    sprint_load: Option<CancellationToken>,
    /// The token of the running search of the Epic picker.
    epic_search: Option<CancellationToken>,
    /// The environment of the Settings (`JUI_CONFIG`, `JUI_BASE_URL`), to read the file again.
    settings_env: settings::Env,
}

/// The path of the config file for an environment.
fn settings_path(env: &settings::Env) -> std::path::PathBuf {
    settings::config_path(env)
}

impl App {
    async fn run_loop(&mut self, st: &mut State, mut rx: UnboundedReceiver<Event>, first: Vec<Action>) -> Result<()> {
        let mut actions = first;
        loop {
            for a in std::mem::take(&mut actions) {
                match a {
                    // The editor needs the terminal. It runs here, and its result is the next event.
                    Action::OpenEditor { text, label } => {
                        let prefix = crate::editor::template_prefix(&label);
                        let result = self.edit_text(&self.editor.clone(), &text, &prefix, ".md").await;
                        actions.extend(update::update(st, Event::Edited { result }));
                    }
                    Action::EditConfig { original, text } => {
                        let original = match original {
                            Some(o) => Ok(o),
                            None => settings::edit_text(&st.config_path),
                        };
                        let ev = match original {
                            Ok(original) => {
                                let text = text.unwrap_or_else(|| original.clone());
                                let result = self.edit_text(&self.editor.clone(), &text, "jui-config-", ".toml").await;
                                Event::ConfigEdited { original, result }
                            }
                            Err(e) => Event::ConfigEdited { original: String::new(), result: Err(e) },
                        };
                        actions.extend(update::update(st, ev));
                    }
                    a => self.perform(a),
                }
            }
            // The editor can give more actions (for example, open the editor again).
            if !actions.is_empty() {
                continue;
            }
            if st.quit {
                return Ok(());
            }
            self.terminal
                .draw(|f| view::view(f, st))
                .map_err(|e| Error::input(format!("Cannot draw the screen: {e}")))?;
            let Some(ev) = rx.recv().await else { return Ok(()) };
            actions = update::update(st, ev);
            // Use all events that are ready before the next draw.
            while let Ok(ev) = rx.try_recv() {
                actions.extend(update::update(st, ev));
            }
        }
    }

    /// Do one action. Results come back as events on the channel.
    fn perform(&mut self, a: Action) {
        match a {
            Action::FetchPage { tab, jql, start_at, generation } => {
                let Some(jira) = self.jira.clone() else { return };
                if start_at == 0 {
                    // A new query cancels the requests of the old query of this list.
                    if let Some(old) = self.lists.remove(&tab) {
                        old.cancel();
                    }
                }
                let token = self.lists.entry(tab).or_insert_with(|| self.root.child_token()).clone();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = token.cancelled() => {}
                        result = jira.search_rows(&jql, start_at, PAGE_SIZE) => {
                            let _ = tx.send(Event::Page { tab, generation, start_at, result });
                        }
                    }
                });
            }
            Action::FetchTicket { key } => {
                let Some(jira) = self.jira.clone() else { return };
                let token = self.root.child_token();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => Err(Error::Cancelled),
                        r = jira.ticket(&key) => r,
                    };
                    let _ = tx.send(Event::Ticket { key, result: Box::new(result) });
                });
            }
            Action::SchedulePreview { key } => {
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = token.cancelled() => {}
                        _ = tokio::time::sleep(Duration::from_millis(PREVIEW_DELAY_MS)) => {
                            let _ = tx.send(Event::PreviewDue { key });
                        }
                    }
                });
            }
            Action::OpenBrowser { url } => {
                if let Err(e) = open::that_detached(&url) {
                    let _ = self.tx.send(Event::Failed {
                        context: "Open the browser".into(),
                        error: Error::input_hint(format!("Cannot open the browser: {e}"), format!("Open {url}")),
                    });
                }
            }
            Action::FetchTransitions { key } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.transitions(&key) => r,
                    };
                    let _ = tx.send(Event::Transitions { key, result });
                });
            }
            Action::DoTransition { key, transition, fields } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A write is not cancelled: the request can already be in Jira.
                tokio::spawn(async move {
                    let result = jira.transition(&key, &transition, &fields, None).await;
                    let _ = tx.send(Event::Moved { key, to: transition.to, result });
                });
            }
            Action::CheckMatch { tab, key, jql } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.search(&jql, 0, 1) => r.map(|p| p.total > 0),
                    };
                    let _ = tx.send(Event::Matched { tab, key, result });
                });
            }
            Action::PrepareCreate { kind, project, issue_type } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = tokio::try_join!(jira.create_meta(&project, &issue_type), jira.myself());
                    let _ = tx.send(Event::CreateReady { kind, result: Box::new(result) });
                });
            }
            Action::PrepareEdit { kind, key } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = tokio::try_join!(jira.edit_data(&key), jira.myself());
                    let _ = tx.send(Event::EditReady { kind, result: Box::new(result) });
                });
            }
            Action::OpenEditor { .. } => unreachable!("run_loop opens the editor"),
            Action::Recheck { key } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.edit_data(&key).await;
                    let _ = tx.send(Event::Rechecked { key, result: Box::new(result) });
                });
            }
            Action::SendCreate { body } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A write is not cancelled: the request can already be in Jira.
                tokio::spawn(async move {
                    let result = jira.create(&body).await;
                    let _ = tx.send(Event::Created { result });
                });
            }
            Action::SendEdit { key, body, changed } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.edit(&key, &body).await;
                    let _ = tx.send(Event::Saved { key, changed, result });
                });
            }
            Action::WriteDraft { label, text, reason, command } => {
                let result = crate::core::template::write_draft(&crate::core::template::draft_dir(), &label, &text);
                let _ = self.tx.send(Event::DraftWritten { reason, command, result });
            }
            Action::PrepareComment { key, start } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let id = match &start {
                        update_start::Edit { id, .. } => Some(id.clone()),
                        update_start::Add { .. } => None,
                    };
                    let result = jira.comment_context(&key, id.as_deref()).await;
                    let _ = tx.send(Event::CommentReady { key, start, result: Box::new(result) });
                });
            }
            Action::SendComment { key, id, wiki } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A write is not cancelled: the request can already be in Jira.
                tokio::spawn(async move {
                    let result = match &id {
                        Some(id) => jira.edit_comment(&key, id, &wiki).await,
                        None => jira.add_comment(&key, &wiki).await,
                    };
                    let _ = tx.send(Event::CommentSent { key, id, result: Box::new(result) });
                });
            }
            Action::RecheckComment { key, id } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.comment(&key, &id).await;
                    let _ = tx.send(Event::CommentRechecked { key, id, result: Box::new(result) });
                });
            }
            Action::DeleteComment { key, id } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.delete_comment(&key, &id).await;
                    let _ = tx.send(Event::CommentDeleted { key, id, result });
                });
            }
            Action::FetchLinkTypes => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.link_types() => r,
                    };
                    let _ = tx.send(Event::LinkTypes { result });
                });
            }
            Action::AddLink { plan } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A write is not cancelled: the request can already be in Jira.
                tokio::spawn(async move {
                    let result = jira.add_link(&plan).await;
                    let _ = tx.send(Event::LinkAdded { plan, result });
                });
            }
            Action::DeleteLink { key, id } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.delete_link(&id).await;
                    let _ = tx.send(Event::LinkDeleted { key, id, result });
                });
            }
            Action::PrepareWorklog { key, start } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.worklog_context(&key).await;
                    let _ = tx.send(Event::WorklogReady { key, start, result: Box::new(result) });
                });
            }
            Action::SendWorklog { key, plan } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A write is not cancelled: the request can already be in Jira.
                tokio::spawn(async move {
                    let id = plan.id.clone();
                    let result = match &id {
                        Some(id) => jira.edit_worklog(&key, id, &plan).await,
                        None => jira.add_worklog(&key, &plan).await,
                    };
                    let _ = tx.send(Event::WorklogSent { key, id, result: Box::new(result) });
                });
            }
            Action::RecheckWorklog { key, id } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.worklog(&key, &id).await;
                    let _ = tx.send(Event::WorklogRechecked { key, id, result: Box::new(result) });
                });
            }
            Action::DeleteWorklog { key, id } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.delete_worklog(&key, &id, &crate::core::worklog::Adjust::Auto).await;
                    let _ = tx.send(Event::WorklogDeleted { key, id, result });
                });
            }
            Action::Upload { key, path } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A write is not cancelled: the request can already be in Jira.
                tokio::spawn(async move {
                    let result = upload_file(&jira, &key, &path).await;
                    let _ = tx.send(Event::Uploaded { key, result });
                });
            }
            Action::DeleteAttachment { key, id } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = jira.delete_attachment(&id).await;
                    let _ = tx.send(Event::AttachmentDeleted { key, id, result });
                });
            }
            Action::ScheduleUserSearch { seq } => {
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = token.cancelled() => {}
                        _ = tokio::time::sleep(Duration::from_millis(state::USER_SEARCH_DELAY_MS)) => {
                            let _ = tx.send(Event::UserSearchDue { seq });
                        }
                    }
                });
            }
            Action::SearchUsers { key, text, seq } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A new search replaces the old one: cancel the request of the old text.
                if let Some(old) = self.user_search.take() {
                    old.cancel();
                }
                let token = self.root.child_token();
                self.user_search = Some(token.clone());
                tokio::spawn(async move {
                    let scope = crate::core::jira::AssignScope::Ticket(key.clone());
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.assignable_users(&scope, &text, state::USER_SEARCH_MAX) => r,
                    };
                    let _ = tx.send(Event::Users { key, seq, result });
                });
            }
            Action::Assign { key, who } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A write is not cancelled: the request can already be in Jira.
                tokio::spawn(async move {
                    let result = jira.assign(&key, &who).await;
                    let _ = tx.send(Event::Assigned { key, who, result });
                });
            }
            Action::Watch { key, on } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = if on { jira.watch(&key, None).await } else { jira.unwatch(&key, None).await };
                    let _ = tx.send(Event::Watched { key, on, result });
                });
            }
            Action::Copy { text } => {
                // The OSC 52 sequence goes to the terminal; ratatui does not show it.
                let result = crate::clipboard::copy(&text, &mut stdout()).map(|via| match via {
                    crate::clipboard::Via::Command(c) => c,
                    crate::clipboard::Via::Osc52 => "terminal",
                });
                let _ = self.tx.send(Event::Copied { text, result });
            }
            Action::FetchFavouriteFilters => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.favourite_filters() => r,
                    };
                    let _ = tx.send(Event::FavouriteFilters { result });
                });
            }
            Action::FetchFilter { id } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.filter(&id) => r,
                    };
                    let _ = tx.send(Event::FilterLoaded { id, result });
                });
            }
            Action::FetchBoard { id, generation, only_mine } => {
                let Some(jira) = self.jira.clone() else { return };
                // A new Board replaces the old one: cancel the request of the old Board.
                if let Some(old) = self.board_load.take() {
                    old.cancel();
                }
                let token = self.root.child_token();
                self.board_load = Some(token.clone());
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.board_view(id, crate::core::board::BOARD_LIMIT, only_mine) => r,
                    };
                    let _ = tx.send(Event::BoardLoaded { id, generation, result: Box::new(result) });
                });
            }
            Action::FetchBoardSuggestions { project } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.board_suggestions(&project) => r,
                    };
                    let _ = tx.send(Event::BoardSuggestions { result });
                });
            }
            Action::ScheduleBoardSearch { seq } => {
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = token.cancelled() => {}
                        _ = tokio::time::sleep(Duration::from_millis(state::USER_SEARCH_DELAY_MS)) => {
                            let _ = tx.send(Event::BoardSearchDue { seq });
                        }
                    }
                });
            }
            Action::SearchBoards { text, seq } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                if let Some(old) = self.board_search.take() {
                    old.cancel();
                }
                let token = self.root.child_token();
                self.board_search = Some(token.clone());
                tokio::spawn(async move {
                    let q = crate::core::jira::BoardQuery { name: Some(text), ..Default::default() };
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.boards(&q, 0, crate::core::board::LIST_PAGE) => r.map(|p| p.items),
                    };
                    let _ = tx.send(Event::BoardsFound { seq, result });
                });
            }
            Action::SaveBoards { path, list } => {
                // A small local file: write it now, and send the result as an event.
                let result = settings::write_boards(&path, &list);
                let _ = self.tx.send(Event::BoardsSaved { list, result });
            }
            Action::FetchSprintBoard { id, generation, sprints } => {
                let Some(jira) = self.jira.clone() else { return };
                // A new Board replaces the old one: cancel the request of the old Board.
                if let Some(old) = self.sprint_load.take() {
                    old.cancel();
                }
                let token = self.root.child_token();
                self.sprint_load = Some(token.clone());
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.sprint_board(id, sprints) => r,
                    };
                    let _ = tx.send(Event::SprintBoardLoaded { id, generation, result: Box::new(result) });
                });
            }
            Action::FetchMyEpicCounts { generation } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.my_epic_counts() => r,
                    };
                    let _ = tx.send(Event::MyEpicCounts { generation, result });
                });
            }
            Action::FetchOpenSprints { board_id, key } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.sprints(board_id, "active,future") => r,
                    };
                    let _ = tx.send(Event::OpenSprints { board_id, key, result });
                });
            }
            Action::MoveSprint { key, target } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A write is not cancelled: the request can already be in Jira.
                tokio::spawn(async move {
                    let keys = [key.clone()];
                    let result = match &target {
                        crate::core::agile::Target::Sprint(s) => jira.move_to_sprint(s.id, &keys).await,
                        crate::core::agile::Target::Backlog => jira.move_to_backlog(&keys).await,
                    };
                    let _ = tx.send(Event::SprintMoved { key, target, result });
                });
            }
            Action::ScheduleEpicSearch { seq } => {
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = token.cancelled() => {}
                        _ = tokio::time::sleep(Duration::from_millis(state::USER_SEARCH_DELAY_MS)) => {
                            let _ = tx.send(Event::EpicSearchDue { seq });
                        }
                    }
                });
            }
            Action::SearchEpics { key, project, text, seq } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                // A new search replaces the old one. The list with no text (seq 0) is not cancelled.
                let token = self.root.child_token();
                if seq > 0 {
                    if let Some(old) = self.epic_search.take() {
                        old.cancel();
                    }
                    self.epic_search = Some(token.clone());
                }
                tokio::spawn(async move {
                    let jql = crate::core::agile::epic_search_jql(&project, &text);
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.search_epics(&jql, 0, crate::core::agile::EPIC_PICK_MAX) => r.map(|p| p.items),
                    };
                    let _ = tx.send(Event::Epics { key, seq, result });
                });
            }
            Action::SetEpic { key, epic, old } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let keys = [key.clone()];
                    let result = match &epic {
                        Some(e) => jira.set_epic(&e.key, &keys).await,
                        None => jira.unset_epic(&keys).await,
                    };
                    let _ = tx.send(Event::EpicSet { key, epic, old, result });
                });
            }
            Action::FetchEpicChildren { key } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let jql = crate::core::agile::epic_jql(&key);
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.search_all(&jql, Some(state::EPIC_CHILDREN_MAX)) => r,
                    };
                    let _ = tx.send(Event::EpicChildren { key, result });
                });
            }
            Action::CheckProject { input } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.project_key(&input) => r,
                    };
                    let _ = tx.send(Event::ProjectChecked { input, result });
                });
            }
            Action::FetchTicketTypes { project } => {
                let Some(jira) = self.jira.clone() else { return };
                let tx = self.tx.clone();
                let token = self.root.child_token();
                tokio::spawn(async move {
                    let result = tokio::select! {
                        _ = token.cancelled() => return,
                        r = jira.issue_types(&project) => r,
                    };
                    // Sub-task types are not permitted for a new Ticket.
                    let result = result.map(|ts| ts.into_iter().filter(|t| !t.subtask).map(|t| t.name).collect());
                    let _ = tx.send(Event::TicketTypes { project, result });
                });
            }
            Action::SaveSetting { key, value } => {
                // A small local file: write it now, read it again, and send the result as an event.
                let path = settings_path(&self.settings_env);
                let result = match &value {
                    Some(v) => settings::set(&path, &key, v),
                    None => settings::unset(&path, &key),
                }
                .and_then(|()| Settings::load(&self.settings_env));
                let _ = self.tx.send(Event::SettingSaved { key, result });
            }
            Action::SaveConfigText { text } => {
                let path = settings_path(&self.settings_env);
                let result = settings::save_text(&path, &text).and_then(|()| Settings::load(&self.settings_env));
                let _ = self.tx.send(Event::ConfigSaved { result });
            }
            Action::ReloadSettings => {
                let result = Settings::load(&self.settings_env);
                let _ = self.tx.send(Event::SettingsReloaded { result });
            }
            Action::UseConnection { base_url, timeout_secs, editor_command } => {
                self.editor = crate::editor::editor_from_env(&editor_command);
                let s = Settings { base_url, timeout_secs, ..Settings::default() };
                match Jira::new(&s, std::env::var(settings::ENV_TOKEN).ok()) {
                    Ok(j) => {
                        // The requests of the old client stop; their results are not useful.
                        for (_, t) in self.lists.drain() {
                            t.cancel();
                        }
                        self.jira = Some(j);
                    }
                    Err(e) => {
                        let _ = self.tx.send(Event::Failed { context: "Use the new Settings".into(), error: e });
                    }
                }
            }
            Action::EditConfig { .. } => unreachable!("run_loop opens the editor"),
            Action::Quit => self.root.cancel(),
        }
    }

    /// Start the task that reads terminal events and the tick timer.
    fn start_events(&mut self) {
        let stop = CancellationToken::new();
        let tx = self.tx.clone();
        let token = stop.clone();
        let handle = tokio::spawn(async move {
            let mut events = EventStream::new();
            let mut tick = tokio::time::interval(Duration::from_millis(TICK_MS));
            loop {
                let ev = tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tick.tick() => Event::Tick,
                    ev = events.next() => match ev {
                        Some(Ok(CEvent::Key(k))) => Event::Key(k),
                        Some(Ok(CEvent::Resize(w, h))) => Event::Resize(w, h),
                        Some(Ok(_)) => continue,
                        Some(Err(_)) | None => break,
                    },
                };
                if tx.send(ev).is_err() {
                    break;
                }
            }
        });
        self.events = Some((stop, handle));
    }

    async fn stop_events(&mut self) {
        if let Some((stop, handle)) = self.events.take() {
            stop.cancel();
            let _ = handle.await;
        }
    }

    /// Stop the terminal mode, start the editor on `text`, and start the terminal mode again.
    /// The event task stops during the edit, so that it does not read the keys of the editor.
    async fn edit_text(&mut self, command: &str, text: &str, prefix: &str, suffix: &str) -> Result<String> {
        self.stop_events().await;
        let result = crate::editor::edit_text_with(command, text, prefix, suffix, leave_terminal, enter_terminal);
        let _ = self.terminal.clear();
        self.start_events();
        result
    }
}

/// Clean the path of the `u` popup, check the file, and upload it.
async fn upload_file(jira: &Jira, key: &str, input: &str) -> Result<Vec<crate::core::model::Attachment>> {
    use crate::core::attachment as at;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let path = at::clean_path(input, home.as_deref());
    let meta = jira.attachment_meta().await?;
    let file = tokio::task::spawn_blocking(move || at::read_upload(&path, meta.limit))
        .await
        .map_err(|e| Error::input(format!("Cannot read the file: {e}")))??;
    jira.upload(key, &file).await
}

fn leave_terminal() -> std::io::Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)
}

fn enter_terminal() -> std::io::Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)
}
