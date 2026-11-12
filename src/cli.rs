//! The command-line interface. See ticket 05 "jui CLI command tree".

pub mod agile;
pub mod args;
pub mod attachment;
pub mod board;
pub mod comment;
pub mod filter;
pub mod mv;
pub mod output;
pub mod people;
pub mod prompt;
pub mod relations;
pub mod view;
pub mod write;

use std::io::IsTerminal;

use clap::Parser;

use crate::core::settings::{self, Env, Settings};
use crate::core::{Error, Jira, Result};
use args::{AttachmentCmd, BoardCmd, Cli, Command, CommentCmd, ConfigCmd, EpicCmd, FilterCmd, LinkCmd, SprintCmd, SubtaskCmd, TicketCmd, WorklogCmd};
use output::Out;

/// Insert `view` before a bare Ticket key: `jui BCE-1` becomes `jui view BCE-1`.
pub fn rewrite_args(mut args: Vec<String>) -> Vec<String> {
    // Find the first argument that is not a global flag.
    let first = args.iter().skip(1).position(|a| !a.starts_with('-')).map(|i| i + 1);
    if let Some(i) = first
        && crate::core::model::looks_like_key(&args[i])
    {
        args.insert(i, "view".into());
    }
    args
}

/// The function that starts the TUI. `main.rs` gives it, because the `cli` module
/// must not import the `tui` module.
pub type TuiFn = fn(Settings) -> futures::future::BoxFuture<'static, Result<()>>;

/// Run the CLI. Returns the process exit code.
pub async fn main(tui: TuiFn) -> i32 {
    let args = rewrite_args(std::env::args().collect());
    let cli = Cli::parse_from(args);
    // The TUI uses the terminal, so `-v` cannot write to stderr. Use `JUI_LOG` for the TUI.
    let tui_cmd = matches!(cli.command, None | Some(Command::Tui));
    crate::core::logging::init(if tui_cmd { 0 } else { cli.verbose });
    let json = cli.json_errors();
    match run(cli, tui).await {
        Ok(()) => 0,
        Err(e) => {
            output::print_error(&e, json);
            e.exit_code()
        }
    }
}

async fn run(cli: Cli, tui_fn: TuiFn) -> Result<()> {
    let env = Env::from_process();
    let cmd = cli.command.clone().unwrap_or(Command::Tui);

    // Completion needs no Settings, no token, and no server.
    if let Command::Completions { shell } = &cmd {
        print_completions(*shell, &mut std::io::stdout());
        return Ok(());
    }
    // `config path` and `config edit` must work when the file is not valid.
    if let Command::Config { cmd: ConfigCmd::Path } = &cmd {
        println!("{}", settings::config_path(&env).display());
        return Ok(());
    }
    if let Command::Config { cmd: ConfigCmd::Edit } = &cmd {
        return config_edit(&env);
    }

    let s = Settings::load(&env)?;
    for w in &s.warnings {
        output::print_warning(w);
    }
    let out = Out::new(&s, cli.color_choice());

    match cmd {
        Command::Tui => tui(s, tui_fn).await,
        Command::Ls(a) => list(&s, &out, &s.my_tickets_jql.clone(), s.group_by_sprint, a).await,
        Command::Search { jql, list: a } => list(&s, &out, &jql, false, a).await,
        Command::Ticket { cmd: TicketCmd::List(a) } => list(&s, &out, &s.my_tickets_jql.clone(), s.group_by_sprint, a).await,
        Command::View(v) | Command::Ticket { cmd: TicketCmd::View(v) } => view(&s, &out, v).await,
        Command::Open { key, print } | Command::Ticket { cmd: TicketCmd::Open { key, print } } => open(&s, &key, print),
        Command::Mv(m) | Command::Ticket { cmd: TicketCmd::Move(m) } => move_ticket(&s, &out, m).await,
        Command::New(a) | Command::Ticket { cmd: TicketCmd::Create(a) } => new_ticket(&s, &out, a).await,
        Command::Edit(a) | Command::Ticket { cmd: TicketCmd::Edit(a) } => edit_ticket(&s, &out, a).await,
        Command::Comment { cmd } => comment_cmd(&s, &out, cmd).await,
        Command::Subtask { cmd } => subtask_cmd(&s, &out, cmd).await,
        Command::Link { cmd } => link_cmd(&s, &out, cmd).await,
        Command::Worklog { cmd } => worklog_cmd(&s, &out, cmd).await,
        Command::Attachment { cmd } => attachment_cmd(&s, &out, cmd).await,
        Command::Filter { cmd } => filter_cmd(&s, &out, cmd).await,
        Command::Board { cmd } => board_cmd(&s, &out, cmd).await,
        Command::Sprint { cmd } => sprint_cmd(&s, &out, cmd).await,
        Command::Epic { cmd } => epic_cmd(&s, &out, cmd).await,
        Command::Assign(a) | Command::Ticket { cmd: TicketCmd::Assign(a) } => assign(&s, &out, a).await,
        Command::Ticket { cmd: TicketCmd::Watch(a) } => watch(&s, a, true).await,
        Command::Ticket { cmd: TicketCmd::Unwatch(a) } => watch(&s, a, false).await,
        Command::Ticket { cmd: TicketCmd::Watchers { key, json } } => watchers(&s, &out, &key, json).await,
        Command::Config { cmd } => config(&env, &s, &out, cmd),
        Command::Completions { .. } => unreachable!("handled before the Settings load"),
    }
}

