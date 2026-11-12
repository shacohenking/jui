//! Draw the state. The view writes only display data back to the state
//! (the table scroll position, the render caches, and the body height).

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table, Wrap};
use unicode_width::UnicodeWidthStr;

use super::keys::{KEYS, Where};
use super::state::*;
use crate::core::format::{format_date, format_when, human_duration, human_size};
use crate::core::markup;
use crate::core::model::{StatusCategory, Text, Ticket, TicketSummary};

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

pub fn view(f: &mut Frame, st: &mut State) {
    let area = f.area();
    st.width = area.width;
    st.height = area.height;
    let [tabs, body, status] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);
    st.body_height = body.height.saturating_sub(2);
    draw_tabs(f, st, tabs);
    if st.current().zoom.is_empty() {
        draw_main(f, st, body);
    } else {
        draw_zoom(f, st, body);
    }
    draw_status(f, st, status);
    match st.popup.clone() {
        Some(Popup::Help { scroll }) => draw_help(f, area, scroll),
        Some(Popup::Errors { scroll }) => draw_errors(f, st, area, scroll),
        Some(Popup::Fatal { message, hint }) => draw_fatal(f, area, &message, hint.as_deref()),
        Some(Popup::Transition(p)) => draw_transition(f, st, area, &p),
        Some(Popup::Fields(p)) => draw_fields(f, area, &p),
        Some(Popup::Create(p)) => draw_create(f, st, area, &p),
        Some(Popup::Diff(p)) => draw_diff(f, area, &p),
        Some(Popup::Conflict { key, message }) => draw_conflict(f, area, &key, &message),
        Some(Popup::Comment(p)) => draw_comment(f, st, area, &p),
        Some(Popup::User(p)) => draw_user(f, st, area, &p),
        Some(Popup::Filters(p)) => draw_filters(f, st, area, &p),
        Some(Popup::ConfirmDelete { what, key, id, preview, sending }) => draw_delete(f, area, what, &key, &id, &preview, sending),
        Some(Popup::Link(p)) => draw_link(f, st, area, &p),
        Some(Popup::Worklog(p)) => draw_worklog(f, st, area, &p),
        Some(Popup::Upload(p)) => draw_upload(f, st, area, &p),
        Some(Popup::Boards(p)) => draw_board_picker(f, st, area, &p),
        Some(Popup::ConfirmRemoveBoard { name, .. }) => draw_remove_board(f, area, &name),
        Some(Popup::Sprint(p)) => draw_sprint_popup(f, st, area, &p),
        Some(Popup::Epic(p)) => draw_epic_popup(f, st, area, &p),
        Some(Popup::Setting(p)) => draw_setting_popup(f, st, area, &p),
        Some(Popup::ConfigInvalid(p)) => draw_config_invalid(f, area, &p),
        None => {}
    }
}

// ------------------------------------------------------------------ frame

