//! Command-line arguments (clap derive).

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "jui", version, about = "A CLI and TUI client for Jira Data Center", propagate_version = true)]
pub struct Cli {
    /// Show HTTP requests on stderr. -vv also shows the bodies.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// When to use colors.
    #[arg(long, global = true, value_name = "WHEN")]
    pub color: Option<ColorArg>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ColorArg {
    Auto,
    Always,
    Never,
}

impl Cli {
    /// True when the command asked for `--json`, so errors are also JSON.
    pub fn json_errors(&self) -> bool {
        match &self.command {
            Some(Command::Ls(a)) | Some(Command::Search { list: a, .. }) => a.json,
            Some(Command::Ticket { cmd: TicketCmd::List(a) }) => a.json,
            Some(Command::View(v)) | Some(Command::Ticket { cmd: TicketCmd::View(v) }) => v.json || v.raw,
            Some(Command::Config { cmd: ConfigCmd::Show { json } }) => *json,
            Some(Command::Mv(m)) | Some(Command::Ticket { cmd: TicketCmd::Move(m) }) => m.json,
            Some(Command::New(a)) | Some(Command::Ticket { cmd: TicketCmd::Create(a) }) => a.common.json,
            Some(Command::Edit(a)) | Some(Command::Ticket { cmd: TicketCmd::Edit(a) }) => a.common.json,
            Some(Command::Comment { cmd: CommentCmd::List { json, .. } }) => *json,
            Some(Command::Comment { cmd: CommentCmd::Add(a) }) => a.json,
            Some(Command::Comment { cmd: CommentCmd::Edit(a) }) => a.json,
            Some(Command::Assign(a)) | Some(Command::Ticket { cmd: TicketCmd::Assign(a) }) => a.json,
            Some(Command::Ticket { cmd: TicketCmd::Watchers { json, .. } }) => *json,
            Some(Command::Filter { cmd: FilterCmd::List { json } | FilterCmd::View { json, .. } }) => *json,
            Some(Command::Filter { cmd: FilterCmd::Run { list, .. } }) => list.json,
            Some(Command::Board {
                cmd: BoardCmd::List { json, .. } | BoardCmd::View { json, .. } | BoardCmd::Mine { json },
            }) => *json,
            Some(Command::Sprint { cmd: SprintCmd::List { json, .. } }) => *json,
            Some(Command::Sprint { cmd: SprintCmd::Active { list, .. } | SprintCmd::View { list, .. } }) => list.json,
            Some(Command::Epic { cmd: EpicCmd::List { json, .. } | EpicCmd::Mine { json } }) => *json,
            Some(Command::Epic { cmd: EpicCmd::Tickets { list, .. } }) => list.json,
            Some(Command::Subtask { cmd: SubtaskCmd::List { json, .. } }) => *json,
            Some(Command::Subtask { cmd: SubtaskCmd::Create { common, .. } }) => common.json,
            Some(Command::Link { cmd: LinkCmd::List { json, .. } | LinkCmd::Types { json } }) => *json,
            Some(Command::Worklog { cmd: WorklogCmd::List { json, .. } }) => *json,
            Some(Command::Worklog { cmd: WorklogCmd::Add { flags, .. } | WorklogCmd::Edit { flags, .. } }) => flags.json,
            Some(Command::Attachment { cmd: AttachmentCmd::List { json, .. } | AttachmentCmd::Upload { json, .. } }) => *json,
            _ => false,
        }
    }