/// Write the completion script of a shell. The script is static: it has all commands,
/// aliases, flags, and fixed values (see ticket 05 "jui CLI command tree").
pub fn print_completions(shell: clap_complete::Shell, out: &mut dyn std::io::Write) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    // `generate` panics on a write error. A buffer cannot fail, and a closed pipe (for
    // example `| head`) is not an error here.
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut cmd, "jui", &mut buf);
    let _ = out.write_all(&buf).and_then(|()| out.flush());
}

/// Start the TUI. It needs a terminal on stdin and stdout.
async fn tui(s: Settings, start: TuiFn) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Error::input_hint(
            "The TUI needs a terminal.",
            "Use `jui ls` for My Tickets, or `jui --help` for all commands.",
        ));
    }
    start(s).await
}

/// Check and complete a Ticket key from the command line.
fn ticket_key(s: &Settings, raw: &str) -> Result<String> {
    let key = crate::core::model::normalize_key(raw, &s.default_project);
    if !crate::core::model::looks_like_key(&key) {
        return Err(Error::input_hint(
            format!("`{raw}` is not a Ticket key."),
            "Use a key like BCE-123, or only the number for the default project.",
        ));
    }
    Ok(key)
}

async fn view(s: &Settings, out: &Out, v: args::ViewArgs) -> Result<()> {
    let key = ticket_key(s, &v.key)?;
    let j = jira(s)?;
    if v.raw {
        return out.json(&j.ticket_raw(&key).await?);
    }
    let t = j.ticket(&key).await?;
    if v.json {
        return out.json(&t);
    }
    let opts = view::ViewOpts { color: out.color(), wiki: v.wiki, comments: v.comments, date_format: s.date_format };
    println!("{}", view::render(&t, &opts));
    Ok(())
}

async fn move_ticket(s: &Settings, out: &Out, m: args::MoveArgs) -> Result<()> {
    let key = ticket_key(s, &m.key)?;
    let j = jira(s)?;
    let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut err = std::io::stderr();
    let mut ask = mv::Asker { input: if terminal { Some(&mut lock) } else { None }, out: &mut err };
    let t = match mv::run(&j, &key, &m, &mut ask).await {
        Err(Error::Cancelled) => {
            eprintln!("Cancelled. {key} did not change.");
            return Ok(());
        }
        r => r?,
    };
    if m.json {
        return out.json(&j.ticket(&key).await?);
    }
    eprintln!("{key} moved to {}.", t.to);
    Ok(())
}

