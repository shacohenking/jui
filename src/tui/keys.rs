//! The key table: one table for all keys. `update` uses it to find the command for a key,
//! and the help popup shows it. See ticket 06 "jui TUI layout and navigation", part 3.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A command that a key starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Down,
    Up,
    First,
    Last,
    ScrollDown,
    ScrollUp,
    PageDown,
    PageUp,
    Open,
    Back,
    NextTab,
    PrevTab,
    Tab(usize),
    SearchFocus,
    Filters,
    Refresh,
    Help,
    Errors,
    Quit,
    Exit,
    Browser,
    Transition,
    Comment,
    Assign,
    Edit,
    Create,
    CreateSubtask,
    EditWiki,
    Watch,
    Copy,
    AddLink,
    AddWorklog,
    MoveSprint,
    SetEpic,
    Upload,
    Delete,
    Left,
    Right,
    PrevBoard,
    NextBoard,
    PickBoards,
    RemoveBoard,
    OnlyMine,
    PrevSection,
    NextSection,
    ResetSetting,
    EditConfig,
}

/// Where a key works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    /// In the tabs and in the zoom.
    Both,
    /// Only in the zoom.
    Zoom,
    /// Only on the Boards tab (not in the zoom).
    Board,
    /// Only on the Sprint tab (not in the zoom).
    Sprint,
    /// Only on the Settings page.
    Settings,
}

/// The view that has the focus when the user pushes a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctx {
    Main,
    Zoom,
    /// The columns of the Boards tab.
    Board,
    /// The list of the Sprint tab.
    Sprint,
    /// The Settings page.
    Settings,
}