fn draw_tabs(f: &mut Frame, st: &State, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    for (i, t) in TAB_TITLES.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::new().fg(Color::DarkGray)));
        }
        let text = format!("{} {t}", i + 1);
        if i == st.tab {
            spans.push(Span::styled(text, Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)));
        } else {
            spans.push(Span::raw(text));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_status(f: &mut Frame, st: &State, area: Rect) {
    let hint = if st.current().zoom.is_empty() && st.tab == SEARCH && st.search.focus {
        "Type JQL · Enter run · ↑↓ history · Ctrl-F filters · Esc go to the list"
    } else if st.current().zoom.is_empty() && st.tab == BOARDS {
        "h/l column · j/k card · [ ] Board · m mine · + Boards · x remove · Enter zoom · t move · ? help"
    } else if st.current().zoom.is_empty() && st.tab == SETTINGS {
        match st.selected_setting() {
            Some(SettingsRow::Board(_)) => "j/k move · K/J order · d remove · Enter Board picker · E edit file · ? help",
            Some(SettingsRow::AddBoards) => "j/k move · Enter Board picker · E edit file · r read file · ? help",
            _ => "j/k move · Enter edit · d default · E edit file · r read file · ? help",
        }
    } else if st.current().zoom.is_empty() && st.tab == SPRINT {
        "[ ] Sprint or backlog · m mine · j/k move · Enter zoom · t move · a assign · r refresh · ? help"
    } else if st.current().zoom.is_empty() && st.tab == SEARCH {
        "/ edit JQL · Ctrl-F filters · j/k move · Enter zoom · t move · c comment · a assign · ? help"
    } else if st.current().zoom.is_empty() && st.tab == EPICS {
        "j/k move · Enter zoom (Tickets of the Epic) · t move · c comment · r refresh · ? help"
    } else if st.current().zoom.is_empty() {
        "j/k move · Enter zoom · t move · c comment · a assign · e edit · n new · w watch · y copy · ? help"
    } else if st.zoom_is_epic() && matches!(st.selected_item().map(|(_, i)| i), None | Some(ZoomItem::Ticket(_))) {
        "j/k select · Enter open · n new Ticket in this Epic · Esc back · t move · c comment · ? help"
    } else {
        match st.selected_item().map(|(_, i)| i) {
            Some(ZoomItem::Comment(_)) => "j/k select · e edit comment · W wiki · d delete · c comment · Esc back · ? help",
            Some(ZoomItem::Worklog(_)) => "j/k select · e edit Worklog · d delete · L add Worklog · Esc back · ? help",
            Some(ZoomItem::Link { .. }) => "j/k select · Enter open · d delete Link · l add Link · Esc back · ? help",
            Some(ZoomItem::Attachment(_)) => "j/k select · Enter open in browser · d delete · u upload · Esc back · ? help",
            _ => "j/k select · Enter open · Esc back · t move · c comment · l Link · L Worklog · u upload · ? help",
        }
    };
    let mut spans = Vec::new();
    if st.busy() {
        spans.push(Span::styled(format!("{} ", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    }
    match &st.status {
        Some(m) => {
            let style = match m.kind {
                MsgKind::Info => Style::new().fg(Color::Green),
                MsgKind::Warning => Style::new().fg(Color::Yellow),
                MsgKind::Error => Style::new().fg(Color::Red),
            };
            spans.push(Span::styled(m.text.clone(), style));
        }
        None => spans.push(Span::styled(hint, Style::new().fg(Color::DarkGray))),
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ------------------------------------------------------------------ main view

fn draw_main(f: &mut Frame, st: &mut State, area: Rect) {
    if st.tab == BOARDS {
        draw_boards(f, st, area);
        return;
    }
    if st.tab == SETTINGS {
        draw_settings(f, st, area);
        return;
    }
    let area = if st.tab == SPRINT {
        match draw_sprint_head(f, st, area) {
            Some(a) => a,
            None => return,
        }
    } else {
        area
    };
    if st.current().list.is_none() {
        let msg = later_tab_text();
        let p = Paragraph::new(msg).block(Block::bordered().title(TAB_TITLES[st.tab])).wrap(Wrap { trim: false });
        f.render_widget(p, area);
        return;
    }
    let area = if st.tab == SEARCH { draw_search_box(f, st, area) } else { area };
    if st.tab == SEARCH && st.tabs[SEARCH].list.as_ref().is_some_and(|l| l.jql.is_empty()) {
        draw_search_help(f, st, area);
        return;
    }
    if st.preview_visible() {
        let [left, right] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
        draw_list(f, st, left);
        draw_preview(f, st, right);
    } else {
        draw_list(f, st, area);
    }
}

/// The JQL input at the top of the Search tab. Returns the area below it.
fn draw_search_box(f: &mut Frame, st: &State, area: Rect) -> Rect {
    let s = &st.search;
    let error_h = if s.error.is_some() { 1 } else { 0 };
    let [input, rest] = Layout::vertical([Constraint::Length(3 + error_h), Constraint::Min(1)]).areas(area);
    let mut title = " JQL ".to_string();
    if let Some(name) = &s.filter {
        title = format!(" JQL · filter: {name} ");
    }
    let hint = if s.focus { " Enter run · ↑↓ history · Ctrl-F filters · Esc list " } else { " / edit · Ctrl-F filters " };
    let border = if s.focus { Style::new().fg(Color::Cyan) } else { Style::new().fg(Color::DarkGray) };
    let block = Block::bordered().title(title).title_bottom(Line::from(hint).right_aligned()).border_style(border);
    let inner_w = input.width.saturating_sub(2) as usize;
    // Scroll the text to the left when the cursor is past the right edge.
    let before: String = s.input.chars().take(s.cursor).collect();
    let skip = before.width().saturating_sub(inner_w.saturating_sub(1));
    let mut shown = String::new();
    let mut w = 0;
    for c in s.input.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w >= skip {
            shown.push(c);
        }
        w += cw;
    }
    let mut lines = vec![Line::raw(shown)];
    if let Some(e) = &s.error {
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    f.render_widget(Paragraph::new(lines).block(block), input);
    if s.focus && st.popup.is_none() {
        let x = input.x + 1 + (before.width() - skip) as u16;
        f.set_cursor_position((x.min(input.x + input.width.saturating_sub(2)), input.y + 1));
    }
    rest
}

/// The Search tab before the first query.
fn draw_search_help(f: &mut Frame, st: &State, area: Rect) {
    let mut lines = vec![
        Line::raw(""),
        Line::raw("  Push / to type a JQL query, then push Enter. Examples:"),
        Line::raw(""),
        Line::styled("    project = BCE AND status = \"In Progress\"", Style::new().fg(Color::Cyan)),
        Line::styled("    text ~ \"login\" ORDER BY updated DESC", Style::new().fg(Color::Cyan)),
        Line::styled("    reporter = currentUser() AND created >= -7d", Style::new().fg(Color::Cyan)),
        Line::raw(""),
        Line::raw("  Ctrl-F runs a favourite filter, or any filter by its ID."),
    ];
    if !st.search.history.is_empty() {
        lines.push(Line::raw("  ↑ in the input shows the last queries."));
    }
    f.render_widget(Paragraph::new(lines).block(Block::bordered().title("Search")), area);
}

// ------------------------------------------------------------------ Boards tab

/// The Boards tab: a title line with My Boards, the columns, and the preview of the card.
/// The Board uses all of the width; the preview is below the columns when the screen is high.
fn draw_boards(f: &mut Frame, st: &mut State, area: Rect) {
    let [head, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    draw_board_head(f, st, head);
    let b = &st.boards;
    let message: Option<Vec<Line<'static>>> = if st.my_boards.is_empty() {
        Some(vec![
            Line::raw(""),
            Line::raw("  My Boards is empty. Push + to select Boards."),
            Line::raw(""),
            dim("  The Board picker suggests your recent Boards and the Boards of the default project."),
            dim("  Or use `jui board add <id>` in the shell."),
        ])
    } else if let (Some((id, e)), None) = (&b.error, &b.view) {
        let gone = b.missing.contains_key(id);
        let mut l = vec![Line::raw(""), Line::styled(format!("  {e}"), Style::new().fg(Color::Red))];
        if gone {
            l.push(Line::raw("  The Board does not exist, or you cannot see it. Push x to remove it from My Boards."));
        } else {
            l.push(Line::raw("  Push r to try again."));
        }
        Some(l)
    } else if b.view.is_none() {
        Some(vec![Line::raw(""), Line::raw("  Loading the Board…")])
    } else {
        None
    };
    if let Some(lines) = message {
        f.render_widget(Paragraph::new(lines).block(Block::bordered().title("Boards")).wrap(Wrap { trim: false }), body);
        return;
    }
    let v = b.view.as_ref().expect("checked");
    if v.no_active_sprint() {
        let lines = vec![
            Line::raw(""),
            Line::raw(format!("  {} has no active Sprint.", v.board.name)),
            dim("  Push 3 for the Sprint tab: the backlog and the future Sprints."),
        ];
        f.render_widget(Paragraph::new(lines).block(Block::bordered().title("Board")), body);
        return;
    }
    // The Ticket preview gets more of the height than the columns: a column needs only one
    // line for each card, and the preview shows the description.
    let show_preview = body.height >= BOARD_PREVIEW_MIN_HEIGHT;
    let (cols_area, preview_area) = if show_preview {
        let [a, b] = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(body);
        (a, Some(b))
    } else {
        (body, None)
    };
    draw_columns(f, st, cols_area);
    if let Some(p) = preview_area {
        draw_preview(f, st, p);
    }
}

/// Below this height of the Board area, the Boards tab shows only the columns. At 20 lines,
/// the columns get 8 lines (3 cards of 2 lines) and the preview gets 12 lines.
const BOARD_PREVIEW_MIN_HEIGHT: u16 = 20;

/// The line above the Board: `‹ 1/3 BCE board (scrum) › · Sprint 21 · 670 Tickets`.
fn draw_board_head(f: &mut Frame, st: &State, area: Rect) {
    let b = &st.boards;
    let n = st.my_boards.len();
    let mut spans = vec![Span::raw(" ")];
    if n == 0 {
        spans.push(dim("No Boards · + add").spans.remove(0));
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }
    let i = b.index.min(n - 1);
    let r = &st.my_boards[i];
    let name = b.view.as_ref().filter(|v| v.board.id == r.id).map(|v| v.board.name.clone()).or_else(|| r.name.clone());
    let name = name.unwrap_or_else(|| format!("Board {}", r.id));
    spans.push(Span::styled(format!("[ {}/{n} ] ", i + 1), Style::new().fg(Color::DarkGray)));
    spans.push(Span::styled(name, Style::new().add_modifier(Modifier::BOLD)));
    if let Some(v) = b.view.as_ref().filter(|v| v.board.id == r.id && !b.stale) {
        spans.push(Span::styled(format!(" ({})", v.board.kind), Style::new().fg(Color::DarkGray)));
        let sep = || Span::styled(" · ", Style::new().fg(Color::DarkGray));
        if !v.sprints.is_empty() {
            spans.push(sep());
            spans.push(Span::raw(v.sprints.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")));
        }
        if !v.no_active_sprint() {
            spans.push(sep());
            let style = if v.truncated() { Style::new().fg(Color::Yellow) } else { Style::new() };
            let mine = if v.only_mine { " (only mine)" } else { "" };
            spans.push(Span::styled(format!("{} Tickets{mine}", v.count()), style));
        }
        if v.unmapped > 0 {
            spans.push(sep());
            spans.push(dim(format!("{} not in a column", v.unmapped)).spans.remove(0));
        }
    }
    if b.loading.is_some() {
        spans.push(Span::styled("  Loading…", Style::new().fg(Color::Yellow)));
    }
    // The filter shows also while the Board loads, so that the user sees the effect of `m`.
    if b.only_mine && !b.view.as_ref().is_some_and(|v| v.only_mine && v.board.id == r.id && !b.stale) {
        spans.push(Span::styled("  only mine", Style::new().fg(Color::Magenta)));
    }
    if b.missing.contains_key(&r.id) {
        spans.push(Span::styled("  not found", Style::new().fg(Color::Red)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The columns, side by side. Each column scrolls on its own and keeps the selected card visible.
fn draw_columns(f: &mut Frame, st: &mut State, area: Rect) {
    let b = &st.boards;
    let v = b.view.as_ref().expect("draw_columns needs a Board");
    let n = v.columns.len().max(1);
    // At least 18 columns of text for each Board column. Too many columns: show the
    // columns around the selected one.
    let fit = (area.width as usize / 18).clamp(1, n);
    let first = b.col.saturating_sub(fit - 1).min(n - fit);
    let areas = Layout::horizontal((0..fit).map(|_| Constraint::Ratio(1, fit as u32))).split(area);
    let dim_all = b.stale;
    for (slot, ci) in (first..first + fit).enumerate() {
        let Some(col) = v.columns.get(ci) else { break };
        let r = areas[slot];
        let on = ci == b.col;
        let mut title = format!(" {} ({}) ", col.name, col.tickets.len());
        if slot == 0 && first > 0 {
            title = format!("‹{title}");
        }
        if slot + 1 == fit && first + fit < n {
            title.push('›');
        }
        let border = if on { Style::new().fg(Color::Cyan) } else { Style::new().fg(Color::DarkGray) };
        let block = Block::bordered().title(title).border_style(border);
        let inner_h = r.height.saturating_sub(2) as usize;
        let inner_w = r.width.saturating_sub(2) as usize;
        let row = b.rows.get(ci).copied().unwrap_or(0);
        // Each card has 2 lines: the key and the summary.
        let per = 2usize;
        let visible = (inner_h / per).max(1);
        let start = row.saturating_sub(visible - 1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (i, t) in col.tickets.iter().enumerate().skip(start).take(visible) {
            let sel = on && i == row && !dim_all;
            let mut head = vec![Span::styled(t.key.clone(), Style::new().fg(Color::Cyan))];
            if let Some(u) = &t.assignee {
                head.push(Span::styled(format!(" @{}", u.name), Style::new().fg(Color::DarkGray)));
            }
            let summary: String = cut(&t.summary, inner_w.saturating_sub(1));
            let mut l1 = Line::from(head);
            let mut l2 = Line::raw(format!(" {summary}"));
            if sel {
                l1 = l1.style(Style::new().add_modifier(Modifier::REVERSED));
                l2 = l2.style(Style::new().add_modifier(Modifier::REVERSED));
            }
            if dim_all {
                l1 = l1.style(Style::new().add_modifier(Modifier::DIM));
                l2 = l2.style(Style::new().add_modifier(Modifier::DIM));
            }
            lines.push(l1);
            lines.push(l2);
        }
        if col.tickets.is_empty() {
            lines.push(dim(" (empty)"));
        }
        f.render_widget(Paragraph::new(lines).block(block), r);
    }
}

/// Cut a text to a width, with `…` at the end when it is longer.
fn cut(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) + 1 > w {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn draw_board_picker(f: &mut Frame, st: &State, area: Rect, p: &BoardPopup) {
    let mut lines = vec![Line::from(vec![
        Span::styled("Board: ", Style::new().fg(Color::DarkGray)),
        Span::raw(p.input.clone()),
        Span::styled("▏", Style::new().fg(Color::DarkGray)),
    ])];
    lines.push(Line::raw(""));
    let rows = p.rows();
    let w = 76u16.min(area.width.saturating_sub(4));
    let max_rows = (area.height.saturating_sub(12) as usize).clamp(3, 15);
    let start = p.selected.saturating_sub(max_rows - 1);
    for (i, r) in rows.iter().enumerate().skip(start).take(max_rows) {
        let on = i == p.selected;
        let mark = if p.is_marked(r.board.id) { "[x] " } else { "[ ] " };
        let mut spans = vec![
            Span::raw(if on { "▶ " } else { "  " }),
            Span::styled(mark, Style::new().fg(Color::Yellow)),
            Span::styled(format!("{:<6}", r.board.id), Style::new().fg(Color::Cyan)),
            Span::raw(r.board.name.clone()),
        ];
        if !r.board.kind.is_empty() {
            spans.push(Span::styled(format!("  {}", r.board.kind), Style::new().fg(Color::DarkGray)));
        }
        if !r.tag.is_empty() {
            spans.push(Span::styled(format!("  ({})", r.tag), Style::new().fg(Color::DarkGray)));
        }
        let mut line = Line::from(spans);
        if on {
            line = line.style(Style::new().add_modifier(Modifier::REVERSED));
        }
        lines.push(line);
    }
    if rows.len() > start + max_rows {
        lines.push(dim(format!("  … {} more. Type to search.", rows.len() - start - max_rows)));
    }
    let spin = SPINNER[st.spinner % SPINNER.len()];
    if p.searching {
        lines.push(dim(format!("{spin} Searching…")));
    } else if p.suggestions.is_none() && !p.searches() {
        lines.push(dim(format!("{spin} Getting the suggestions…")));
    } else if rows.is_empty() && p.searches() {
        lines.push(dim("No Board matches."));
    } else if rows.is_empty() && !p.input.trim().is_empty() {
        lines.push(dim(format!("Type {USER_SEARCH_MIN} characters or more to search all Boards.")));
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    let n = p.marked.len();
    lines.push(Line::raw(format!("{n} Board{} marked{}", if n == 1 { "" } else { "s" }, if p.changed() { " (changed)" } else { "" })));
    if p.saving {
        lines.push(Line::styled("Saving…", Style::new().fg(Color::Yellow)));
    } else {
        lines.push(dim("Type to search · ↑↓ select · Space mark · Enter save · Esc cancel"));
    }
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(" My Boards ").padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_remove_board(f: &mut Frame, area: Rect, name: &str) {
    let lines = vec![
        Line::raw(name.to_string()),
        Line::raw(""),
        dim("The Board stays in Jira. Only My Boards changes."),
        Line::raw(""),
        dim("y remove · n keep"),
    ];
    let w = 64u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(" Remove from My Boards? ").padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

// ------------------------------------------------------------------ Sprint tab

/// The line above the Sprint list: the Board and the parts (`Sprint 21 │ Backlog`). Returns
/// the area below it, or `None` when the tab shows only a message.
fn draw_sprint_head(f: &mut Frame, st: &State, area: Rect) -> Option<Rect> {
    let sp = &st.sprint;
    let mut message = |lines: Vec<Line<'static>>| {
        f.render_widget(Paragraph::new(lines).block(Block::bordered().title("Sprint")).wrap(Wrap { trim: false }), area);
        None
    };
    if sp.board_id.is_none() {
        return message(vec![
            Line::raw(""),
            Line::raw("  My Boards is empty. Push 2, then + to select a Scrum Board."),
            dim("  The Sprint tab shows the active Sprints and the backlog of the Board on the Boards tab."),
        ]);
    }
    let Some(b) = &sp.data else {
        return match &sp.error {
            Some(e) => message(vec![Line::raw(""), Line::styled(format!("  {e}"), Style::new().fg(Color::Red)), Line::raw("  Push r to try again.")]),
            None => message(vec![Line::raw(""), Line::raw("  Loading the Sprints…")]),
        };
    };
    if !b.board.is_scrum() {
        return message(vec![
            Line::raw(""),
            Line::raw(format!("  {} is a Kanban Board. It has no Sprints and no backlog.", b.board.name)),
            dim("  Push 2 and ] to select a Scrum Board."),
        ]);
    }
    let [head, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    let mut spans = vec![Span::raw(" "), Span::styled(b.board.name.clone(), Style::new().add_modifier(Modifier::BOLD)), Span::raw("  ")];
    let sections = sp.sections();
    if sections.is_empty() {
        spans.push(dim("The Board has no filter.").spans.remove(0));
    }
    for (i, s) in sections.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::new().fg(Color::DarkGray)));
        }
        let style = if i == sp.section { Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED) } else { Style::new() };
        spans.push(Span::styled(format!(" {} ", s.name()), style));
    }
    if b.sprints.is_empty() {
        spans.push(dim("  (no active Sprint)").spans.remove(0));
    }
    if sp.only_mine {
        spans.push(Span::styled("  only mine", Style::new().fg(Color::Magenta)));
    }
    if sp.loading {
        spans.push(Span::styled("  Loading…", Style::new().fg(Color::Yellow)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), head);
    Some(rest)
}

fn draw_sprint_popup(f: &mut Frame, st: &State, area: Rect, p: &SprintPopup) {
    let mut lines = vec![Line::from(vec![
        Span::styled("Filter: ", Style::new().fg(Color::DarkGray)),
        Span::raw(p.filter.clone()),
        Span::styled("▏", Style::new().fg(Color::DarkGray)),
    ])];
    lines.push(dim(format!("Board: {}", p.board_name)));
    lines.push(Line::raw(""));
    let current = st.cache.get(&p.key).and_then(|t| t.sprint.as_ref()).filter(|s| !s.state.eq_ignore_ascii_case("closed")).map(|s| s.id);
    match &p.list {
        None => lines.push(dim(format!("{} Getting the Sprints…", SPINNER[st.spinner % SPINNER.len()]))),
        Some(_) => {
            let rows = p.rows();
            if rows.is_empty() {
                lines.push(dim("No match."));
            }
            for (i, r) in rows.iter().enumerate() {
                let on = i == p.selected;
                let mut spans = vec![Span::raw(if on { "▶ " } else { "  " })];
                match r {
                    crate::core::agile::Target::Sprint(s) => {
                        spans.push(Span::raw(s.name.clone()));
                        let style = if s.is_active() { Style::new().fg(Color::Blue) } else { Style::new().fg(Color::DarkGray) };
                        spans.push(Span::styled(format!("  {}", s.state), style));
                        if let Some(e) = s.end {
                            spans.push(Span::styled(format!("  ends {}", e.format("%Y-%m-%d")), Style::new().fg(Color::DarkGray)));
                        }
                        if current == Some(s.id) {
                            spans.push(Span::styled("  (now)", Style::new().fg(Color::Yellow)));
                        }
                    }
                    crate::core::agile::Target::Backlog => {
                        spans.push(Span::styled("Backlog", Style::new().fg(Color::Cyan)));
                        spans.push(Span::styled("  (no Sprint)", Style::new().fg(Color::DarkGray)));
                        if current.is_none() && st.cache.contains(&p.key) {
                            spans.push(Span::styled("  (now)", Style::new().fg(Color::Yellow)));
                        }
                    }
                }
                let mut line = Line::from(spans);
                if on {
                    line = line.style(Style::new().add_modifier(Modifier::REVERSED));
                }
                lines.push(line);
            }
        }
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.sending {
        lines.push(Line::styled(format!("{} Sending…", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    } else {
        lines.push(dim("Type to filter · ↑↓ select · Enter move · Esc close"));
    }
    let w = 64u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Move {} to a Sprint ", p.key)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_epic_popup(f: &mut Frame, st: &State, area: Rect, p: &EpicPopup) {
    let mut lines = vec![Line::from(vec![
        Span::styled("Epic: ", Style::new().fg(Color::DarkGray)),
        Span::raw(p.input.clone()),
        Span::styled("▏", Style::new().fg(Color::DarkGray)),
    ])];
    lines.push(Line::raw(""));
    let rows = p.rows();
    let current = st.cache.get(&p.key).and_then(|t| t.epic.as_ref()).map(|e| e.key.clone());
    let w = 76u16.min(area.width.saturating_sub(4));
    let max_rows = (area.height.saturating_sub(12) as usize).clamp(3, 15);
    let start = p.selected.saturating_sub(max_rows - 1);
    for (i, r) in rows.iter().enumerate().skip(start).take(max_rows) {
        let on = i == p.selected;
        let mut spans = vec![Span::raw(if on { "▶ " } else { "  " })];
        match r {
            EpicRow::Nobody => {
                spans.push(Span::styled("none (remove the Epic)", Style::new().fg(Color::Cyan)));
                if current.is_none() && st.cache.contains(&p.key) {
                    spans.push(Span::styled("  (now)", Style::new().fg(Color::Yellow)));
                }
            }
            EpicRow::Epic(e) => {
                spans.push(Span::styled(format!("{:<10} ", e.key), Style::new().fg(Color::Cyan)));
                spans.push(Span::raw(e.name.clone()));
                if current.as_deref() == Some(e.key.as_str()) {
                    spans.push(Span::styled("  (now)", Style::new().fg(Color::Yellow)));
                }
            }
        }
        let mut line = Line::from(spans);
        if on {
            line = line.style(Style::new().add_modifier(Modifier::REVERSED));
        }
        lines.push(line);
    }
    if rows.len() > start + max_rows {
        lines.push(dim(format!("  … {} more. Type to search.", rows.len() - start - max_rows)));
    }
    let spin = SPINNER[st.spinner % SPINNER.len()];
    if p.searching {
        lines.push(dim(format!("{spin} Searching…")));
    } else if p.base.is_none() && !p.searches() {
        lines.push(dim(format!("{spin} Getting the open Epics of {}…", p.project)));
    } else if p.searches() && p.results.as_ref().is_some_and(Vec::is_empty) {
        lines.push(dim("No open Epic matches."));
    } else if !p.searches() && !p.input.trim().is_empty() && rows.is_empty() {
        lines.push(dim(format!("Type {USER_SEARCH_MIN} characters or more to search all open Epics of {}.", p.project)));
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.sending {
        lines.push(Line::styled(format!("{spin} Sending…"), Style::new().fg(Color::Yellow)));
    } else {
        lines.push(dim("Type a key or words · ↑↓ select · Enter set · Esc close"));
    }
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Epic of {} ", p.key)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn later_tab_text() -> Vec<Line<'static>> {
    vec![Line::raw(""), Line::raw("  This tab has no content."), Line::raw("  Use 1 for My Tickets.")]
}

// ------------------------------------------------------------------ Settings page

fn source_text(s: crate::core::settings::Source) -> &'static str {
    use crate::core::settings::Source;
    match s {
        Source::Default => "default",
        Source::File => "file",
        Source::Env => "env",
    }
}

/// The Settings page: a table (Setting, Value, Source) in groups, and the help of the
/// selected row below it.
fn draw_settings(f: &mut Frame, st: &mut State, area: Rect) {
    use crate::core::settings::{self as cfg, Source};
    let [table_area, help_area] = Layout::vertical([Constraint::Min(5), Constraint::Length(4)]).areas(area);
    let rows = st.settings_rows();
    let s = &st.settings;
    let label_w = 22usize;
    let source_w = 8usize;
    let value_w = (table_area.width as usize).saturating_sub(label_w + source_w + 8).max(10);
    let group_style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

    // The table rows: a title row before each group. `index` maps each selectable row to its table row.
    let mut table_rows: Vec<Row> = Vec::new();
    let mut index: Vec<usize> = Vec::new();
    let mut last_group = "";
    for r in &rows {
        let group = match r {
            SettingsRow::Value(m) => m.group,
            SettingsRow::Board(_) | SettingsRow::AddBoards => "My Boards",
        };
        if group != last_group {
            if !last_group.is_empty() {
                table_rows.push(Row::new(vec![Cell::from("")]));
            }
            table_rows.push(Row::new(vec![Cell::from(Span::styled(group.to_string(), group_style))]));
            last_group = group;
        }
        index.push(table_rows.len());
        let row = match r {
            SettingsRow::Value(m) => {
                let src = s.source(m.key);
                let mut value = s.page_value(m.key).unwrap_or_default();
                if value.is_empty() {
                    value = match m.key {
                        "editor.command" => format!("(empty: {})", crate::editor::editor_from_env("")),
                        _ => "(empty)".into(),
                    };
                }
                let value_style = match src {
                    Source::Default => Style::new().fg(Color::DarkGray),
                    _ => Style::new(),
                };
                let mut value_cell = vec![Span::styled(cut(&value, value_w), value_style)];
                if src == Source::Env {
                    value_cell = vec![Span::styled(cut(&s.get(m.key).unwrap_or_default(), value_w), Style::new().fg(Color::Magenta))];
                }
                Row::new(vec![
                    Cell::from(format!("  {}", m.label)),
                    Cell::from(Line::from(value_cell)),
                    Cell::from(Span::styled(source_text(src), source_style(src))),
                ])
            }
            SettingsRow::Board(i) => {
                let b = &st.my_boards[*i];
                let name = b.name.clone().unwrap_or_else(|| "(no name)".into());
                Row::new(vec![
                    Cell::from(format!("  {}. Board {}", i + 1, b.id)),
                    Cell::from(cut(&name, value_w)),
                    Cell::from(Span::styled("file", source_style(Source::File))),
                ])
            }
            SettingsRow::AddBoards => {
                let (label, text) = if st.my_boards.is_empty() {
                    ("  (empty)", "Enter: add Boards with the Board picker")
                } else {
                    ("  + Boards", "Enter: add or remove Boards with the Board picker")
                };
                let grey = Style::new().fg(Color::DarkGray);
                Row::new(vec![Cell::from(Span::styled(label, grey)), Cell::from(Span::styled(text, grey))])
            }
        };
        table_rows.push(row);
    }

    let selected = index.get(st.settings_page.selected).copied();
    st.settings_page.table.select(selected);
    let title = if st.config_path.as_os_str().is_empty() {
        " Settings ".to_string()
    } else {
        format!(" Settings · {} ", st.config_path.display())
    };
    let table = Table::new(
        table_rows,
        [Constraint::Length(label_w as u16), Constraint::Min(10), Constraint::Length(source_w as u16)],
    )
    .header(Row::new(vec!["  Setting", "Value", "Source"]).style(Style::new().add_modifier(Modifier::BOLD)))
    .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
    .block(Block::bordered().title(title));
    f.render_stateful_widget(table, table_area, &mut st.settings_page.table);

    // The help of the selected row.
    let mut help: Vec<Line<'static>> = Vec::new();
    match st.selected_setting() {
        Some(SettingsRow::Value(m)) => {
            help.push(Line::from(vec![
                Span::styled(format!("{}  ", m.key), Style::new().fg(Color::Cyan)),
                Span::raw(m.help),
            ]));
            let def = cfg::default_value(m.key).unwrap_or_default();
            let def = if def.is_empty() { "(empty)".to_string() } else { def };
            help.push(dim(format!("Default: {def}")));
            if s.source(m.key) == Source::Env
                && let Some(env) = cfg::env_name(m.key)
            {
                help.push(Line::styled(
                    format!("{env} replaces this value. Enter edits the file value: {}", s.page_value(m.key).unwrap_or_default()),
                    Style::new().fg(Color::Magenta),
                ));
            }
        }
        Some(SettingsRow::Board(_)) => {
            help.push(Line::raw("A Board of My Boards. The order is the order of the Boards tab ([ and ])."));
            help.push(dim("K and J change the order. d removes the Board from My Boards. The Board stays in Jira."));
        }
        Some(SettingsRow::AddBoards) | None => {
            help.push(Line::raw("The Board picker suggests your recent Boards and the Boards of the default project."));
            help.push(dim("Type 2 characters or more to search all Boards."));
        }
    }
    let help = wrap_lines(help, help_area.width.saturating_sub(2));
    f.render_widget(Paragraph::new(help).block(Block::new().borders(Borders::TOP).border_style(Style::new().fg(Color::DarkGray))), help_area);
}

fn source_style(s: crate::core::settings::Source) -> Style {
    use crate::core::settings::Source;
    match s {
        Source::Default => Style::new().fg(Color::DarkGray),
        Source::File => Style::new().fg(Color::Green),
        Source::Env => Style::new().fg(Color::Magenta),
    }
}

fn draw_setting_popup(f: &mut Frame, st: &State, area: Rect, p: &SettingPopup) {
    use crate::core::settings::Kind;
    let spin = SPINNER[st.spinner % SPINNER.len()];
    let wide = p.meta.kind == Kind::Jql;
    let w = (if wide { 110u16 } else { 72 }).min(area.width.saturating_sub(4));
    let inner = w.saturating_sub(4) as usize;
    let mut lines: Vec<Line<'static>> = vec![dim(p.meta.help.to_string()), Line::raw("")];
    let label = if p.is_list() { "Filter: " } else { "Value: " };
    lines.push(Line::from(vec![
        Span::styled(label, Style::new().fg(Color::DarkGray)),
        Span::raw(p.input.clone()),
        Span::styled(if p.busy() { "" } else { "▏" }, Style::new().fg(Color::DarkGray)),
    ]));
    if p.is_list() {
        lines.push(Line::raw(""));
        match &p.choices {
            None => lines.push(dim(format!("{spin} Getting the Ticket types of {}…", st.default_project))),
            Some(_) => {
                let current = st.settings.page_value(p.meta.key).unwrap_or_default();
                let shown = p.shown();
                if shown.is_empty() && p.choices.as_ref().is_some_and(|c| !c.is_empty()) {
                    lines.push(dim("No match."));
                }
                let start = p.selected.saturating_sub(9);
                for (i, v) in shown.iter().enumerate().skip(start).take(10) {
                    let on = i == p.selected;
                    let mut spans = vec![Span::raw(if on { "▶ " } else { "  " }), Span::raw(v.clone())];
                    if v.eq_ignore_ascii_case(&current) {
                        spans.push(Span::styled("  (now)", Style::new().fg(Color::Yellow)));
                    }
                    let mut line = Line::from(spans);
                    if on {
                        line = line.style(Style::new().add_modifier(Modifier::REVERSED));
                    }
                    lines.push(line);
                }
            }
        }
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.checking {
        lines.push(Line::styled(format!("{spin} Checking project {}…", p.input), Style::new().fg(Color::Yellow)));
    } else if p.saving {
        lines.push(Line::styled(format!("{spin} Saving…"), Style::new().fg(Color::Yellow)));
    } else if p.is_list() {
        lines.push(dim("Type to filter · ↑↓ select · Enter save · Esc close"));
    } else {
        lines.push(dim("Enter save · Ctrl-U clear · Esc close"));
    }
    let lines = wrap_lines(lines, inner as u16);
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" {} ", p.meta.label)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_config_invalid(f: &mut Frame, area: Rect, p: &ConfigInvalid) {
    let lines = vec![
        Line::styled(p.message.clone(), Style::new().fg(Color::Red)),
        Line::raw(""),
        Line::raw("jui did not save the file."),
        Line::raw(""),
        dim("e edit again · q discard the changes"),
    ];
    let w = 80u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(" The config file is not valid ").padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

pub fn status_style(status: &str, cat: StatusCategory) -> Style {
    if status == "Blocked" {
        return Style::new().fg(Color::Red);
    }
    match cat {
        StatusCategory::Done => Style::new().fg(Color::Green),
        StatusCategory::InProgress => Style::new().fg(Color::Blue),
        StatusCategory::ToDo => Style::new().fg(Color::Gray),
        StatusCategory::Unknown => Style::new(),
    }
}

fn draw_list(f: &mut Frame, st: &mut State, area: Rect) {
    let wide = st.width > WIDE;
    let now = st.now();
    let date_format = st.date_format;
    let title_name = if st.tab == SPRINT { st.sprint.current().map(|s| s.name()).unwrap_or_else(|| "Sprint".into()) } else { TAB_TITLES[st.tab].to_string() };
    let tab = st.tab;
    let list = st.tabs[tab].list.as_mut().expect("draw_list needs a list");

    let mut title = title_name;
    if let Some(c) = list.count() {
        title.push_str(&format!(" ({c})"));
    }
    if list.offline {
        title.push_str(" offline");
    }
    let block = Block::bordered().title(title);

    if list.rows.is_empty() {
        let text = if list.loading || (tab == EPICS && st.epics.loading) {
            "Loading…"
        } else if tab == SEARCH && st.search.error.is_some() {
            "Correct the JQL above."
        } else if tab == EPICS && list.total == Some(0) {
            "None of your open Tickets is in an Epic."
        } else if list.total == Some(0) {
            "No Tickets."
        } else {
            "No data. Push r to try again."
        };
        f.render_widget(Paragraph::new(format!(" {text}")).block(block), area);
        return;
    }

    let dim_style = Style::new().add_modifier(Modifier::DIM);
    // The Epics tab: a "Mine" column (the number of open Tickets of the user in the Epic)
    // replaces the Type column, and a Done Epic is dim.
    let epic_counts = (tab == EPICS).then_some(&st.epics.counts);
    let row_cells = |r: &crate::core::model::ListRow| {
        let t = &r.ticket;
        let done = epic_counts.is_some() && t.status_category == StatusCategory::Done;
        let row_style = if list.stale || list.gone.contains(&t.key) || done { dim_style } else { Style::new() };
        let mut cells = vec![Cell::from(t.key.clone())];
        if let Some(c) = epic_counts {
            cells.push(Cell::from(c.get(&t.key).map(|n| n.to_string()).unwrap_or_default()));
        } else if wide {
            cells.push(Cell::from(t.kind.clone()));
        }
        cells.push(Cell::from(Span::styled(t.status.clone(), status_style(&t.status, t.status_category))));
        if wide {
            cells.push(Cell::from(t.priority.clone().unwrap_or_default()));
            cells.push(Cell::from(t.updated.map(|d| format_date(d, date_format, now)).unwrap_or_default()));
        }
        cells.push(Cell::from(t.summary.clone()));
        Row::new(cells).style(row_style)
    };
    let key_w = list.rows.iter().map(|r| r.ticket.key.width()).max().unwrap_or(9) as u16;
    let status_w = list.rows.iter().map(|r| r.ticket.status.width()).max().unwrap_or(6).min(22) as u16;
    let mut widths = vec![Constraint::Length(key_w)];
    let mut header = vec!["Key"];
    if epic_counts.is_some() {
        widths.push(Constraint::Length(4));
        header.push("Mine");
    } else if wide {
        widths.push(Constraint::Length(list_width(&list.rows, |t| t.kind.width(), "Type")));
        header.push("Type");
    }
    widths.push(Constraint::Length(status_w));
    header.push("Status");
    if wide {
        widths.push(Constraint::Length(list_width(&list.rows, |t| t.priority.as_deref().unwrap_or("").width(), "Priority")));
        widths.push(Constraint::Length(7));
        header.push("Priority");
        header.push("Updated");
    }
    widths.push(Constraint::Min(10));
    header.push("Summary");

    // With groups by Sprint, a heading row comes before each group. `index` maps each
    // Ticket row to its table row.
    // A table cell cannot span columns, so a heading is an empty row, and its text is drawn
    // over the row after the table (`headings`: table row and text line).
    let groups = list.sprint_groups();
    let mut rows: Vec<Row> = Vec::with_capacity(list.rows.len() + groups.len() * 2);
    let mut index: Vec<usize> = Vec::with_capacity(list.rows.len());
    let mut headings: Vec<(usize, Line<'static>)> = Vec::with_capacity(groups.len());
    if groups.is_empty() {
        for r in &list.rows {
            index.push(rows.len());
            rows.push(row_cells(r));
        }
    } else {
        let more = list.has_more();
        for (n, g) in groups.iter().enumerate() {
            if n > 0 {
                rows.push(Row::new(vec![Cell::from("")]));
            }
            // A group can get more Tickets from the next page.
            let count = g.range.len();
            let count = if more { format!("{count}+") } else { count.to_string() };
            let style = match &g.sprint {
                Some(s) if s.is_active() => Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                Some(_) => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                None => Style::new().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
            };
            let line = Line::from(vec![Span::styled(g.title(), style), Span::styled(format!(" · {count}"), Style::new().fg(Color::DarkGray))]);
            headings.push((rows.len(), line));
            rows.push(Row::new(vec![Cell::from("")]));
            for r in &list.rows[g.range.clone()] {
                index.push(rows.len());
                rows.push(row_cells(r));
            }
        }
    }
    if list.loading && !list.stale {
        rows.push(Row::new(vec![Cell::from(Span::styled("Loading…", Style::new().fg(Color::DarkGray)))]));
    }

    let table = Table::new(rows, widths)
        .header(Row::new(header).style(Style::new().add_modifier(Modifier::BOLD)))
        .block(block)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
    let selected = index.get(list.selected).copied();
    // Show the heading of the first group when the first Ticket is selected.
    if list.selected == 0 && !groups.is_empty() {
        *list.table.offset_mut() = 0;
    }
    list.table.select(selected);
    f.render_stateful_widget(table, area, &mut list.table);

    // The heading texts, on the visible heading rows. Inside the border, below the header
    // row, after the column of the highlight symbol.
    let offset = list.table.offset();
    let top = area.y + 2;
    let visible = area.height.saturating_sub(3) as usize;
    let x = area.x + 1 + HIGHLIGHT_W;
    let w = area.width.saturating_sub(2 + HIGHLIGHT_W);
    for (row, line) in headings {
        if row >= offset && row - offset < visible {
            let r = Rect { x, y: top + (row - offset) as u16, width: w, height: 1 };
            f.render_widget(Paragraph::new(line), r);
        }
    }
}

/// The width of the highlight symbol `▶ ` of the lists.
const HIGHLIGHT_W: u16 = 2;

/// The width of a column: the widest value, at least the header, at most 14.
fn list_width(rows: &[crate::core::model::ListRow], w: impl Fn(&TicketSummary) -> usize, header: &str) -> u16 {
    rows.iter().map(|r| w(&r.ticket)).max().unwrap_or(0).max(header.width()).min(14) as u16
}

fn draw_preview(f: &mut Frame, st: &mut State, area: Rect) {
    let block = Block::bordered().title("Preview");
    let inner_w = area.width.saturating_sub(2);
    let selected = if st.tab == BOARDS {
        st.boards.selected_key().map(str::to_string)
    } else {
        st.current().list.as_ref().and_then(|l| l.selected_key()).map(str::to_string)
    };
    let Some(key) = selected else {
        f.render_widget(block, area);
        return;
    };
    let cache_key = (key.clone(), st.cache.contains(&key), None, inner_w);
    if st.preview_render.key.as_ref() != Some(&cache_key) {
        let lines = match st.cache.get(&key) {
            Some(t) => ticket_lines(t, Mode::Preview, None, st).0,
            None => preview_from_row(st, &key),
        };
        st.preview_render.lines = wrap_lines(lines, inner_w);
        st.preview_render.key = Some(cache_key);
    }
    let height = area.height.saturating_sub(2);
    let max = (st.preview_render.lines.len() as u16).saturating_sub(height);
    let scroll = st.current().preview_scroll.min(max);
    st.current_mut().preview_scroll = scroll;
    let p = Paragraph::new(st.preview_render.lines.clone()).block(block).scroll((scroll, 0));
    f.render_widget(p, area);
}

/// The preview before the full Ticket arrives: the list fields and the description.
fn preview_from_row(st: &State, key: &str) -> Vec<Line<'static>> {
    let row = st.current().list.as_ref().and_then(|l| l.rows.iter().find(|r| r.ticket.key == key));
    // A Board card has no description. The preview shows the card fields until the full Ticket arrives.
    let card = st.boards.view.as_ref().and_then(|v| v.columns.iter().flat_map(|c| c.tickets.iter()).find(|t| t.key == key));
    let (t, description) = match (row, card) {
        (Some(r), _) => (&r.ticket, Some(r.description.as_deref())),
        (None, Some(c)) if st.tab == BOARDS => (c, None),
        _ => return Vec::new(),
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(t.key.clone(), Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(t.summary.clone(), Style::new().add_modifier(Modifier::BOLD)),
        ]),
        meta_line(&t.status, t.status_category, &t.kind, t.priority.as_deref()),
        field_line("Assignee", t.assignee.as_ref().map(|u| u.display_name.clone()).unwrap_or_else(|| "Unassigned".into())),
        Line::raw(""),
    ];
    match description {
        Some(Some(d)) => lines.extend(markup::display(d).iter().map(to_line)),
        Some(None) => lines.push(dim("(no description)")),
        None => {}
    }
    lines.push(Line::raw(""));
    match st.failed.get(key) {
        Some(e) => lines.push(Line::styled(format!("Cannot get the full Ticket: {e}"), Style::new().fg(Color::Red))),
        None if description.is_none() => lines.push(dim("Loading the description, comments, Sub-tasks, and Links…")),
        None => lines.push(dim("Loading comments, Sub-tasks, and Links…")),
    }
    lines
}

// ------------------------------------------------------------------ zoom

fn draw_zoom(f: &mut Frame, st: &mut State, area: Rect) {
    let [crumb, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    let frame = st.current().zoom.last().cloned().expect("draw_zoom needs a zoom frame");

    let mut parts = vec![Span::styled(format!(" {}", TAB_TITLES[st.tab]), Style::new().fg(Color::DarkGray))];
    for (i, z) in st.current().zoom.iter().enumerate() {
        parts.push(Span::styled(" › ", Style::new().fg(Color::DarkGray)));
        let last = i + 1 == st.current().zoom.len();
        let style = if last { Style::new().add_modifier(Modifier::BOLD) } else { Style::new().fg(Color::DarkGray) };
        parts.push(Span::styled(z.key.clone(), style));
    }
    f.render_widget(Paragraph::new(Line::from(parts)), crumb);

    let block = Block::new().borders(Borders::TOP);
    let inner_w = rest.width;
    let height = rest.height.saturating_sub(1);
    let Some(t) = st.cache.get(&frame.key) else {
        let text = match st.failed.get(&frame.key) {
            Some(e) => Line::styled(format!(" Cannot get {}: {e}. Push r to try again.", frame.key), Style::new().fg(Color::Red)),
            None => Line::raw(format!(" Loading {}…", frame.key)),
        };
        f.render_widget(Paragraph::new(text).block(block), rest);
        return;
    };
    let cache_key = (frame.key.clone(), true, Some(frame.item), inner_w);
    if st.zoom_render.key.as_ref() != Some(&cache_key) {
        let (lines, items) = ticket_lines(t, Mode::Zoom, Some(frame.item), st);
        let (lines, items) = wrap_with_items(lines, &items, inner_w);
        st.zoom_render.lines = lines;
        st.zoom_render.items = items;
        st.zoom_render.key = Some(cache_key);
    }
    let total = st.zoom_render.lines.len() as u16;
    let max = total.saturating_sub(height);
    let mut scroll = frame.scroll.min(max);
    if frame.follow
        && let Some(&line) = st.zoom_render.items.get(frame.item)
    {
        let line = line as u16;
        if line >= scroll + height {
            scroll = line + 1 - height;
        } else if line < scroll {
            scroll = line;
        }
    }
    if let Some(z) = st.current_mut().zoom.last_mut() {
        z.scroll = scroll;
    }
    let p = Paragraph::new(st.zoom_render.lines.clone()).block(block).scroll((scroll, 0));
    f.render_widget(p, rest);
}

// ------------------------------------------------------------------ Ticket text

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The last 2 comments; no Worklogs.
    Preview,
    /// All comments and Worklogs; Sub-tasks and Links can be selected.
    Zoom,
}

fn dim(text: impl Into<String>) -> Line<'static> {
    Line::styled(text.into(), Style::new().fg(Color::DarkGray))
}

fn field_line(name: &str, value: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{name:<11}"), Style::new().fg(Color::DarkGray)), Span::raw(value)])
}

fn meta_line(status: &str, cat: StatusCategory, kind: &str, priority: Option<&str>) -> Line<'static> {
    let sep = || Span::styled(" · ", Style::new().fg(Color::DarkGray));
    let mut spans = vec![Span::styled(status.to_string(), status_style(status, cat).add_modifier(Modifier::BOLD)), sep(), Span::raw(kind.to_string())];
    if let Some(p) = priority {
        spans.push(sep());
        spans.push(Span::raw(p.to_string()));
    }
    Line::from(spans)
}

fn section(title: String) -> [Line<'static>; 2] {
    [Line::raw(""), Line::styled(title, Style::new().add_modifier(Modifier::BOLD))]
}

/// The lines of a full Ticket, and the line index of each selectable item (zoom only).
fn ticket_lines(t: &Ticket, mode: Mode, selected: Option<usize>, st: &State) -> (Vec<Line<'static>>, Vec<usize>) {
    let now = st.now();
    let when = |d: Option<chrono::DateTime<chrono::FixedOffset>>| d.map(|d| format_when(d, st.date_format, now)).unwrap_or_default();
    let user = |u: &Option<crate::core::model::User>| u.as_ref().map(|u| format!("{} ({})", u.display_name, u.name)).unwrap_or_else(|| "Unassigned".into());
    let mut out = vec![
        Line::from(vec![
            Span::styled(t.key.clone(), Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(t.summary.clone(), Style::new().add_modifier(Modifier::BOLD)),
        ]),
        meta_line(&t.status, t.status_category, &t.kind, t.priority.as_deref()),
        field_line("Assignee", user(&t.assignee)),
        field_line("Reporter", user(&t.reporter)),
    ];
    if t.watching || t.watch_count > 0 {
        let you = if t.watching { "you watch · " } else { "" };
        let n = t.watch_count;
        out.push(field_line("Watchers", format!("{you}{n} watcher{}", if n == 1 { "" } else { "s" })));
    }
    if let Some(p) = &t.parent {
        out.push(field_line("Parent", format!("{} {}", p.key, p.summary)));
    }
    if let Some(e) = &t.epic {
        out.push(field_line("Epic", format!("{} {}", e.key, e.name.as_deref().unwrap_or(""))));
    }
    if let Some(s) = &t.sprint {
        out.push(field_line("Sprint", format!("{} ({})", s.name, s.state.to_lowercase())));
    }
    if let Some(sp) = t.story_points {
        out.push(field_line("Points", format!("{sp}")));
    }
    if !t.labels.is_empty() {
        out.push(field_line("Labels", t.labels.join(", ")));
    }
    if !t.components.is_empty() {
        out.push(field_line("Components", t.components.join(", ")));
    }
    if mode == Mode::Zoom {
        if !t.fix_versions.is_empty() {
            out.push(field_line("Fix ver.", t.fix_versions.join(", ")));
        }
        out.push(field_line("Created", when(t.created)));
    }
    out.push(field_line("Updated", when(t.updated)));

    out.push(Line::raw(""));
    match &t.description {
        Some(d) => out.extend(text_lines(d)),
        None => out.push(dim("(no description)")),
    }

    let mut items = Vec::new();
    let mut index = 0usize;
    let mut item_line = |out: &mut Vec<Line<'static>>, mut line: Line<'static>| {
        if mode == Mode::Zoom {
            let on = selected == Some(index);
            line.spans.insert(0, Span::raw(if on { "▶ " } else { "  " }));
            if on {
                line = line.style(Style::new().add_modifier(Modifier::REVERSED));
            }
            items.push(out.len());
            index += 1;
        } else {
            line.spans.insert(0, Span::raw("  "));
        }
        out.push(line);
    };
    let status_pad = t
        .subtasks
        .iter()
        .map(|s| s.status.width())
        .chain(t.links.iter().map(|l| l.ticket.status.width()))
        .chain(st.children_of(&t.key).iter().map(|c| c.status.width()))
        .max()
        .unwrap_or(6);
    let related = |key: &str, status: &str, cat: StatusCategory, summary: &str, pad: usize| {
        Line::from(vec![
            Span::styled(format!("{key:<pad$} "), Style::new().fg(Color::Cyan)),
            Span::styled(format!("{status:<status_pad$} "), status_style(status, cat)),
            Span::raw(summary.to_string()),
        ])
    };
    let key_pad = t.subtasks.iter().map(|s| s.key.len()).chain(t.links.iter().map(|l| l.ticket.key.len())).max().unwrap_or(10);

    if !t.subtasks.is_empty() {
        out.extend(section(format!("Sub-tasks ({})", t.subtasks.len())));
        for s in &t.subtasks {
            item_line(&mut out, related(&s.key, &s.status, s.status_category, &s.summary, key_pad));
        }
    }
    if t.kind.eq_ignore_ascii_case("epic") {
        match st.epic_children.get(&t.key) {
            Some(EpicChildren::Loaded { tickets, total }) => {
                let title = if (tickets.len() as u64) < *total {
                    format!("Tickets in this Epic ({} of {total})", tickets.len())
                } else {
                    format!("Tickets in this Epic ({total})")
                };
                out.extend(section(title));
                if tickets.is_empty() {
                    out.push(dim("  (no Tickets)"));
                }
                let pad = tickets.iter().map(|c| c.key.len()).max().unwrap_or(10).max(key_pad);
                for c in tickets.iter() {
                    item_line(&mut out, related(&c.key, &c.status, c.status_category, &c.summary, pad));
                }
                if (tickets.len() as u64) < *total {
                    out.push(dim(format!("  Use `jui epic tickets {}` for all Tickets.", t.key)));
                }
            }
            Some(EpicChildren::Failed(e)) => {
                out.extend(section("Tickets in this Epic".into()));
                out.push(Line::styled(format!("  Cannot get the Tickets: {e}. Push r to try again."), Style::new().fg(Color::Red)));
            }
            Some(EpicChildren::Loading) => {
                out.extend(section("Tickets in this Epic".into()));
                out.push(dim("  Loading…"));
            }
            None => {}
        }
    }
    if !t.links.is_empty() {
        out.extend(section(format!("Links ({})", t.links.len())));
        let rel_pad = t.links.iter().map(|l| l.relation.width()).max().unwrap_or(10);
        for l in &t.links {
            let mut line = related(&l.ticket.key, &l.ticket.status, l.ticket.status_category, &l.ticket.summary, key_pad);
            line.spans.insert(0, Span::styled(format!("{:<rel_pad$} ", l.relation), Style::new().fg(Color::DarkGray)));
            item_line(&mut out, line);
        }
    }
    if !t.attachments.is_empty() {
        out.extend(section(format!("Attachments ({})", t.attachments.len())));
        let name_pad = t.attachments.iter().map(|a| a.filename.width()).max().unwrap_or(10);
        for a in &t.attachments {
            // In the zoom, each attachment is a selectable item (Enter, `d`).
            let who = a.author.as_ref().map(|u| format!("  @{}", u.name)).unwrap_or_default();
            let line = Line::from(vec![
                Span::raw(format!("{:<name_pad$}  ", a.filename)),
                Span::styled(format!("{:>7}", human_size(a.size)), Style::new().fg(Color::DarkGray)),
                Span::styled(who, Style::new().fg(Color::Cyan)),
            ]);
            item_line(&mut out, line);
        }
    }
    if mode == Mode::Zoom && !t.worklogs.is_empty() {
        let total: u64 = t.worklogs.iter().map(|w| w.time_spent_seconds).sum();
        out.extend(section(format!("Worklogs ({}, total {})", t.worklogs.len(), human_duration(total))));
        let time_pad = t.worklogs.iter().map(|w| human_duration(w.time_spent_seconds).len()).max().unwrap_or(6);
        for w in &t.worklogs {
            let who = w.author.as_ref().map(|u| u.name.as_str()).unwrap_or("?");
            let day = w.started.map(|d| d.format(crate::core::worklog::STARTED_FORMAT).to_string()).unwrap_or_default();
            // In the zoom, each Worklog is a selectable item (`e`, `d`).
            let line = Line::from(vec![
                Span::styled(format!("{day}  "), Style::new().fg(Color::DarkGray)),
                Span::raw(format!("{:<time_pad$}  ", human_duration(w.time_spent_seconds))),
                Span::styled(format!("@{who}"), Style::new().fg(Color::Cyan)),
                Span::raw(match crate::core::worklog::first_line(w, 60) {
                    c if c.is_empty() => String::new(),
                    c => format!("  {c}"),
                }),
            ]);
            item_line(&mut out, line);
        }
    }

    if t.comment_total > 0 {
        let n = match mode {
            Mode::Preview => t.comments.len().min(2),
            Mode::Zoom => t.comments.len(),
        };
        let title = if n < t.comment_total as usize {
            let more = if mode == Mode::Preview { ", Enter for all" } else { "" };
            format!("Comments ({n} of {}{more})", t.comment_total)
        } else {
            format!("Comments ({})", t.comment_total)
        };
        out.extend(section(title));
        for c in &t.comments[t.comments.len() - n..] {
            let who = c.author.as_ref().map(|u| u.name.as_str()).unwrap_or("?");
            out.push(Line::raw(""));
            let edited = match (c.created, c.updated) {
                (Some(a), Some(b)) if b > a => "  (edited)",
                _ => "",
            };
            let head = Line::from(vec![
                Span::styled(format!("@{who}"), Style::new().fg(Color::Cyan)),
                Span::raw("  "),
                Span::styled(format!("{}{edited}", when(c.created)), Style::new().fg(Color::DarkGray)),
            ]);
            // In the zoom, the author line of each comment is a selectable item (`e`, `W`, `d`).
            item_line(&mut out, head);
            for mut l in text_lines(&c.body) {
                l.spans.insert(0, Span::raw("    "));
                out.push(l);
            }
        }
    }
    (out, items)
}

fn text_lines(t: &Text) -> Vec<Line<'static>> {
    markup::display(&t.wiki).iter().map(to_line).collect()
}

/// Convert a display line from `core::markup` to a ratatui line.
pub fn to_line(l: &markup::Line) -> Line<'static> {
    Line::from(l.spans.iter().map(|s| Span::styled(s.text.clone(), to_style(s.style))).collect::<Vec<_>>())
}

fn to_style(s: markup::Style) -> Style {
    let mut st = Style::new();
    if s.bold {
        st = st.add_modifier(Modifier::BOLD);
    }
    if s.italic {
        st = st.add_modifier(Modifier::ITALIC);
    }
    if s.underline {
        st = st.add_modifier(Modifier::UNDERLINED);
    }
    if s.strike {
        st = st.add_modifier(Modifier::CROSSED_OUT);
    }
    if s.dim {
        st = st.add_modifier(Modifier::DIM);
    }
    if let Some(c) = s.color {
        st = st.fg(match c {
            markup::Color::Red => Color::Red,
            markup::Color::Green => Color::Green,
            markup::Color::Yellow => Color::Yellow,
            markup::Color::Blue => Color::Blue,
            markup::Color::Magenta => Color::Magenta,
            markup::Color::Cyan => Color::Cyan,
            markup::Color::Gray => Color::DarkGray,
        });
    }
    st
}

// ------------------------------------------------------------------ wrap

/// Wrap lines to a width. jui wraps the lines itself, so that it knows the line count
/// (for the scroll limit) and the line of each selected item.
pub fn wrap_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    wrap_with_items(lines, &[], width).0
}

fn wrap_with_items(lines: Vec<Line<'static>>, items: &[usize], width: u16) -> (Vec<Line<'static>>, Vec<usize>) {
    let width = width.max(1) as usize;
    let mut out = Vec::with_capacity(lines.len());
    let mut new_items = Vec::with_capacity(items.len());
    for (i, line) in lines.into_iter().enumerate() {
        if items.contains(&i) {
            new_items.push(out.len());
        }
        out.extend(wrap_line(line, width));
    }
    (out, new_items)
}

fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line.width() <= width {
        return vec![line];
    }
    let line_style = line.style;
    // Split each span into words (each word keeps its trailing spaces).
    let mut words: Vec<(String, Style)> = Vec::new();
    for s in &line.spans {
        let mut cur = String::new();
        let mut in_space = false;
        for c in s.content.chars() {
            if c == ' ' {
                in_space = true;
            } else if in_space {
                words.push((std::mem::take(&mut cur), s.style));
                in_space = false;
            }
            cur.push(c);
        }
        if !cur.is_empty() {
            words.push((cur, s.style));
        }
    }
    let mut out = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0;
    let push_line = |out: &mut Vec<Line<'static>>, cur: &mut Vec<Span<'static>>| {
        out.push(Line::from(std::mem::take(cur)).style(line_style));
    };
    for (word, style) in words {
        let w = word.trim_end().width();
        if cur_w + w > width && cur_w > 0 {
            push_line(&mut out, &mut cur);
            cur_w = 0;
        }
        // Keep the indent of the first line. Continuation lines start with no spaces.
        let word = if cur_w == 0 && !out.is_empty() { word.trim_start().to_string() } else { word };
        if word.width() > width {
            // A word longer than the line: split it at the width.
            let mut piece = String::new();
            for c in word.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                if cur_w + piece.width() + cw > width && !(cur_w == 0 && piece.is_empty()) {
                    cur.push(Span::styled(std::mem::take(&mut piece), style));
                    push_line(&mut out, &mut cur);
                    cur_w = 0;
                }
                piece.push(c);
            }
            cur_w += piece.width();
            cur.push(Span::styled(piece, style));
            continue;
        }
        cur_w += word.width();
        cur.push(Span::styled(word, style));
    }
    if !cur.is_empty() {
        push_line(&mut out, &mut cur);
    }
    out
}

// ------------------------------------------------------------------ popups

fn draw_transition(f: &mut Frame, st: &State, area: Rect, p: &TransitionPopup) {
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Filter: ", Style::new().fg(Color::DarkGray)),
        Span::raw(p.filter.clone()),
        Span::styled("▏", Style::new().fg(Color::DarkGray)),
    ]));
    lines.push(Line::raw(""));
    match &p.list {
        None => lines.push(dim("Loading the Transitions…")),
        Some(ts) => {
            let shown = p.filtered();
            if shown.is_empty() {
                lines.push(dim("No match."));
            }
            for (n, &i) in shown.iter().enumerate() {
                let t = &ts[i];
                let on = n == p.selected;
                let mut spans = vec![
                    Span::raw(if on { "▶ " } else { "  " }),
                    Span::styled(t.to.clone(), status_style(&t.to, t.to_category)),
                ];
                if !t.name.eq_ignore_ascii_case(&t.to) {
                    spans.push(Span::styled(format!("  via {}", t.name), Style::new().fg(Color::DarkGray)));
                }
                let required: Vec<&str> = t.fields.iter().filter(|f| f.required && !f.has_default).map(|f| f.name.as_str()).collect();
                if !required.is_empty() {
                    spans.push(Span::styled(format!("  (needs {})", required.join(", ")), Style::new().fg(Color::Yellow)));
                }
                let mut line = Line::from(spans);
                if on {
                    line = line.style(Style::new().add_modifier(Modifier::REVERSED));
                }
                lines.push(line);
            }
        }
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.sending {
        lines.push(Line::styled(format!("{} Sending…", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    } else {
        lines.push(dim("Type to filter · ↑↓ select · Enter move · Esc close"));
    }
    let w = 60u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Move {} ", p.key)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_fields(f: &mut Frame, area: Rect, p: &FieldsPopup) {
    let mut lines = vec![Line::raw(format!("The Transition to {} needs these fields:", p.transition.to)), Line::raw("")];
    for (i, field) in p.fields.iter().enumerate() {
        let focus = i == p.index;
        let label = Style::new().add_modifier(if focus { Modifier::BOLD } else { Modifier::empty() });
        let mut spans = vec![Span::styled(format!("{}: ", field.name), label), Span::raw(p.inputs[i].clone())];
        if focus {
            spans.push(Span::styled("▏", Style::new().fg(Color::DarkGray)));
        } else if p.values[i].is_some() {
            spans.push(Span::styled(" ✓", Style::new().fg(Color::Green)));
        }
        lines.push(Line::from(spans));
        if focus && !field.allowed.is_empty() {
            let choices = p.choices();
            if choices.is_empty() {
                lines.push(Line::styled("  No match.", Style::new().fg(Color::DarkGray)));
            }
            let start = p.choice.saturating_sub(7);
            for (n, &c) in choices.iter().enumerate().skip(start).take(8) {
                let on = n == p.choice;
                let mut line = Line::raw(format!("  {}{}", if on { "▶ " } else { "  " }, field.allowed[c].name));
                if on {
                    line = line.style(Style::new().add_modifier(Modifier::REVERSED));
                }
                lines.push(line);
            }
        }
        if let Some(e) = &p.errors[i] {
            lines.push(Line::styled(format!("  {e}"), Style::new().fg(Color::Red)));
        }
    }
    if let Some(e) = &p.general {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    lines.push(if p.sending { Line::styled("Sending…", Style::new().fg(Color::Yellow)) } else { dim("Enter accept · Tab next field · Esc cancel") });
    let w = 64u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Move {} to {} ", p.key, p.transition.to)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

/// A short hint for an empty field of the create form.
fn create_hint(field: &crate::core::model::ScreenField) -> String {
    use crate::core::model::FieldKind;
    match (&field.kind, field.id.as_str()) {
        (_, "assignee") => "me, none, or a username".into(),
        (FieldKind::User, _) => "a username, or me".into(),
        (FieldKind::Labels, _) => "labels with commas, no spaces in a label".into(),
        (FieldKind::MultiChoice, _) => "type to filter · ↓ select · Enter add".into(),
        (FieldKind::Choice, _) => "type to filter · ↓ select · Enter take".into(),
        (FieldKind::Date, _) => "YYYY-MM-DD".into(),
        (FieldKind::Number, _) => "a number".into(),
        (FieldKind::Key, _) => "a Ticket key".into(),
        _ => String::new(),
    }
}

fn draw_create(f: &mut Frame, st: &State, area: Rect, p: &CreatePopup) {
    let w = 90u16.min(area.width.saturating_sub(4));
    let inner = w.saturating_sub(4);
    let label_w = p.fields.iter().map(|x| x.name.width()).max().unwrap_or(0).clamp(11, 24);
    let mut lines: Vec<Line<'static>> = Vec::new();
    // The first and the last line of the focused part, to scroll it into the view.
    let mut focus_lines = (0usize, 0usize);
    for (i, field) in p.fields.iter().enumerate() {
        let on = i == p.focus;
        let start = lines.len();
        let mut name = field.name.clone();
        if field.required {
            name.push('*');
        }
        let mut spans = vec![
            Span::raw(if on { "▶ " } else { "  " }),
            Span::styled(
                format!("{name:<label_w$} "),
                if on { Style::new().add_modifier(Modifier::BOLD) } else { Style::new().fg(Color::DarkGray) },
            ),
            Span::raw(p.inputs[i].clone()),
        ];
        if on && !p.sending {
            spans.push(Span::styled("▏", Style::new().fg(Color::DarkGray)));
        }
        if p.inputs[i].is_empty() && on {
            let hint = create_hint(field);
            if !hint.is_empty() {
                spans.push(Span::styled(format!(" {hint}"), Style::new().fg(Color::DarkGray)));
            }
        }
        lines.extend(wrap_lines(vec![Line::from(spans)], inner));
        if on && !field.allowed.is_empty() && (p.browsing || !p.filter_text().is_empty()) {
            let choices = p.choices();
            if choices.is_empty() {
                lines.push(dim("      No match."));
            }
            let start = p.choice.saturating_sub(5);
            for (n, &c) in choices.iter().enumerate().skip(start).take(6) {
                let sel = p.browsing && n == p.choice;
                let mut line = Line::raw(format!("    {}{}", if sel { "▶ " } else { "  " }, field.allowed[c].name));
                if sel {
                    line = line.style(Style::new().add_modifier(Modifier::REVERSED));
                }
                lines.push(line);
            }
            if choices.len() > start + 6 {
                lines.push(dim(format!("      … {} more", choices.len() - start - 6)));
            }
        }
        if let Some(e) = &p.errors[i] {
            lines.extend(wrap_lines(vec![Line::styled(format!("    {e}"), Style::new().fg(Color::Red))], inner));
        }
        if on {
            focus_lines = (start, lines.len());
        }
    }
    // The description box.
    lines.push(Line::raw(""));
    let on = p.on_description();
    let start = lines.len();
    lines.push(Line::from(vec![
        Span::raw(if on { "▶ " } else { "  " }),
        Span::styled(
            "Description",
            if on { Style::new().add_modifier(Modifier::BOLD) } else { Style::new().fg(Color::DarkGray) },
        ),
        Span::styled(" (Markdown, Enter new line)", Style::new().fg(Color::DarkGray)),
    ]));
    let box_style = if on { Style::new() } else { Style::new().fg(Color::DarkGray) };
    let mut desc: Vec<Line<'static>> = p.description.split('\n').map(|l| Line::styled(format!("  │ {l}"), box_style)).collect();
    if on
        && !p.sending
        && let Some(last) = desc.last_mut()
    {
        last.spans.push(Span::styled("▏", Style::new().fg(Color::DarkGray)));
    }
    if p.description.is_empty() && !on {
        desc = vec![Line::styled("  │", box_style)];
    }
    lines.extend(wrap_lines(desc, inner));
    if on {
        focus_lines = (start, lines.len());
    }
    if let Some(e) = &p.general {
        lines.push(Line::raw(""));
        lines.extend(wrap_lines(vec![Line::styled(e.clone(), Style::new().fg(Color::Red))], inner));
    }
    let footer = if p.sending {
        Line::styled(format!(" {} Creating… ", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow))
    } else {
        Line::raw(" Ctrl-S create · Tab next · Ctrl-E editor (all fields) · Esc cancel ")
    };
    // The borders and the top padding take 3 lines.
    let max_h = area.height.saturating_sub(2).max(6);
    let h = (lines.len() as u16 + 3).min(max_h);
    let visible = h.saturating_sub(3) as usize;
    let scroll = if focus_lines.1 > visible { (focus_lines.1 - visible).min(focus_lines.0) } else { 0 };
    let r = centered(area, w, h);
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .title(format!(" {} ", p.title))
        .title_bottom(footer.centered())
        .padding(Padding::new(1, 1, 1, 0));
    f.render_widget(Paragraph::new(lines).block(block).scroll((scroll as u16, 0)), r);
}

fn draw_diff(f: &mut Frame, area: Rect, p: &DiffPopup) {
    let mut lines = Vec::new();
    if let Some(c) = &p.conflict {
        lines.push(Line::styled(c.clone(), Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw(format!("{} changes (wiki markup):", p.subject)));
    if !p.fields.is_empty() {
        lines.push(dim(format!("The fields {} also change.", p.fields.join(", "))));
    }
    lines.push(Line::raw(""));
    for l in &p.lines {
        let style = match l.tag {
            '-' => Style::new().fg(Color::Red),
            '+' => Style::new().fg(Color::Green),
            _ => Style::new().fg(Color::DarkGray),
        };
        lines.push(Line::styled(format!("{}{}", l.tag, l.text), style));
    }
    for w in &p.warnings {
        lines.push(Line::styled(format!("warning: {w}"), Style::new().fg(Color::Yellow)));
    }
    let w = 100u16.min(area.width.saturating_sub(4));
    let body = wrap_lines(lines, w.saturating_sub(4));
    let max_h = area.height.saturating_sub(4).max(5);
    let h = (body.len() as u16 + 4).min(max_h);
    let r = centered(area, w, h);
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .title(format!(" Send this change to {}? ", p.key))
        .title_bottom(Line::from(" y send · n edit again · Esc keep a Draft · j/k scroll ").centered())
        .padding(Padding::horizontal(1));
    let scroll = p.scroll.min((body.len() as u16).saturating_sub(h.saturating_sub(2)));
    f.render_widget(Paragraph::new(body).block(block).scroll((scroll, 0)), r);
}

fn draw_comment(f: &mut Frame, st: &State, area: Rect, p: &CommentPopup) {
    let mut lines = vec![Line::from(vec![
        Span::raw(p.input.clone()),
        Span::styled("▏", Style::new().fg(Color::DarkGray)),
    ])];
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.sending {
        lines.push(Line::styled(format!("{} Sending…", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    } else {
        lines.push(dim("Markdown · @name mentions · Enter send · Ctrl-E editor · Esc close"));
    }
    let w = 80u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Comment on {} ", p.key)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_user(f: &mut Frame, st: &State, area: Rect, p: &UserPopup) {
    let mut lines = vec![Line::from(vec![
        Span::styled("User: ", Style::new().fg(Color::DarkGray)),
        Span::raw(p.input.clone()),
        Span::styled("▏", Style::new().fg(Color::DarkGray)),
    ])];
    lines.push(Line::raw(""));
    let rows = p.rows(&st.recent_users);
    let current = st.cache.get(&p.key).and_then(|t| t.assignee.as_ref()).map(|u| u.name.clone());
    for (i, r) in rows.iter().enumerate() {
        let on = i == p.selected;
        let mut spans = vec![Span::raw(if on { "▶ " } else { "  " })];
        match r {
            PickRow::Me => spans.push(Span::styled("me", Style::new().fg(Color::Cyan))),
            PickRow::Nobody => spans.push(Span::styled("none (unassigned)", Style::new().fg(Color::Cyan))),
            PickRow::User(u) => {
                spans.push(Span::raw(u.display_name.clone()));
                spans.push(Span::styled(format!("  {}", u.name), Style::new().fg(Color::DarkGray)));
                if current.as_deref() == Some(u.name.as_str()) {
                    spans.push(Span::styled("  (now)", Style::new().fg(Color::Yellow)));
                }
            }
        }
        let mut line = Line::from(spans);
        if on {
            line = line.style(Style::new().add_modifier(Modifier::REVERSED));
        }
        lines.push(line);
    }
    if p.searching {
        lines.push(dim(format!("{} Searching…", SPINNER[st.spinner % SPINNER.len()])));
    } else if p.searches() && p.results.as_ref().is_some_and(Vec::is_empty) && rows.is_empty() {
        lines.push(dim("No match."));
    } else if !p.searches() && !p.input.trim().is_empty() && rows.is_empty() {
        lines.push(dim(format!("Type {USER_SEARCH_MIN} characters or more to search.")));
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.sending {
        lines.push(Line::styled(format!("{} Sending…", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    } else {
        lines.push(dim("Type to search · ↑↓ select · Enter assign · Esc close"));
    }
    let w = 64u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Assign {} ", p.key)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_filters(f: &mut Frame, st: &State, area: Rect, p: &FilterPopup) {
    let mut lines = vec![Line::from(vec![
        Span::styled("Filter: ", Style::new().fg(Color::DarkGray)),
        Span::raw(p.input.clone()),
        Span::styled("▏", Style::new().fg(Color::DarkGray)),
    ])];
    lines.push(Line::raw(""));
    let w = 80u16.min(area.width.saturating_sub(4));
    let inner = w.saturating_sub(4) as usize;
    match &p.list {
        None => lines.push(dim(format!("{} Getting your favourite filters…", SPINNER[st.spinner % SPINNER.len()]))),
        Some(fs) => {
            let shown = p.shown();
            if fs.is_empty() {
                lines.push(dim("You have no favourite filters."));
                lines.push(dim("Type a filter ID (the number in the filter URL) and push Enter."));
            } else if shown.is_empty() {
                lines.push(dim("No match."));
            }
            // Show at most 12 rows, and keep the selected row visible.
            let start = p.selected.saturating_sub(11);
            for (n, &i) in shown.iter().enumerate().skip(start).take(12) {
                let fl = &fs[i];
                let on = n == p.selected;
                let id_w = fl.id.width() + 2;
                let name_w = fl.name.width();
                let jql_room = inner.saturating_sub(id_w + name_w + 4);
                let mut jql: String = fl.jql.replace('\n', " ");
                if jql.width() > jql_room {
                    jql = jql.chars().take(jql_room.saturating_sub(1)).collect::<String>() + "…";
                }
                let spans = vec![
                    Span::raw(if on { "▶ " } else { "  " }),
                    Span::styled(format!("{:<id_w$}", fl.id), Style::new().fg(Color::Cyan)),
                    Span::raw(fl.name.clone()),
                    Span::styled(if jql_room > 3 { format!("  {jql}") } else { String::new() }, Style::new().fg(Color::DarkGray)),
                ];
                let mut line = Line::from(spans);
                if on {
                    line = line.style(Style::new().add_modifier(Modifier::REVERSED));
                }
                lines.push(line);
            }
        }
    }
    if let Some(id) = &p.loading_id {
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("{} Getting filter {id}…", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    lines.push(dim("Type to filter, or a filter ID · ↑↓ select · Enter run · Esc close"));
    let lines = wrap_lines(lines, inner as u16);
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(" Favourite filters ").padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_delete(f: &mut Frame, area: Rect, what: DeleteKind, key: &str, id: &str, preview: &str, sending: bool) {
    let lines = vec![
        Line::raw(preview.to_string()),
        Line::raw(""),
        Line::styled("A delete cannot be reversed.", Style::new().fg(Color::Yellow)),
        Line::raw(""),
        if sending { Line::styled("Deleting…", Style::new().fg(Color::Yellow)) } else { dim("y delete · n keep") },
    ];
    let w = 70u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Delete {} {id} on {key}? ", what.name())).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_link(f: &mut Frame, st: &State, area: Rect, p: &LinkPopup) {
    let focus = |on: bool| if on { Style::new().add_modifier(Modifier::BOLD) } else { Style::new().fg(Color::DarkGray) };
    let mut lines = vec![Line::from(vec![
        Span::styled("Relation: ", focus(!p.on_target)),
        Span::raw(p.filter.clone()),
        Span::styled(if p.on_target { "" } else { "▏" }, Style::new().fg(Color::DarkGray)),
    ])];
    lines.push(Line::raw(""));
    let shown = st.link_types.as_deref().map(|t| p.shown(t));
    match &shown {
        None if p.error.is_none() => lines.push(dim(format!("{} Getting the Link types…", SPINNER[st.spinner % SPINNER.len()]))),
        None => {}
        Some(rs) if rs.is_empty() => lines.push(dim("No match.")),
        Some(rs) => {
            // Show at most 10 rows, and keep the selected row visible.
            let start = p.selected.saturating_sub(9);
            for (i, r) in rs.iter().enumerate().skip(start).take(10) {
                let on = i == p.selected;
                let mut line = Line::from(vec![
                    Span::raw(if on { "▶ " } else { "  " }),
                    Span::raw(format!("{} {} ", p.key, r.text())),
                    Span::styled("…", Style::new().fg(Color::Cyan)),
                    Span::styled(format!("  ({})", r.link_type.name), Style::new().fg(Color::DarkGray)),
                ]);
                if on {
                    line = line.style(if p.on_target { Style::new().add_modifier(Modifier::BOLD) } else { Style::new().add_modifier(Modifier::REVERSED) });
                }
                lines.push(line);
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("Ticket:   ", focus(p.on_target)),
        Span::raw(p.target.clone()),
        Span::styled(if p.on_target { "▏" } else { "" }, Style::new().fg(Color::DarkGray)),
    ]));
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.sending {
        lines.push(Line::styled(format!("{} Sending…", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    } else if p.on_target {
        lines.push(dim("Type the key or number · Enter add · Tab relation · Esc close"));
    } else {
        lines.push(dim("Type to filter · ↑↓ select · Enter or Tab next · Esc close"));
    }
    let w = 70u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Add a Link to {} ", p.key)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_worklog(f: &mut Frame, st: &State, area: Rect, p: &WorklogPopup) {
    let hints = ["1h30m, 90m, 1.5h, 2d", "now, yesterday, 10:00, 2026-10-19 10:00", "Markdown, one line"];
    let mut lines = Vec::new();
    for (i, name) in WORKLOG_FIELDS.iter().enumerate() {
        let on = i == p.focus;
        let label = format!("{name:<11} ");
        let mut spans = vec![
            Span::raw(if on { "▶ " } else { "  " }),
            Span::styled(label, if on { Style::new().add_modifier(Modifier::BOLD) } else { Style::new().fg(Color::DarkGray) }),
            Span::raw(p.inputs[i].clone()),
        ];
        if on {
            spans.push(Span::styled("▏", Style::new().fg(Color::DarkGray)));
        }
        if p.inputs[i].is_empty() {
            spans.push(Span::styled(format!(" {}", if i == 1 { "now" } else { hints[i] }), Style::new().fg(Color::DarkGray)));
        }
        lines.push(Line::from(spans));
        if let Some(e) = &p.errors[i] {
            lines.push(Line::styled(format!("    {e}"), Style::new().fg(Color::Red)));
        }
    }
    if let Some(e) = &p.general {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.sending {
        lines.push(Line::styled(format!("{} Sending…", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    } else {
        lines.push(dim("Tab next field · Enter add · Ctrl-E editor (estimate) · Esc close"));
    }
    let w = 72u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Add a Worklog to {} ", p.key)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_upload(f: &mut Frame, st: &State, area: Rect, p: &UploadPopup) {
    let mut lines = vec![Line::from(vec![
        Span::styled("File: ", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(p.input.clone()),
        Span::styled(if p.sending { "" } else { "▏" }, Style::new().fg(Color::DarkGray)),
    ])];
    if p.input.is_empty() {
        lines.push(dim("Type or paste a path, for example ~/Desktop/screen.png. A dragged file also works."));
    }
    if let Some(e) = &p.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.clone(), Style::new().fg(Color::Red)));
    }
    lines.push(Line::raw(""));
    if p.sending {
        lines.push(Line::styled(format!("{} Uploading…", SPINNER[st.spinner % SPINNER.len()]), Style::new().fg(Color::Yellow)));
    } else {
        lines.push(dim("Enter upload · Ctrl-U clear · Esc close"));
    }
    let w = 80u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Upload a file to {} ", p.key)).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn draw_conflict(f: &mut Frame, area: Rect, key: &str, message: &str) {
    let lines = vec![
        Line::styled(message.to_string(), Style::new().fg(Color::Yellow)),
        Line::raw(""),
        Line::raw("Send your changes?"),
        Line::raw(""),
        dim("y send · n keep a Draft and do not send"),
    ];
    let w = 70u16.min(area.width.saturating_sub(4));
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(format!(" Edit {key} ")).padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_help(f: &mut Frame, area: Rect, scroll: u16) {
    let mut lines = Vec::new();
    let mut last_place = None;
    for b in KEYS {
        if last_place != Some(b.place) {
            let title = match b.place {
                Where::Zoom => Some("In the zoom only"),
                Where::Board => Some("On the Boards tab"),
                Where::Sprint => Some("On the Sprint tab"),
                Where::Settings => Some("On the Settings page"),
                Where::Both => None,
            };
            if let Some(t) = title {
                lines.push(Line::raw(""));
                lines.push(Line::styled(t, Style::new().add_modifier(Modifier::BOLD)));
            }
            last_place = Some(b.place);
        }
        let mut spans = vec![Span::styled(format!("{:<11}", b.label), Style::new().fg(Color::Cyan)), Span::raw(b.help)];
        if !b.built {
            spans.push(Span::styled("  (later)", Style::new().fg(Color::DarkGray)));
        }
        lines.push(Line::from(spans));
    }
    let r = centered(area, 72, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let p = Paragraph::new(lines).block(Block::bordered().title(" Keys (any key closes) ")).scroll((scroll, 0));
    f.render_widget(p, r);
}

fn draw_errors(f: &mut Frame, st: &State, area: Rect, scroll: u16) {
    let mut lines = Vec::new();
    for (i, e) in st.errors.iter().enumerate() {
        if i > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(vec![
            Span::styled(format!("{}. ", i + 1), Style::new().fg(Color::DarkGray)),
            Span::styled(e.context.clone(), Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(format!("  [{}]", e.kind), Style::new().fg(Color::DarkGray)),
        ]));
        lines.push(Line::styled(format!("   {}", e.message), Style::new().fg(Color::Red)));
        if let Some(h) = &e.hint {
            lines.push(Line::raw(format!("   hint: {h}")));
        }
    }
    let w = area.width.saturating_sub(10).min(100);
    let lines = wrap_lines(lines, w.saturating_sub(2));
    let r = centered(area, w, (lines.len() as u16 + 2).min(area.height.saturating_sub(4)));
    f.render_widget(Clear, r);
    let p = Paragraph::new(lines).block(Block::bordered().title(" Last errors (any key closes) ")).scroll((scroll, 0));
    f.render_widget(p, r);
}

fn draw_fatal(f: &mut Frame, area: Rect, message: &str, hint: Option<&str>) {
    let mut lines = vec![Line::raw(""), Line::styled(message.to_string(), Style::new().fg(Color::Red).add_modifier(Modifier::BOLD))];
    if let Some(h) = hint {
        lines.push(Line::raw(""));
        lines.push(Line::raw(h.to_string()));
    }
    lines.push(Line::raw(""));
    lines.push(dim("Push q to quit."));
    let w = area.width.saturating_sub(10).min(80);
    let lines = wrap_lines(lines, w.saturating_sub(4));
    let r = centered(area, w, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let block = Block::bordered().title(" jui cannot start ").padding(Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), r);
}