/// Run `f` with the terminal parts of `write::Io`: the editor and the questions on a
/// terminal, nothing in a script.
async fn with_io<T>(
    s: &Settings,
    stdin_text: Option<String>,
    f: impl AsyncFnOnce(&mut write::Io<'_>) -> Result<T>,
) -> Result<T> {
    use std::io::IsTerminal;
    // With `-d -`, stdin is the text, so jui cannot ask questions on it.
    let terminal = stdin_text.is_none() && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let command = crate::editor::editor_from_env(&s.editor_command);
    let mut run_editor = move |text: &str, label: &str| {
        crate::editor::edit_text_with(&command, text, &crate::editor::template_prefix(label), ".md", || Ok(()), || Ok(()))
    };
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut err = std::io::stderr();
    let mut io = write::Io {
        editor: if terminal { Some(&mut run_editor) } else { None },
        input: if terminal { Some(&mut lock) } else { None },
        out: &mut err,
        color: terminal && std::env::var_os("NO_COLOR").is_none(),
        drafts: crate::core::template::draft_dir(),
        stdin: stdin_text,
    };
    f(&mut io).await
}

async fn new_ticket(s: &Settings, out: &Out, a: args::NewArgs) -> Result<()> {
    let j = jira(s)?;
    let r = with_io(s, write::read_stdin_if(&a.common.fields), async |io| write::new(&j, &a, &s.default_project, &s.default_type, io).await).await?;
    match r {
        write::Outcome::Created(key) => {
            if a.common.json {
                return out.json(&j.ticket(&key).await?);
            }
            println!("{key}");
            eprintln!("Created {key}: {}", j.browse_url(&key));
        }
        write::Outcome::Cancelled(_) => {}
        write::Outcome::Edited { .. } => unreachable!(),
    }
    Ok(())
}

async fn edit_ticket(s: &Settings, out: &Out, a: args::EditArgs) -> Result<()> {
    let key = ticket_key(s, &a.key)?;
    let j = jira(s)?;
    let r = with_io(s, write::read_stdin_if(&a.common.fields), async |io| write::edit(&j, &a, &key, io).await).await?;
    match r {
        write::Outcome::Edited { key, changed } => {
            if a.common.json {
                return out.json(&j.ticket(&key).await?);
            }
            eprintln!("Saved {key}. Changed: {}.", changed.join(", "));
        }
        write::Outcome::Cancelled(_) => {}
        write::Outcome::Created(_) => unreachable!(),
    }
    Ok(())
}

async fn comment_cmd(s: &Settings, out: &Out, cmd: CommentCmd) -> Result<()> {
    let j = jira(s)?;
    match cmd {
        CommentCmd::List { key, wiki, json } => {
            let key = ticket_key(s, &key)?;
            let comments = j.comments(&key).await?;
            if json {
                return out.json(&comments);
            }
            if comments.is_empty() {
                eprintln!("{key} has no comments.");
                return Ok(());
            }
            println!("{}", view::render_comments(&comments, out.color(), wiki, s.date_format));
            Ok(())
        }
        CommentCmd::Add(a) => {
            let key = ticket_key(s, &a.key)?;
            let stdin = write::read_stdin_when(a.text.as_deref() == Some("-"));
            match with_io(s, stdin, async |io| comment::add(&j, &a, &key, io).await).await? {
                comment::Outcome::Added(c) if a.json => out.json(&c),
                comment::Outcome::Added(c) => {
                    println!("{}", c.id);
                    eprintln!("Added comment {} to {key}.", c.id);
                    Ok(())
                }
                _ => Ok(()),
            }
        }
        CommentCmd::Edit(a) => {
            let key = ticket_key(s, &a.key)?;
            let stdin = write::read_stdin_when(a.text.as_deref() == Some("-"));
            match with_io(s, stdin, async |io| comment::edit(&j, &a, &key, io).await).await {
                Ok(comment::Outcome::Edited(c)) if a.json => out.json(&c),
                Ok(comment::Outcome::Edited(c)) => {
                    eprintln!("Saved comment {} on {key}.", c.id);
                    Ok(())
                }
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            }
        }
        CommentCmd::Delete { key, id, yes } => {
            let key = ticket_key(s, &key)?;
            if with_io(s, None, async |io| comment::delete(&j, &key, &id, yes, io).await).await? {
                eprintln!("Deleted comment {id} on {key}.");
            } else {
                eprintln!("Cancelled. Comment {id} on {key} did not change.");
            }
            Ok(())
        }
    }
}

async fn subtask_cmd(s: &Settings, out: &Out, cmd: SubtaskCmd) -> Result<()> {
    match cmd {
        SubtaskCmd::List { key, json } => {
            let key = ticket_key(s, &key)?;
            let (summary, subs) = jira(s)?.subtasks(&key).await?;
            if json {
                return out.json(&subs);
            }
            if subs.is_empty() {
                eprintln!("{key} has no Sub-tasks.");
                return Ok(());
            }
            eprintln!("{key}: {summary}");
            println!("{}", relations::render_subtasks(&subs));
            Ok(())
        }
        SubtaskCmd::Create { parent, kind, common } => {
            let parent = ticket_key(s, &parent)?;
            let a = args::NewArgs { project: None, kind, parent: Some(parent), common: *common };
            new_ticket(s, out, a).await
        }
    }
}

async fn link_cmd(s: &Settings, out: &Out, cmd: LinkCmd) -> Result<()> {
    let j = jira(s)?;
    match cmd {
        LinkCmd::List { key, json } => {
            let key = ticket_key(s, &key)?;
            let tl = j.links(&key).await?;
            if json {
                return out.json(&tl.links);
            }
            if tl.links.is_empty() {
                eprintln!("{key} has no Links.");
                return Ok(());
            }
            println!("{}", relations::render_links(&tl.links));
            Ok(())
        }
        LinkCmd::Types { json } => {
            let types = j.link_types().await?;
            if json {
                return out.json(&types);
            }
            eprintln!("Name, outward text (`A <text> B`), inward text:");
            println!("{}", relations::render_types(&types));
            Ok(())
        }
        LinkCmd::Add { from, relation, to, comment } => {
            let (from, to) = (ticket_key(s, &from)?, ticket_key(s, &to)?);
            let r = with_io(s, None, async |io| relations::add_link(&j, &from, &relation, &to, comment.as_deref(), io).await).await;
            match r {
                Ok(relations::LinkOutcome::Added(d)) => eprintln!("Added the Link: {d}."),
                Ok(relations::LinkOutcome::Exists(d)) => eprintln!("The Link exists: {d}. Nothing changed."),
                Err(Error::Cancelled) => eprintln!("Cancelled. No Link added."),
                Err(e) => return Err(e),
            }
            Ok(())
        }
        LinkCmd::Delete { key, link, yes } => {
            let key = ticket_key(s, &key)?;
            match with_io(s, None, async |io| relations::delete_link(&j, &key, &link, yes, io).await).await {
                Ok(Some(d)) => eprintln!("Deleted the Link: {d}."),
                Ok(None) | Err(Error::Cancelled) => eprintln!("Cancelled. The Links of {key} did not change."),
                Err(e) => return Err(e),
            }
            Ok(())
        }
    }
}

async fn worklog_cmd(s: &Settings, out: &Out, cmd: WorklogCmd) -> Result<()> {
    let j = jira(s)?;
    let clock = crate::core::worklog::Clock::system();
    match cmd {
        WorklogCmd::List { key, json } => {
            let key = ticket_key(s, &key)?;
            let ws = j.worklogs(&key).await?;
            if json {
                return out.json(&ws);
            }
            if ws.is_empty() {
                eprintln!("{key} has no Worklogs.");
                return Ok(());
            }
            println!("{}", relations::render_worklogs(&ws));
            Ok(())
        }
        WorklogCmd::Add { key, time, flags } => {
            let key = ticket_key(s, &key)?;
            let stdin = write::read_stdin_when(flags.comment.as_deref() == Some("-"));
            match with_io(s, stdin, async |io| relations::add_worklog(&j, &key, time.as_deref(), &flags, clock, io).await).await? {
                relations::WorklogOutcome::Added(w, _) if flags.json => out.json(&w),
                relations::WorklogOutcome::Added(w, p) => {
                    println!("{}", w.id);
                    eprintln!("Added Worklog {} to {key}: {}.", w.id, p.describe());
                    Ok(())
                }
                _ => Ok(()),
            }
        }
        WorklogCmd::Edit { key, id, time, flags, yes } => {
            let key = ticket_key(s, &key)?;
            let stdin = write::read_stdin_when(flags.comment.as_deref() == Some("-"));
            match with_io(s, stdin, async |io| relations::edit_worklog(&j, relations::EditCmd { key: &key, id: &id, time: time.as_deref(), flags: &flags, yes }, clock, io).await).await? {
                relations::WorklogOutcome::Edited(w, _) if flags.json => out.json(&w),
                relations::WorklogOutcome::Edited(w, p) => {
                    eprintln!("Saved Worklog {} on {key}: {}.", w.id, p.describe());
                    Ok(())
                }
                _ => Ok(()),
            }
        }
        WorklogCmd::Delete { key, id, adjust, new_estimate, increase_by, yes } => {
            let key = ticket_key(s, &key)?;
            let adj = relations::delete_adjust(adjust, new_estimate.as_deref(), increase_by.as_deref())?;
            if with_io(s, None, async |io| relations::delete_worklog(&j, &key, &id, &adj, yes, io).await).await? {
                eprintln!("Deleted Worklog {id} on {key}.");
            } else {
                eprintln!("Cancelled. Worklog {id} on {key} did not change.");
            }
            Ok(())
        }
    }
}

async fn attachment_cmd(s: &Settings, out: &Out, cmd: AttachmentCmd) -> Result<()> {
    let j = jira(s)?;
    match cmd {
        AttachmentCmd::List { key, json } => {
            let key = ticket_key(s, &key)?;
            let (_, atts) = j.attachments(&key).await?;
            if json {
                return out.json(&atts);
            }
            if atts.is_empty() {
                eprintln!("{key} has no attachments.");
                return Ok(());
            }
            println!("{}", attachment::render_list(&atts));
            Ok(())
        }
        AttachmentCmd::Upload { key, files, json } => {
            let key = ticket_key(s, &key)?;
            let added = with_io(s, None, async |io| attachment::upload(&j, &key, &files, io).await).await?;
            if json {
                return out.json(&added);
            }
            for a in &added {
                println!("{}", a.id);
            }
            Ok(())
        }
        AttachmentCmd::Download { key, names, all, output, force } => {
            let key = ticket_key(s, &key)?;
            let c = attachment::DownloadCmd { key: &key, names: &names, all, output: output.as_deref(), force };
            match with_io(s, None, async |io| attachment::download(&j, c, io).await).await {
                Ok(_) => Ok(()),
                Err(Error::Cancelled) => {
                    eprintln!("Cancelled. No file written.");
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        AttachmentCmd::Delete { key, name, yes } => {
            let key = ticket_key(s, &key)?;
            match with_io(s, None, async |io| attachment::delete(&j, &key, &name, yes, io).await).await {
                Ok(Some(a)) => eprintln!("Deleted attachment {} ({}) on {key}.", a.id, a.filename),
                Ok(None) | Err(Error::Cancelled) => eprintln!("Cancelled. The attachments of {key} did not change."),
                Err(e) => return Err(e),
            }
            Ok(())
        }
    }
}

async fn filter_cmd(s: &Settings, out: &Out, cmd: FilterCmd) -> Result<()> {
    let j = jira(s)?;
    let find = async |arg: &str| -> Result<crate::core::model::Filter> {
        let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        let mut err = std::io::stderr();
        let mut ask = filter::Ask { input: if terminal { Some(&mut lock) } else { None }, out: &mut err };
        filter::find(&j, arg, &mut ask).await
    };
    match cmd {
        FilterCmd::List { json } => {
            let favs = j.favourite_filters().await?;
            if json {
                return out.json(&favs);
            }
            if favs.is_empty() {
                let e = crate::core::filter::no_favourites();
                eprintln!("{}", e);
                if let Some(h) = e.hint() {
                    eprintln!("hint: {h}");
                }
                return Ok(());
            }
            out.filter_table(&favs);
            Ok(())
        }
        FilterCmd::View { filter, json } => {
            let f = match find(&filter).await {
                Err(Error::Cancelled) => return Ok(()),
                r => r?,
            };
            if json {
                return out.json(&f);
            }
            println!("{}", filter::render(&f));
            Ok(())
        }
        FilterCmd::Run { filter, list: a } => {
            let f = match find(&filter).await {
                Err(Error::Cancelled) => return Ok(()),
                r => r?,
            };
            if !a.json {
                eprintln!("Filter {}: {}", f.id, f.name);
            }
            list(s, out, &f.jql, false, a).await
        }
    }
}

/// Run `f` with the Board questions on a terminal (stdin and stderr), or with no questions in a script.
async fn with_board_ask<T>(f: impl AsyncFnOnce(&mut board::Ask<'_, '_>) -> Result<T>) -> Result<T> {
    let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut err = std::io::stderr();
    let mut ask = board::Ask { input: if terminal { Some(&mut lock) } else { None }, out: &mut err };
    f(&mut ask).await
}

async fn board_cmd(s: &Settings, out: &Out, cmd: BoardCmd) -> Result<()> {
    use crate::core::board as bd;
    // `remove` changes only the file. It needs no token and no request.
    if let BoardCmd::Remove { boards } = &cmd {
        let c = match with_board_ask(async |ask| board::remove(s, boards, ask)).await {
            Err(Error::Cancelled) => {
                eprintln!("Cancelled. My Boards did not change.");
                return Ok(());
            }
            r => r?,
        };
        for r in &c.changed {
            eprintln!("Removed {} from My Boards.", bd::ref_label(r));
        }
        return Ok(());
    }
    let j = jira(s)?;
    match cmd {
        BoardCmd::Remove { .. } => unreachable!("handled above"),
        BoardCmd::List { project, name, kind, limit, all, json } => {
            let q = crate::core::jira::BoardQuery {
                name,
                project: project.map(|p| p.trim().to_uppercase()),
                kind: kind.map(|k| match k {
                    args::BoardType::Scrum => "scrum".to_string(),
                    args::BoardType::Kanban => "kanban".to_string(),
                }),
            };
            let page = j.boards_all(&q, if all { None } else { Some(limit) }).await?;
            let mine: Vec<u64> = s.my_boards.iter().map(|b| b.id).collect();
            if json {
                let rows: Vec<board::ListRow> =
                    page.items.iter().map(|b| board::ListRow { mine: mine.contains(&b.id), board: b.clone() }).collect();
                return out.json(&rows);
            }
            if page.items.is_empty() {
                eprintln!("No Boards.");
                return Ok(());
            }
            out.board_table(&page.items, &mine);
            if page.has_more() {
                eprintln!("Showing {} of {}. Use --limit, --all, --name, or --project.", page.items.len(), page.total);
            }
            Ok(())
        }
        BoardCmd::View { board: arg, limit, mine, json } => {
            // An ID needs no extra request: the configuration has the name.
            let id = match crate::core::board::BoardArg::parse(&arg) {
                crate::core::board::BoardArg::Id(id) => id,
                crate::core::board::BoardArg::Name(_) => match with_board_ask(async |ask| board::find(&j, &arg, ask).await).await {
                    Err(Error::Cancelled) => return Ok(()),
                    r => r?.id,
                },
            };
            // Only the flag: the TUI Setting `boards.only_mine` does not change the CLI output.
            let v = j.board_view(id, limit, mine).await?;
            let b = &v.board;
            if s.my_boards.iter().any(|r| r.id == b.id) {
                board::save_names(s, std::slice::from_ref(&v.board));
            }
            if json {
                out.json(&v)?;
            } else {
                println!("{}", board::render(&v, out.color()));
            }
            if let Some(note) = board::truncated_note(&v) {
                eprintln!("{note}");
            }
            Ok(())
        }
        BoardCmd::Mine { json } => {
            if s.my_boards.is_empty() {
                if json {
                    return out.json(&Vec::<board::MineRow>::new());
                }
                let added = match with_board_ask(async |ask| board::pick_first(&j, s, ask).await).await {
                    Err(Error::Cancelled) => Vec::new(),
                    r => r?,
                };
                if added.is_empty() {
                    eprintln!("My Boards is still empty. jui asks again the next time.");
                }
                for r in &added {
                    println!("{}", bd::ref_label(r));
                }
                if !added.is_empty() {
                    eprintln!("Saved {} Boards to My Boards in {}.", added.len(), s.path.display());
                }
                return Ok(());
            }
            let rows = board::mine(&j, s).await?;
            if json {
                return out.json(&rows);
            }
            for r in &rows {
                let name = r.name.as_deref().unwrap_or("");
                match (&r.kind, r.found) {
                    (Some(k), true) => println!("{}\t{name}\t{k}", r.id),
                    _ => println!("{}\t{name}\tnot found", r.id),
                }
            }
            let missing = rows.iter().filter(|r| !r.found).count();
            if missing > 0 {
                output::print_warning(&format!(
                    "{missing} Board{} not found. Jira gives HTTP 404: the Board does not exist, or you cannot see it. Use `jui board remove <id>`.",
                    if missing == 1 { " is" } else { "s are" }
                ));
            }
            Ok(())
        }
        BoardCmd::Add { boards } => {
            let c = match with_board_ask(async |ask| board::add(&j, s, &boards, ask).await).await {
                Err(Error::Cancelled) => {
                    eprintln!("Cancelled. My Boards did not change.");
                    return Ok(());
                }
                r => r?,
            };
            for r in &c.changed {
                eprintln!("Added {} to My Boards.", bd::ref_label(r));
            }
            for r in &c.skipped {
                eprintln!("{} is already in My Boards.", bd::ref_label(r));
            }
            Ok(())
        }
    }
}

/// A Board argument as an ID: a number needs no request; a name searches the Boards
/// (more matches ask on a terminal and stop in a script).
async fn board_id(j: &Jira, arg: &str) -> Result<u64> {
    match crate::core::board::BoardArg::parse(arg) {
        crate::core::board::BoardArg::Id(id) => Ok(id),
        crate::core::board::BoardArg::Name(_) => Ok(with_board_ask(async |ask| board::find(j, arg, ask).await).await?.id),
    }
}

/// Check and complete the Ticket keys of a move. Duplicates are removed.
fn ticket_keys(s: &Settings, raw: &[String]) -> Result<Vec<String>> {
    let keys = raw.iter().map(|k| ticket_key(s, k)).collect::<Result<Vec<_>>>()?;
    Ok(agile::unique(keys))
}

async fn sprint_cmd(s: &Settings, out: &Out, cmd: SprintCmd) -> Result<()> {
    use crate::core::agile as ag;
    let j = jira(s)?;
    match cmd {
        SprintCmd::List { board: arg, closed, json } => {
            let id = match board_id(&j, &arg).await {
                Err(Error::Cancelled) => return Ok(()),
                r => r?,
            };
            let states = if closed { "active,future,closed" } else { "active,future" };
            let mut sprints = j.sprints(id, states).await?;
            if !closed {
                sprints = ag::open_sprints(sprints);
            }
            if json {
                return out.json(&sprints);
            }
            if sprints.is_empty() {
                eprintln!("Board {id} has no open Sprints.");
                return Ok(());
            }
            println!("{}", agile::render_sprints(&sprints));
            Ok(())
        }
        SprintCmd::Active { board: arg, mine, list: a } => {
            let id = match board_id(&j, &arg).await {
                Err(Error::Cancelled) => return Ok(()),
                r => r?,
            };
            let sb = j.sprint_board(id, None).await?;
            if !sb.board.is_scrum() {
                return Err(Error::input_hint(
                    format!("Board {} ({}) is a Kanban Board. It has no Sprints.", sb.board.name, sb.board.id),
                    format!("Use `jui board view {}` for its Tickets.", sb.board.id),
                ));
            }
            let limit = if a.all { None } else { Some(a.limit) };
            let mut all = Vec::new();
            // Only the flag: the TUI Setting `sprint.only_mine` does not change the CLI output.
            let jql = |id| if mine { ag::only_mine(&ag::sprint_jql(id)) } else { ag::sprint_jql(id) };
            for sp in &sb.sprints {
                let page = j.search_all(&jql(sp.id), limit).await?;
                all.push((sp.clone(), page));
            }
            if a.json {
                let rows: Vec<agile::SprintTickets<_>> =
                    all.into_iter().map(|(sp, p)| agile::SprintTickets { sprint: sp, total: p.total, tickets: p.items }).collect();
                return out.json(&rows);
            }
            if all.is_empty() {
                eprintln!("Board {} ({}) has no active Sprint. Use `jui sprint list {}` for the future Sprints.", sb.board.name, sb.board.id, sb.board.id);
                return Ok(());
            }
            for (i, (sp, page)) in all.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                let mine_note = if mine { " (only mine)" } else { "" };
                eprintln!("{}{mine_note}", agile::sprint_title(sp, page.items.len(), page.total));
                out.ticket_table(&page.items);
            }
            if all.iter().any(|(_, p)| p.has_more()) {
                eprintln!("Use --limit or --all for more Tickets.");
            }
            Ok(())
        }
        SprintCmd::View { id, mine, list: a } => {
            let limit = if a.all { None } else { Some(a.limit) };
            let jql = if mine { ag::only_mine(&ag::sprint_jql(id)) } else { ag::sprint_jql(id) };
            let (sp, page) = tokio::try_join!(j.sprint(id), j.search_all(&jql, limit))?;
            if a.json {
                return out.json(&agile::SprintTickets { sprint: sp, total: page.total, tickets: page.items });
            }
            println!("{}", agile::render_sprint_head(&sp));
            println!();
            out.ticket_table(&page.items);
            if page.has_more() {
                eprintln!("Showing {} of {}. Use --limit or --all.", page.items.len(), page.total);
            }
            Ok(())
        }
        SprintCmd::Add { sprint, keys } => {
            let keys = ticket_keys(s, &keys)?;
            let sp = j.sprint(sprint).await?;
            ag::check_target(&sp)?;
            j.move_to_sprint(sp.id, &keys).await?;
            eprintln!("{}", agile::moved_message(&keys, &ag::Target::Sprint(sp)));
            Ok(())
        }
        SprintCmd::Backlog { keys } => {
            let keys = ticket_keys(s, &keys)?;
            j.move_to_backlog(&keys).await?;
            eprintln!("{}", agile::moved_message(&keys, &ag::Target::Backlog));
            Ok(())
        }
    }
}

async fn epic_cmd(s: &Settings, out: &Out, cmd: EpicCmd) -> Result<()> {
    use crate::core::agile as ag;
    let j = jira(s)?;
    match cmd {
        EpicCmd::List { board: arg, done, limit, all, json } => {
            let id = match board_id(&j, &arg).await {
                Err(Error::Cancelled) => return Ok(()),
                r => r?,
            };
            let (b, page) = j.board_epics(id, done, if all { None } else { Some(limit) }).await?;
            if json {
                return out.json(&page.items);
            }
            if page.items.is_empty() {
                eprintln!("Board {} ({}) has no {}Epics.", b.name, b.id, if done { "" } else { "open " });
                return Ok(());
            }
            println!("{}", agile::render_epics(&page.items, done));
            if page.has_more() {
                eprintln!("Showing {} of {}. Use --limit or --all.", page.items.len(), page.total);
            }
            Ok(())
        }
        EpicCmd::Mine { json } => {
            let counts = j.my_epic_counts().await?;
            let epics = match ag::my_epics_jql(&counts) {
                Some(q) => j.search_epics_all(&q, None).await?.items,
                None => Vec::new(),
            };
            let rows = agile::my_epic_rows(epics, &counts);
            if json {
                return out.json(&rows);
            }
            if rows.is_empty() {
                eprintln!("None of your open Tickets is in an Epic.");
                return Ok(());
            }
            println!("{}", agile::render_my_epics(&rows));
            Ok(())
        }
        EpicCmd::Tickets { epic, list: a } => {
            let key = ticket_key(s, &epic)?;
            let limit = if a.all { None } else { Some(a.limit) };
            let jql = ag::epic_jql(&key);
            let (e, page) = tokio::try_join!(j.epic(&key), j.search_all(&jql, limit))?;
            if a.json {
                return out.json(&page.items);
            }
            eprintln!("Epic {}: {}", e.key, e.name);
            out.ticket_table(&page.items);
            if page.has_more() {
                eprintln!("Showing {} of {}. Use --limit or --all.", page.items.len(), page.total);
            }
            Ok(())
        }
        EpicCmd::Set { epic, keys } => {
            let key = ticket_key(s, &epic)?;
            let keys = ticket_keys(s, &keys)?;
            if keys.contains(&key) {
                return Err(Error::input(format!("{key} cannot be in its own Epic.")));
            }
            let e = j.epic(&key).await?;
            j.set_epic(&e.key, &keys).await?;
            eprintln!("Set the Epic of {} to {} ({}).", ag::key_list(&keys), e.key, e.name);
            Ok(())
        }
        EpicCmd::Unset { keys } => {
            let keys = ticket_keys(s, &keys)?;
            j.unset_epic(&keys).await?;
            eprintln!("Removed the Epic of {}.", ag::key_list(&keys));
            Ok(())
        }
    }
}

/// Run `f` with the questions on a terminal (stdin and stderr), or with no questions in a script.
async fn with_ask<T>(f: impl AsyncFnOnce(&mut people::Ask<'_, '_>) -> Result<T>) -> Result<T> {
    let terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut err = std::io::stderr();
    let mut ask = people::Ask { input: if terminal { Some(&mut lock) } else { None }, out: &mut err };
    f(&mut ask).await
}

async fn assign(s: &Settings, out: &Out, a: args::AssignArgs) -> Result<()> {
    let key = ticket_key(s, &a.key)?;
    let j = jira(s)?;
    let who = match with_ask(async |ask| people::assign(&j, &key, &a.user, ask).await).await {
        Err(Error::Cancelled) => {
            eprintln!("Cancelled. {key} did not change.");
            return Ok(());
        }
        r => r?,
    };
    if a.json {
        return out.json(&j.ticket(&key).await?);
    }
    match who {
        crate::core::user::Assignee::Nobody => eprintln!("{key} is now unassigned."),
        w => eprintln!("Assigned {key} to {}.", w.label()),
    }
    Ok(())
}

async fn watch(s: &Settings, a: args::WatchArgs, on: bool) -> Result<()> {
    let key = ticket_key(s, &a.key)?;
    let j = jira(s)?;
    let user = match with_ask(async |ask| people::find_watcher(&j, a.user.as_deref(), ask).await).await {
        Err(Error::Cancelled) => {
            eprintln!("Cancelled. {key} did not change.");
            return Ok(());
        }
        r => r?,
    };
    let name = user.as_ref().map(|u| u.name.as_str());
    let who = user.as_ref().map(|u| u.label()).unwrap_or_else(|| "You".into());
    if on {
        j.watch(&key, name).await?;
        eprintln!("{who} now watch{} {key}.", if user.is_some() { "es" } else { "" });
    } else {
        j.unwatch(&key, name).await?;
        eprintln!("{who} no longer watch{} {key}.", if user.is_some() { "es" } else { "" });
    }
    Ok(())
}

async fn watchers(s: &Settings, out: &Out, raw: &str, json: bool) -> Result<()> {
    let key = ticket_key(s, raw)?;
    let w = jira(s)?.watchers(&key).await?;
    if json {
        return out.json(&w);
    }
    if w.users.is_empty() {
        eprintln!("{key} has no watchers.");
        return Ok(());
    }
    for u in &w.users {
        println!("{}\t{}", u.name, u.display_name);
    }
    let you = if w.watching { " You watch it." } else { "" };
    eprintln!("{} watcher{} on {key}.{you}", w.count, if w.count == 1 { "" } else { "s" });
    Ok(())
}

fn open(s: &Settings, raw: &str, print: bool) -> Result<()> {
    let key = ticket_key(s, raw)?;
    let url = format!("{}/browse/{key}", s.base_url);
    if print {
        println!("{url}");
        return Ok(());
    }
    open::that_detached(&url).map_err(|e| Error::input_hint(format!("Cannot open the browser: {e}"), format!("Open {url}")))?;
    eprintln!("Opened {url}");
    Ok(())
}

fn jira(s: &Settings) -> Result<Jira> {
    Jira::new(s, std::env::var(settings::ENV_TOKEN).ok())
}

/// List Tickets. `by_sprint` (My Tickets with `tickets.group_by_sprint`) shows one table for
/// each Sprint group. `--json` keeps the query order; each item has its `sprint`.
async fn list(s: &Settings, out: &Out, jql: &str, by_sprint: bool, a: args::ListArgs) -> Result<()> {
    let j = jira(s)?;
    let limit = if a.all { None } else { Some(a.limit) };
    let mut page = j.search_all(jql, limit).await?;
    if a.json {
        out.json(&page.items)?;
    } else if by_sprint {
        crate::core::agile::sort_by_sprint(&mut page.items, |t| t.sprint.as_ref());
        out.sprint_tables(&page.items);
    } else {
        out.ticket_table(&page.items);
    }
    if page.has_more() {
        eprintln!(
            "Showing {} of {}. Use --limit or --all.",
            page.items.len(),
            page.total
        );
    }
    Ok(())
}

fn config(env: &Env, s: &Settings, out: &Out, cmd: ConfigCmd) -> Result<()> {
    let path = settings::config_path(env);
    match cmd {
        ConfigCmd::Path => unreachable!("handled before the Settings load"),
        ConfigCmd::Show { json } => {
            if json {
                out.json(s)?;
            } else {
                out.settings_table(s);
            }
        }
        ConfigCmd::Get { key } => match s.get(&key) {
            Some(v) => println!("{v}"),
            None if key == "my_boards" => {
                for b in &s.my_boards {
                    println!("{}\t{}", b.id, b.name.as_deref().unwrap_or(""));
                }
            }
            None => return Err(settings::unknown_key(&key)),
        },
        ConfigCmd::Set { key, value } => {
            if key == "my_boards" {
                return Err(Error::input_hint("Use `jui board add` to change My Boards.", "jui board add <id>"));
            }
            settings::set(&path, &key, &value)?;
            eprintln!("Saved {key} to {}", path.display());
        }
        ConfigCmd::Unset { key } => {
            settings::unset(&path, &key)?;
            eprintln!("Removed {key} from {}. jui uses the default value.", path.display());
        }
        ConfigCmd::Init { force } => {
            if settings::init(&path, force)? {
                eprintln!("Wrote {}", path.display());
            } else {
                return Err(Error::input_hint(
                    format!("{} exists.", path.display()),
                    "Use --force to replace it.",
                ));
            }
        }
        ConfigCmd::Edit => unreachable!("handled before the Settings load"),
    }
    Ok(())
}

/// `jui config edit`: open the config file in the editor. After the editor closes, jui
/// checks the file and shows the errors. The file changes only when the text is valid, or
/// when the user tells jui to save it with the error.
fn config_edit(env: &Env) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        let path = settings::config_path(env);
        return Err(Error::input_hint(
            "`jui config edit` needs a terminal.",
            format!("Use `jui config set`, or edit {}.", path.display()),
        ));
    }
    // The editor Setting comes from the file. If the file is not valid, use the environment.
    let command = Settings::load(env).map(|s| s.editor_command).unwrap_or_default();
    let command = crate::editor::editor_from_env(&command);
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut err = std::io::stderr();
    let mut run = |text: &str| crate::editor::edit_text_with(&command, text, "jui-config-", ".toml", || Ok(()), || Ok(()));
    config_edit_with(&settings::config_path(env), &mut run, &mut lock, &mut err)
}

/// The loop of `jui config edit`, with the editor and the terminal as parameters (for tests).
pub fn config_edit_with(
    path: &std::path::Path,
    editor: &mut dyn FnMut(&str) -> Result<String>,
    input: &mut dyn std::io::BufRead,
    out: &mut dyn std::io::Write,
) -> Result<()> {
    use settings::FileCheck;
    let original = settings::edit_text(path)?;
    let mut text = original.clone();
    loop {
        let saved = editor(&text)?;
        if saved == original {
            let _ = writeln!(out, "No changes.");
            return Ok(());
        }
        match settings::check_text(&saved, path) {
            FileCheck::Valid(warnings) => {
                settings::save_text(path, &saved)?;
                for w in &warnings {
                    let _ = writeln!(out, "warning: {w}");
                }
                let _ = writeln!(out, "Saved {}.", path.display());
                return Ok(());
            }
            FileCheck::Invalid(message) => {
                let _ = writeln!(out, "error: The file is not valid: {message}");
                let answer = prompt::ask(input, out, "(e) edit again, (s) save with the error, (q) quit with no change [e]: ")?;
                match answer.to_lowercase().as_str() {
                    "" | "e" => text = saved,
                    "s" => {
                        settings::save_text(path, &saved)?;
                        let _ = writeln!(out, "Saved {}. jui cannot start until you correct it.", path.display());
                        return Ok(());
                    }
                    _ => {
                        let _ = writeln!(out, "The file did not change.");
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_edit_checks_the_file_and_asks_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // First a syntax error, then a valid file with a bad value.
        let mut texts = vec!["[jira\n".to_string(), "[display]\ncolor = \"pink\"\n".to_string()].into_iter();
        let mut seen = Vec::new();
        let mut ed = |t: &str| {
            seen.push(t.to_string());
            Ok(texts.next().unwrap())
        };
        let mut out = Vec::new();
        config_edit_with(&path, &mut ed, &mut "e\n".as_bytes(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("error: The file is not valid"), "{text}");
        assert!(text.contains("warning:"), "{text}");
        assert_eq!(seen[1], "[jira\n", "the editor opens again with the text of the user");
        assert!(seen[0].contains("[tickets]"), "a missing file starts from the template");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[display]\ncolor = \"pink\"\n");
    }

    #[test]
    fn config_edit_quit_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[jira]\n").unwrap();
        let mut ed = |_: &str| Ok("bad = = 1".to_string());
        let mut out = Vec::new();
        config_edit_with(&path, &mut ed, &mut "q\n".as_bytes(), &mut out).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[jira]\n");
        let mut same = |t: &str| Ok(t.to_string());
        let mut out = Vec::new();
        config_edit_with(&path, &mut same, &mut "".as_bytes(), &mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("No changes."));
    }

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bare_key_becomes_view() {
        assert_eq!(rewrite_args(v(&["jui", "BCE-1"])), v(&["jui", "view", "BCE-1"]));
        assert_eq!(rewrite_args(v(&["jui", "-v", "62886"])), v(&["jui", "-v", "view", "62886"]));
        assert_eq!(rewrite_args(v(&["jui", "ls"])), v(&["jui", "ls"]));
        assert_eq!(rewrite_args(v(&["jui"])), v(&["jui"]));
    }
}