impl Where {
    /// True when a binding of this place works in the view.
    pub fn works_in(self, ctx: Ctx) -> bool {
        match self {
            Where::Both => true,
            Where::Zoom => ctx == Ctx::Zoom,
            Where::Board => ctx == Ctx::Board,
            Where::Sprint => ctx == Ctx::Sprint,
            Where::Settings => ctx == Ctx::Settings,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
}

const fn ch(c: char) -> Key {
    Key { code: KeyCode::Char(c), ctrl: false }
}

const fn ctrl(c: char) -> Key {
    Key { code: KeyCode::Char(c), ctrl: true }
}

const fn code(code: KeyCode) -> Key {
    Key { code, ctrl: false }
}

pub struct Binding {
    pub keys: &'static [Key],
    /// The key text in the help popup.
    pub label: &'static str,
    pub cmd: Cmd,
    pub place: Where,
    pub help: &'static str,
    /// False when a later build ticket makes the command.
    pub built: bool,
}

const fn b(keys: &'static [Key], label: &'static str, cmd: Cmd, help: &'static str) -> Binding {
    Binding { keys, label, cmd, place: Where::Both, help, built: true }
}

const fn zoom(keys: &'static [Key], label: &'static str, cmd: Cmd, help: &'static str) -> Binding {
    Binding { keys, label, cmd, place: Where::Zoom, help, built: true }
}

const fn board(keys: &'static [Key], label: &'static str, cmd: Cmd, help: &'static str) -> Binding {
    Binding { keys, label, cmd, place: Where::Board, help, built: true }
}

const fn sprint(keys: &'static [Key], label: &'static str, cmd: Cmd, help: &'static str) -> Binding {
    Binding { keys, label, cmd, place: Where::Sprint, help, built: true }
}

const fn settings(keys: &'static [Key], label: &'static str, cmd: Cmd, help: &'static str) -> Binding {
    Binding { keys, label, cmd, place: Where::Settings, help, built: true }
}

pub const KEYS: &[Binding] = &[
    b(&[ch('j'), code(KeyCode::Down)], "j ↓", Cmd::Down, "Move down"),
    b(&[ch('k'), code(KeyCode::Up)], "k ↑", Cmd::Up, "Move up"),
    b(&[ch('g'), code(KeyCode::Home)], "g", Cmd::First, "First row"),
    b(&[ch('G'), code(KeyCode::End)], "G", Cmd::Last, "Last loaded row (gets the next page)"),
    b(&[ch('J')], "J", Cmd::ScrollDown, "Scroll the preview or the zoom down"),
    b(&[ch('K')], "K", Cmd::ScrollUp, "Scroll the preview or the zoom up"),
    b(&[ctrl('d'), code(KeyCode::PageDown)], "Ctrl-d", Cmd::PageDown, "Scroll half a page down"),
    b(&[ctrl('u'), code(KeyCode::PageUp)], "Ctrl-u", Cmd::PageUp, "Scroll half a page up"),
    b(&[code(KeyCode::Enter)], "Enter", Cmd::Open, "Open the zoom (in the zoom: open the Sub-task or Link)"),
    b(&[code(KeyCode::Esc)], "Esc", Cmd::Back, "Go back one step, close a popup"),
    b(&[code(KeyCode::Tab)], "Tab", Cmd::NextTab, "Next tab"),
    b(&[code(KeyCode::BackTab)], "Shift-Tab", Cmd::PrevTab, "Previous tab"),
    b(&[ch('1')], "1", Cmd::Tab(0), "My Tickets"),
    b(&[ch('2')], "2", Cmd::Tab(1), "Boards"),
    b(&[ch('3')], "3", Cmd::Tab(2), "Sprint"),
    b(&[ch('4')], "4", Cmd::Tab(3), "Epics"),
    b(&[ch('5')], "5", Cmd::Tab(4), "Search"),
    b(&[ch('6')], "6", Cmd::Tab(5), "Settings"),
    b(&[ch('r')], "r", Cmd::Refresh, "Get the data again from Jira"),
    b(&[ch('t')], "t", Cmd::Transition, "Transition (move to a different Status)"),
    b(&[ch('o')], "o", Cmd::Browser, "Open the Ticket in the browser"),
    b(&[ch('?')], "?", Cmd::Help, "This help"),
    b(&[ch('!')], "!", Cmd::Errors, "The last errors, with details"),
    b(&[ch('q')], "q", Cmd::Quit, "Quit (in the zoom: go back)"),
    b(&[ctrl('c')], "Ctrl-c", Cmd::Exit, "Quit from all views"),
    b(&[ch('/')], "/", Cmd::SearchFocus, "Search with JQL (the Search tab, ↑↓ history)"),
    b(&[ctrl('f')], "Ctrl-f", Cmd::Filters, "Run a favourite filter (or a filter ID)"),
    b(&[ch('c')], "c", Cmd::Comment, "Add a comment (Ctrl-E in the popup opens $EDITOR)"),
    b(&[ch('a')], "a", Cmd::Assign, "Assign (user picker: me, none, or a search)"),
    b(&[ch('e')], "e", Cmd::Edit, "Edit the Ticket, the comment, or the Worklog in $EDITOR"),
    b(&[ch('n')], "n", Cmd::Create, "Create a Ticket in a form (in an Epic zoom: in that Epic)"),
    b(&[ch('w')], "w", Cmd::Watch, "Watch or unwatch"),
    b(&[ch('y')], "y", Cmd::Copy, "Copy the key to the clipboard"),
    zoom(&[ch('N')], "N", Cmd::CreateSubtask, "Create a Sub-task of this Ticket in a form"),
    zoom(&[ch('W')], "W", Cmd::EditWiki, "Edit the description or the comment as raw wiki"),
    zoom(&[ch('l')], "l", Cmd::AddLink, "Add a Link (select the relation, then type the key)"),
    zoom(&[ch('L')], "L", Cmd::AddWorklog, "Add a Worklog (Ctrl-E in the popup opens $EDITOR)"),
    zoom(&[ch('s')], "s", Cmd::MoveSprint, "Move to a Sprint or to the backlog"),
    zoom(&[ch('E')], "E", Cmd::SetEpic, "Set or remove the Epic"),
    zoom(&[ch('u')], "u", Cmd::Upload, "Upload a file (type or paste the path)"),
    zoom(&[ch('d')], "d", Cmd::Delete, "Delete the selected comment, Worklog, Link, or attachment"),
    board(&[ch('h'), code(KeyCode::Left)], "h ←", Cmd::Left, "Column to the left"),
    board(&[ch('l'), code(KeyCode::Right)], "l →", Cmd::Right, "Column to the right"),
    board(&[ch('[')], "[", Cmd::PrevBoard, "Previous Board in My Boards"),
    board(&[ch(']')], "]", Cmd::NextBoard, "Next Board in My Boards"),
    board(&[ch('+')], "+", Cmd::PickBoards, "Board picker: add or remove Boards in My Boards"),
    board(&[ch('x')], "x", Cmd::RemoveBoard, "Remove this Board from My Boards"),
    board(&[ch('m')], "m", Cmd::OnlyMine, "Show only my Tickets, or all Tickets (saved in the Settings)"),
    sprint(&[ch('[')], "[", Cmd::PrevSection, "Previous active Sprint, or the backlog"),
    sprint(&[ch(']')], "]", Cmd::NextSection, "Next active Sprint, or the backlog"),
    sprint(&[ch('m')], "m", Cmd::OnlyMine, "Show only my Tickets, or all Tickets (saved in the Settings)"),
    settings(&[ch('d')], "d", Cmd::ResetSetting, "Set the Setting back to its default value (on a Board: remove it)"),
    settings(&[ch('E')], "E", Cmd::EditConfig, "Open the config file in $EDITOR"),
    // Help only: `K` and `J` are ScrollUp and ScrollDown (above). The Settings page has no
    // preview, so there they change the order of My Boards.
    settings(&[], "K J", Cmd::ScrollUp, "Move the selected Board up or down in My Boards"),
];

fn matches(k: &Key, ev: &KeyEvent) -> bool {
    let ev_ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    if k.ctrl != ev_ctrl {
        return false;
    }
    match (k.code, ev.code) {
        // Upper-case letters come with SHIFT. The character already tells the case.
        (KeyCode::Char(a), KeyCode::Char(b)) => a == b,
        (a, b) => a == b,
    }
}

/// Find the binding for a key in a view.
pub fn lookup(ev: &KeyEvent, ctx: Ctx) -> Option<&'static Binding> {
    KEYS.iter().filter(|b| b.place.works_in(ctx)).find(|b| b.keys.iter().any(|k| matches(k, ev)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(c, m)
    }

    #[test]
    fn letters_ignore_shift() {
        let b = lookup(&key(KeyCode::Char('J'), KeyModifiers::SHIFT), Ctx::Main).unwrap();
        assert_eq!(b.cmd, Cmd::ScrollDown);
    }

    #[test]
    fn ctrl_keys_need_ctrl() {
        assert_eq!(lookup(&key(KeyCode::Char('d'), KeyModifiers::CONTROL), Ctx::Main).unwrap().cmd, Cmd::PageDown);
        assert!(lookup(&key(KeyCode::Char('d'), KeyModifiers::NONE), Ctx::Main).is_none());
        assert_eq!(lookup(&key(KeyCode::Char('d'), KeyModifiers::NONE), Ctx::Zoom).unwrap().cmd, Cmd::Delete);
    }

    #[test]
    fn board_keys_only_on_the_boards_tab() {
        let l = key(KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(lookup(&l, Ctx::Board).unwrap().cmd, Cmd::Right);
        assert_eq!(lookup(&l, Ctx::Zoom).unwrap().cmd, Cmd::AddLink);
        assert!(lookup(&key(KeyCode::Char('x'), KeyModifiers::NONE), Ctx::Main).is_none());
        assert_eq!(lookup(&key(KeyCode::Char('j'), KeyModifiers::NONE), Ctx::Board).unwrap().cmd, Cmd::Down);
        let m = key(KeyCode::Char('m'), KeyModifiers::NONE);
        assert_eq!(lookup(&m, Ctx::Board).unwrap().cmd, Cmd::OnlyMine);
        assert_eq!(lookup(&m, Ctx::Sprint).unwrap().cmd, Cmd::OnlyMine);
        assert!(lookup(&m, Ctx::Main).is_none());
        assert!(lookup(&m, Ctx::Zoom).is_none());
    }

    #[test]
    fn sprint_keys_only_on_the_sprint_tab() {
        let k = key(KeyCode::Char(']'), KeyModifiers::NONE);
        assert_eq!(lookup(&k, Ctx::Sprint).unwrap().cmd, Cmd::NextSection);
        assert_eq!(lookup(&k, Ctx::Board).unwrap().cmd, Cmd::NextBoard);
        assert!(lookup(&k, Ctx::Main).is_none());
        let s = key(KeyCode::Char('s'), KeyModifiers::NONE);
        assert!(lookup(&s, Ctx::Sprint).is_none(), "s works only in the zoom");
        assert_eq!(lookup(&s, Ctx::Zoom).unwrap().cmd, Cmd::MoveSprint);
        assert!(lookup(&key(KeyCode::Char('E'), KeyModifiers::SHIFT), Ctx::Zoom).unwrap().built);
    }

    #[test]
    fn settings_keys_only_on_the_settings_page() {
        let d = key(KeyCode::Char('d'), KeyModifiers::NONE);
        assert_eq!(lookup(&d, Ctx::Settings).unwrap().cmd, Cmd::ResetSetting);
        assert!(lookup(&d, Ctx::Main).is_none());
        let e = key(KeyCode::Char('E'), KeyModifiers::SHIFT);
        assert_eq!(lookup(&e, Ctx::Settings).unwrap().cmd, Cmd::EditConfig);
        assert_eq!(lookup(&e, Ctx::Zoom).unwrap().cmd, Cmd::SetEpic);
        let j = key(KeyCode::Char('J'), KeyModifiers::SHIFT);
        assert_eq!(lookup(&j, Ctx::Settings).unwrap().cmd, Cmd::ScrollDown);
    }

    #[test]
    fn zoom_keys_only_in_zoom() {
        assert!(lookup(&key(KeyCode::Char('l'), KeyModifiers::NONE), Ctx::Main).is_none());
        assert_eq!(lookup(&key(KeyCode::Char('l'), KeyModifiers::NONE), Ctx::Zoom).unwrap().cmd, Cmd::AddLink);
    }

    #[test]
    fn no_key_is_in_the_table_two_times() {
        for ctx in [Ctx::Main, Ctx::Zoom, Ctx::Board, Ctx::Sprint, Ctx::Settings] {
            let mut seen = Vec::new();
            for b in KEYS.iter().filter(|b| b.place.works_in(ctx)) {
                for k in b.keys {
                    let id = (format!("{:?}", k.code), k.ctrl);
                    assert!(!seen.contains(&id), "{id:?} is in the table two times");
                    seen.push(id);
                }
            }
        }
    }
}