    pub fn color_choice(&self) -> Option<ColorArg> {
        self.color
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Start the TUI (also: `jui` with no arguments).
    Tui,

    /// List My Tickets (same as `jui ticket list`).
    Ls(ListArgs),

    /// Search Tickets with JQL.
    Search {
        /// The JQL query.
        jql: String,
        #[command(flatten)]
        list: ListArgs,
    },

    /// Show one Ticket (also: `jui <key>`).
    View(ViewArgs),

    /// Move a Ticket to a different Status (same as `jui ticket move`).
    Mv(MoveArgs),

    /// Create a Ticket (same as `jui ticket create`). With no -s, jui opens a Template in the editor.
    New(NewArgs),

    /// Edit a Ticket (same as `jui ticket edit`). With no field flags, jui opens a Template in the editor.
    Edit(EditArgs),

    /// Open a Ticket in the browser.
    Open {
        /// The Ticket key, for example BCE-123 or 123.
        key: String,
        /// Print the URL; do not open the browser.
        #[arg(long)]
        print: bool,
    },

    /// Set the assignee of a Ticket (same as `jui ticket assign`).
    Assign(AssignArgs),

    /// Ticket commands.
    Ticket {
        #[command(subcommand)]
        cmd: TicketCmd,
    },

    /// Saved filters: list the favourite filters, show one, run one.
    Filter {
        #[command(subcommand)]
        cmd: FilterCmd,
    },

    /// Boards and My Boards: list, view, mine, add, remove.
    Board {
        #[command(subcommand)]
        cmd: BoardCmd,
    },

    /// Sprints: list, active, view, add, backlog.
    Sprint {
        #[command(subcommand)]
        cmd: SprintCmd,
    },

    /// Epics: list, tickets, set, unset.
    Epic {
        #[command(subcommand)]
        cmd: EpicCmd,
    },

    /// Comment commands: list, add, edit, delete.
    Comment {
        #[command(subcommand)]
        cmd: CommentCmd,
    },

    /// Sub-tasks: list, create.
    Subtask {
        #[command(subcommand)]
        cmd: SubtaskCmd,
    },

    /// Links between Tickets: list, add, delete, types.
    Link {
        #[command(subcommand)]
        cmd: LinkCmd,
    },

    /// Worklogs: list, add, edit, delete.
    Worklog {
        #[command(subcommand)]
        cmd: WorklogCmd,
    },

    /// Attachments: list, upload, download, delete.
    Attachment {
        #[command(subcommand)]
        cmd: AttachmentCmd,
    },

    /// Read and change the Settings.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },

    /// Write a shell completion script to stdout.
    ///
    /// Examples:
    ///   zsh:  jui completions zsh > "${fpath[1]}/_jui"
    ///   bash: jui completions bash > ~/.local/share/bash-completion/completions/jui
    ///   fish: jui completions fish > ~/.config/fish/completions/jui.fish
    #[command(verbatim_doc_comment)]
    Completions {
        /// The shell.
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TicketCmd {
    /// List My Tickets.
    List(ListArgs),
    /// Show one Ticket.
    View(ViewArgs),
    /// Move a Ticket to a different Status.
    Move(MoveArgs),
    /// Create a Ticket.
    Create(NewArgs),
    /// Edit a Ticket.
    Edit(EditArgs),
    /// Open a Ticket in the browser.
    Open {
        key: String,
        #[arg(long)]
        print: bool,
    },
    /// Set the assignee: a username, a part of a name, `me`, or `none`.
    Assign(AssignArgs),
    /// Watch a Ticket. With no user, you watch it.
    Watch(WatchArgs),
    /// Stop watching a Ticket. With no user, you stop watching it.
    Unwatch(WatchArgs),
    /// List the watchers of a Ticket.
    Watchers {
        /// The Ticket key, for example BCE-123 or 123.
        key: String,
        /// Write the watchers as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum FilterCmd {
    /// List your favourite filters. Jira Data Center cannot list other filters.
    #[command(visible_alias = "ls")]
    List {
        /// Write the filters as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show one filter: the name, the owner, the JQL, and the URL.
    View {
        /// The filter ID (any filter that you can see), or a part of the name of a favourite filter.
        filter: String,
        /// Write the filter as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run a filter and list its Tickets.
    Run {
        /// The filter ID (any filter that you can see), or a part of the name of a favourite filter.
        filter: String,
        #[command(flatten)]
        list: ListArgs,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BoardType {
    Scrum,
    Kanban,
}

#[derive(Debug, Clone, Subcommand)]
pub enum BoardCmd {
    /// Search all Boards that you can see. `*` marks the Boards in My Boards.
    #[command(visible_alias = "ls")]
    List {
        /// Only the Boards of this project.
        #[arg(long, short = 'p', value_name = "KEY")]
        project: Option<String>,
        /// Only the Boards whose name contains this text.
        #[arg(long, short = 'n', value_name = "TEXT")]
        name: Option<String>,
        /// Only Scrum or only Kanban Boards.
        #[arg(long = "type", short = 't', value_name = "TYPE")]
        kind: Option<BoardType>,
        /// The maximum number of Boards.
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u64).range(1..))]
        limit: u64,
        /// Get all Boards (all pages).
        #[arg(long, conflicts_with = "limit")]
        all: bool,
        /// Write the Boards as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the columns and the Tickets of a Board: the active Sprints of a Scrum Board, or
    /// the Tickets of a Kanban Board (Done: the last 14 days).
    View {
        /// The Board ID, or a part of the Board name.
        board: String,
        /// The maximum number of Tickets (at most 1000).
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(1..=1000))]
        limit: u64,
        /// Show only the Tickets that are assigned to you.
        #[arg(long, short = 'm')]
        mine: bool,
        /// Write the Board, the columns, and the Tickets as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show My Boards. When the list is empty, jui shows suggestions to select from (on a terminal).
    Mine {
        /// Write My Boards as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add Boards to My Boards (at the end of the list).
    Add {
        /// Board IDs or parts of Board names.
        #[arg(required = true)]
        boards: Vec<String>,
    },
    /// Remove Boards from My Boards.
    #[command(visible_alias = "rm")]
    Remove {
        /// Board IDs or parts of the Board names in My Boards.
        #[arg(required = true)]
        boards: Vec<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SprintCmd {
    /// List the open Sprints of a Scrum Board (active first, then future).
    #[command(visible_alias = "ls")]
    List {
        /// The Board ID, or a part of the Board name.
        board: String,
        /// Also show the closed Sprints.
        #[arg(long)]
        closed: bool,
        /// Write the Sprints as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List the Tickets of the active Sprints of a Scrum Board.
    Active {
        /// The Board ID, or a part of the Board name.
        board: String,
        /// Show only the Tickets that are assigned to you.
        #[arg(long, short = 'm')]
        mine: bool,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Show one Sprint and its Tickets.
    View {
        /// The Sprint ID (`jui sprint list` shows it).
        id: u64,
        /// Show only the Tickets that are assigned to you.
        #[arg(long, short = 'm')]
        mine: bool,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Move Tickets to a Sprint.
    Add {
        /// The Sprint ID (`jui sprint list` shows it).
        sprint: u64,
        /// The Ticket keys, for example BCE-123 or 123.
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// Move Tickets to the backlog (out of their open Sprint).
    Backlog {
        /// The Ticket keys, for example BCE-123 or 123.
        #[arg(required = true)]
        keys: Vec<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum EpicCmd {
    /// List the open Epics of a Board (from its filter).
    #[command(visible_alias = "ls")]
    List {
        /// The Board ID, or a part of the Board name.
        board: String,
        /// Also show the Epics that are Done.
        #[arg(long)]
        done: bool,
        /// The maximum number of Epics.
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u64).range(1..))]
        limit: u64,
        /// Get all Epics (all pages).
        #[arg(long, conflicts_with = "limit")]
        all: bool,
        /// Write the Epics as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List the Epics that have at least one Ticket that is assigned to you and is not Done.
    Mine {
        /// Write the Epics as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List the Tickets of an Epic.
    Tickets {
        /// The Epic key, for example BCE-13070 or 13070.
        epic: String,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Set the Epic of Tickets.
    Set {
        /// The Epic key, for example BCE-13070 or 13070.
        epic: String,
        /// The Ticket keys, for example BCE-123 or 123.
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// Remove the Epic of Tickets.
    Unset {
        /// The Ticket keys, for example BCE-123 or 123.
        #[arg(required = true)]
        keys: Vec<String>,
    },
}

#[derive(Debug, Clone, Args)]
pub struct AssignArgs {
    /// The Ticket key, for example BCE-123 or 123.
    pub key: String,
    /// A username, a part of a name, `me`, or `none`. When more than one user matches,
    /// jui asks on a terminal and stops in a script.
    pub user: String,
    /// Write the Ticket after the change as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct WatchArgs {
    /// The Ticket key, for example BCE-123 or 123.
    pub key: String,
    /// A username or a part of a name. Default: you (`me`).
    pub user: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum CommentCmd {
    /// List the comments of a Ticket, oldest first.
    #[command(visible_alias = "ls")]
    List {
        /// The Ticket key, for example BCE-123 or 123.
        key: String,
        /// Show the comments as raw wiki markup.
        #[arg(long, conflicts_with = "json")]
        wiki: bool,
        /// Write the comments as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add a comment. With no text, jui opens the comment Template in the editor.
    Add(CommentAddArgs),
    /// Edit a comment in the editor. jui shows the change before it sends it.
    Edit(CommentEditArgs),
    /// Delete a comment. This cannot be reversed.
    #[command(visible_alias = "rm")]
    Delete {
        /// The Ticket key.
        key: String,
        /// The comment ID (`jui comment list` shows it).
        id: String,
        /// Do not ask before the delete.
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct CommentAddArgs {
    /// The Ticket key, for example BCE-123 or 123.
    pub key: String,

    /// The comment in Markdown. `-` reads it from stdin. With no text, jui opens the editor.
    #[arg(value_name = "MARKDOWN|-")]
    pub text: Option<String>,

    /// Open the comment Template in the editor, with the text as the start.
    #[arg(long, short = 'e', conflicts_with = "from")]
    pub editor: bool,

    /// Use a comment file or a Draft. jui opens the editor only when the file is not valid.
    #[arg(long, value_name = "FILE", conflicts_with = "text")]
    pub from: Option<std::path::PathBuf>,

    /// Send the text as raw Jira wiki markup, with no conversion.
    #[arg(long)]
    pub wiki: bool,

    /// Write the new comment as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct CommentEditArgs {
    /// The Ticket key.
    pub key: String,

    /// The comment ID (`jui comment list` shows it).
    pub id: String,

    /// The new comment in Markdown. `-` reads it from stdin. With no text, jui opens the editor.
    #[arg(value_name = "MARKDOWN|-")]
    pub text: Option<String>,

    /// Use a comment file or a Draft. jui opens the editor only when the file is not valid.
    #[arg(long, value_name = "FILE", conflicts_with = "text")]
    pub from: Option<std::path::PathBuf>,

    /// Edit the comment as raw Jira wiki markup, with no conversion.
    #[arg(long)]
    pub wiki: bool,

    /// Do not ask before jui changes the comment.
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Write the changed comment as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SubtaskCmd {
    /// List the Sub-tasks of a Ticket.
    #[command(visible_alias = "ls")]
    List {
        /// The parent Ticket key, for example BCE-123 or 123.
        key: String,
        /// Write the Sub-tasks as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create a Sub-task (same as `jui new --parent <key>`). With no -s, jui opens a Template.
    Create {
        /// The parent Ticket key.
        parent: String,
        /// The Sub-task type. Default: the first Sub-task type of the project.
        #[arg(long = "type", short = 't', value_name = "TYPE")]
        kind: Option<String>,
        #[command(flatten)]
        common: Box<WriteArgs>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum LinkCmd {
    /// List the Links of a Ticket. The ID is for `jui link delete`.
    #[command(visible_alias = "ls")]
    List {
        /// The Ticket key, for example BCE-123 or 123.
        key: String,
        /// Write the Links as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add a Link: `jui link add BCE-1 blocks BCE-2`. The relation is a text from
    /// `jui link types` (a unique prefix is enough) or a type name.
    Add {
        /// The first Ticket.
        from: String,
        /// The relation, for example "blocks", "is blocked by", or "relates to".
        relation: String,
        /// The second Ticket.
        to: String,
        /// Add a comment (Markdown) to the Link.
        #[arg(long, short = 'm', value_name = "TEXT")]
        comment: Option<String>,
    },
    /// Delete a Link. This cannot be reversed.
    #[command(visible_alias = "rm")]
    Delete {
        /// The Ticket key.
        key: String,
        /// The Link ID (`jui link list` shows it), or the key of the other Ticket.
        link: String,
        /// Do not ask before the delete.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// List the Link types and their relation texts.
    Types {
        /// Write the Link types as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// The time flags of `worklog add` and `worklog edit`.
#[derive(Debug, Clone, Default, Args)]
pub struct WorklogFlags {
    /// The start: `now`, `yesterday`, `10:00`, `yesterday 10:00`, or `2026-10-19 10:00`
    /// (local time). Default: now (add) or no change (edit).
    #[arg(long, short = 's', value_name = "WHEN")]
    pub started: Option<String>,
    /// The comment in Markdown. `-` reads it from stdin.
    #[arg(long, short = 'm', value_name = "MARKDOWN|-")]
    pub comment: Option<String>,
    /// What Jira does with the remaining estimate.
    #[arg(long, value_name = "MODE")]
    pub adjust: Option<AdjustArg>,
    /// The new remaining estimate (with --adjust new), for example 2h.
    #[arg(long, value_name = "TIME")]
    pub new_estimate: Option<String>,
    /// Reduce the remaining estimate by this time (with --adjust manual, add only).
    #[arg(long, value_name = "TIME")]
    pub reduce_by: Option<String>,
    /// Open the Worklog Template in the editor. The flags fill it.
    #[arg(long, short = 'e', conflicts_with = "from")]
    pub editor: bool,
    /// Use a Worklog file or a Draft. jui opens the editor only when the file is not valid.
    #[arg(long, value_name = "FILE")]
    pub from: Option<std::path::PathBuf>,
    /// Write the Worklog as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum AdjustArg {
    Auto,
    Leave,
    New,
    Manual,
}

impl AdjustArg {
    pub fn name(self) -> &'static str {
        match self {
            AdjustArg::Auto => "auto",
            AdjustArg::Leave => "leave",
            AdjustArg::New => "new",
            AdjustArg::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum WorklogCmd {
    /// List the Worklogs of a Ticket, oldest first.
    #[command(visible_alias = "ls")]
    List {
        /// The Ticket key, for example BCE-123 or 123.
        key: String,
        /// Write the Worklogs as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add a Worklog: `jui worklog add BCE-1 1h30m -m "Review"`. With no time, jui opens the Template.
    Add {
        /// The Ticket key.
        key: String,
        /// The time spent, for example 1h30m, 90m, 1.5h, or 2d (a day is 8 hours).
        time: Option<String>,
        #[command(flatten)]
        flags: WorklogFlags,
    },
    /// Edit a Worklog. With no time and no flags, jui opens the Template.
    Edit {
        /// The Ticket key.
        key: String,
        /// The Worklog ID (`jui worklog list` shows it).
        id: String,
        /// The new time spent.
        time: Option<String>,
        #[command(flatten)]
        flags: WorklogFlags,
        /// Do not ask before jui changes the comment.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Delete a Worklog. This cannot be reversed.
    #[command(visible_alias = "rm")]
    Delete {
        /// The Ticket key.
        key: String,
        /// The Worklog ID (`jui worklog list` shows it).
        id: String,
        /// What Jira does with the remaining estimate. Default: auto (adds the time back).
        #[arg(long, value_name = "MODE")]
        adjust: Option<AdjustArg>,
        /// The new remaining estimate (with --adjust new).
        #[arg(long, value_name = "TIME")]
        new_estimate: Option<String>,
        /// Increase the remaining estimate by this time (with --adjust manual).
        #[arg(long, value_name = "TIME")]
        increase_by: Option<String>,
        /// Do not ask before the delete.
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AttachmentCmd {
    /// List the attachments of a Ticket. The ID is for download and delete.
    #[command(visible_alias = "ls")]
    List {
        /// The Ticket key, for example BCE-123 or 123.
        key: String,
        /// Write the attachments as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Upload files to a Ticket. jui checks all files before it sends the first one.
    #[command(visible_alias = "add")]
    Upload {
        /// The Ticket key.
        key: String,
        /// The files. The limit of the server is 50 MB for each file.
        #[arg(required = true, value_name = "FILE")]
        files: Vec<std::path::PathBuf>,
        /// Write the new attachments as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Download attachments. With no name and no --all, jui asks on a terminal.
    #[command(visible_alias = "get")]
    Download {
        /// The Ticket key.
        key: String,
        /// Attachment IDs or file names (a unique prefix is enough).
        #[arg(value_name = "ID|NAME")]
        names: Vec<String>,
        /// Download all attachments.
        #[arg(long, short = 'a', conflicts_with = "names")]
        all: bool,
        /// The target: a directory, a file name (one attachment only), or `-` for stdout.
        /// Default: the current directory.
        #[arg(long, short = 'o', value_name = "PATH|-")]
        output: Option<std::path::PathBuf>,
        /// Replace a file that exists.
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Delete an attachment. This cannot be reversed.
    #[command(visible_alias = "rm")]
    Delete {
        /// The Ticket key.
        key: String,
        /// The attachment ID or file name.
        #[arg(value_name = "ID|NAME")]
        name: String,
        /// Do not ask before the delete.
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct ViewArgs {
    /// The Ticket key, for example BCE-123 or 123 (the default project is added).
    pub key: String,

    /// Which comments to show: a number of recent comments, or `all`.
    #[arg(long, default_value = "5", value_name = "N|all")]
    pub comments: CommentCount,

    /// Show the description and comments as raw wiki markup.
    #[arg(long, conflicts_with_all = ["json", "raw"])]
    pub wiki: bool,

    /// Write the jui JSON (domain types) to stdout.
    #[arg(long, conflicts_with = "raw")]
    pub json: bool,

    /// Write the raw Jira JSON to stdout (for debugging).
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Clone, Args)]
pub struct MoveArgs {
    /// The Ticket key, for example BCE-123 or 123.
    pub key: String,

    /// The destination Status or the Transition name. A unique word prefix is enough
    /// (`pull` selects "In Pull Request"). With no target, jui shows a list.
    pub target: Option<String>,

    /// Set the resolution, for example for Done.
    #[arg(long, short = 'r', value_name = "NAME")]
    pub resolution: Option<String>,

    /// Set a field on the Transition screen: `--set "Story Points=3"`. Use the field
    /// name or ID. Repeat for more fields.
    #[arg(long, value_name = "FIELD=VALUE")]
    pub set: Vec<String>,

    /// Add a comment (Markdown) in the same request.
    #[arg(long, short = 'm', value_name = "TEXT")]
    pub comment: Option<String>,

    /// Write the Ticket after the move as JSON.
    #[arg(long)]
    pub json: bool,
}

/// The field flags of `new` and `edit`. See ticket 05, part 5.
#[derive(Debug, Clone, Default, Args)]
pub struct FieldArgs {
    /// The summary.
    #[arg(long, short = 's')]
    pub summary: Option<String>,

    /// A component. Repeat for more components.
    #[arg(long = "component", short = 'c', value_name = "NAME")]
    pub components: Vec<String>,

    /// The description in Markdown. `-` reads it from stdin.
    #[arg(long, short = 'd', value_name = "MARKDOWN|-")]
    pub description: Option<String>,

    /// The assignee: a username, `me`, or `none`.
    #[arg(long, short = 'a', value_name = "USER")]
    pub assignee: Option<String>,

    /// The Epic key (Epic Link).
    #[arg(long, value_name = "KEY")]
    pub epic: Option<String>,

    /// A label. Repeat for more labels.
    #[arg(long = "label", short = 'l', value_name = "LABEL")]
    pub labels: Vec<String>,

    /// The priority, for example High.
    #[arg(long, value_name = "NAME")]
    pub priority: Option<String>,

    /// Set any field: `--set severity=Major`. Use the Template key, the field name, or the
    /// field ID. Repeat for more fields.
    #[arg(long, value_name = "FIELD=VALUE")]
    pub set: Vec<String>,
}

impl FieldArgs {
    /// True when no field flag is given.
    pub fn is_empty(&self) -> bool {
        self.summary.is_none()
            && self.components.is_empty()
            && self.description.is_none()
            && self.assignee.is_none()
            && self.epic.is_none()
            && self.labels.is_empty()
            && self.priority.is_none()
            && self.set.is_empty()
    }
}

/// Flags that `new` and `edit` both have.
#[derive(Debug, Clone, Default, Args)]
pub struct WriteArgs {
    #[command(flatten)]
    pub fields: FieldArgs,

    /// Always open the Template in the editor. The flags fill the Template.
    #[arg(long, short = 'e', conflicts_with = "from")]
    pub editor: bool,

    /// Use a Template file or a Draft. jui opens the editor only when the file is not valid.
    #[arg(long, value_name = "FILE")]
    pub from: Option<std::path::PathBuf>,

    /// Do not ask before jui changes existing text.
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Write the Ticket after the change as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct NewArgs {
    /// The project. Default: the Setting tickets.default_project.
    #[arg(long, short = 'p')]
    pub project: Option<String>,

    /// The Ticket type. Default: the Setting tickets.default_type.
    #[arg(long = "type", short = 't', value_name = "TYPE")]
    pub kind: Option<String>,

    /// The parent Ticket. jui creates a Sub-task when -t is not given.
    #[arg(long, value_name = "KEY")]
    pub parent: Option<String>,

    #[command(flatten)]
    pub common: WriteArgs,
}

#[derive(Debug, Clone, Args)]
pub struct EditArgs {
    /// The Ticket key, for example BCE-123 or 123.
    pub key: String,

    /// Edit the description as raw Jira wiki markup, with no conversion.
    #[arg(long)]
    pub wiki: bool,

    #[command(flatten)]
    pub common: WriteArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentCount {
    Last(usize),
    All,
}

impl std::str::FromStr for CommentCount {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        if s.eq_ignore_ascii_case("all") {
            return Ok(CommentCount::All);
        }
        s.parse::<usize>().map(CommentCount::Last).map_err(|_| "use a number or `all`".to_string())
    }
}

#[derive(Debug, Clone, Args)]
pub struct ListArgs {
    /// The maximum number of Tickets.
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u64).range(1..))]
    pub limit: u64,

    /// Get all Tickets (all pages).
    #[arg(long, conflicts_with = "limit")]
    pub all: bool,

    /// Write JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCmd {
    /// Show all Settings with their values and sources.
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Show one value, for example `tickets.default_project`.
    Get { key: String },
    /// Check and save one value.
    Set { key: String, value: String },
    /// Remove a key from the file, so that jui uses the default value.
    Unset { key: String },
    /// Open the config file in the editor.
    Edit,
    /// Show the path of the config file.
    Path,
    /// Write the config file with comments and the default values.
    Init {
        #[arg(long)]
        force: bool,
    },
}
