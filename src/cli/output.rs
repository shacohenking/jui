//! CLI output: tables, JSON, errors, and colors.

use std::io::{IsTerminal, Write};

use chrono::Utc;

pub use crate::core::format::format_date;
use comfy_table::{Cell, ContentArrangement, Table, presets};
use owo_colors::{OwoColorize, Style};
use serde::Serialize;

use super::args::ColorArg;
use crate::core::model::{StatusCategory, TicketSummary};
use crate::core::settings::{ColorMode, DateFormat, KEYS, Settings};
use crate::core::{Error, Result};

pub struct Out {
    color: bool,
    date_format: DateFormat,
}

/// Decide if stdout gets colors: the flag, then the Setting, then NO_COLOR and the terminal.
pub fn use_color(flag: Option<ColorArg>, setting: ColorMode) -> bool {
    match flag {
        Some(ColorArg::Always) => return true,
        Some(ColorArg::Never) => return false,
        _ => {}
    }
    match setting {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal(),
    }
}

fn stderr_color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
}

impl Out {
    pub fn new(s: &Settings, flag: Option<ColorArg>) -> Self {
        Out { color: use_color(flag, s.color), date_format: s.date_format }
    }

    pub fn color(&self) -> bool {
        self.color
    }

    pub fn json<T: Serialize + ?Sized>(&self, v: &T) -> Result<()> {
        let text = serde_json::to_string_pretty(v).map_err(|e| Error::input(e.to_string()))?;
        println!("{text}");
        Ok(())
    }

    fn style(&self, s: Style, text: &str) -> String {
        if self.color { text.style(s).to_string() } else { text.to_string() }
    }

    fn status(&self, t: &TicketSummary) -> String {
        let s = match t.status_category {
            StatusCategory::ToDo => Style::new().dimmed(),
            StatusCategory::InProgress => Style::new().blue(),
            StatusCategory::Done => Style::new().green(),
            StatusCategory::Unknown => Style::new(),
        };
        let s = if t.status == "Blocked" { Style::new().red() } else { s };
        self.style(s, &t.status)
    }

    /// One heading line and one table for each Sprint group. The items must be in group
    /// order (`agile::sort_by_sprint`).
    pub fn sprint_tables(&self, items: &[TicketSummary]) {
        if items.is_empty() {
            eprintln!("No Tickets.");
            return;
        }
        let groups = crate::core::agile::sprint_groups(items, |t| t.sprint.as_ref());
        for (n, g) in groups.iter().enumerate() {
            if n > 0 {
                println!();
            }
            let title = format!("{} · {}", g.title(), g.range.len());
            println!("{}", self.style(Style::new().yellow().bold(), &title));
            self.ticket_table(&items[g.range.clone()]);
        }
    }

    pub fn ticket_table(&self, items: &[TicketSummary]) {
        if items.is_empty() {
            eprintln!("No Tickets.");
            return;
        }
        let mut table = new_table();
        table.set_header(
            ["Key", "Type", "Status", "Priority", "Assignee", "Updated", "Summary"]
                .map(|h| Cell::new(self.style(Style::new().bold(), h))),
        );
        let now = Utc::now();
        for t in items {
            let mut row = comfy_table::Row::from(vec![
                Cell::new(self.style(Style::new().cyan(), &t.key)),
                Cell::new(&t.kind),
                Cell::new(self.status(t)),
                Cell::new(t.priority.as_deref().unwrap_or("")),
                Cell::new(t.assignee.as_ref().map(|u| u.name.as_str()).unwrap_or("")),
                Cell::new(t.updated.map(|d| format_date(d, self.date_format, now)).unwrap_or_default()),
                Cell::new(&t.summary),
            ]);
            // One line for each Ticket: a long Summary is cut, not wrapped.
            row.max_height(1);
            table.add_row(row);
        }
        println!("{}", table.to_string().lines().map(str::trim_end).collect::<Vec<_>>().join("\n"));
    }

    /// The favourite filters: ID, name, owner, and JQL (one line each).
    pub fn filter_table(&self, items: &[crate::core::model::Filter]) {
        let mut table = new_table();
        table.set_header(["ID", "Name", "Owner", "JQL"].map(|h| Cell::new(self.style(Style::new().bold(), h))));
        for f in items {
            let mut row = comfy_table::Row::from(vec![
                Cell::new(self.style(Style::new().cyan(), &f.id)),
                Cell::new(&f.name),
                Cell::new(f.owner.as_deref().unwrap_or("")),
                Cell::new(f.jql.replace('\n', " ")),
            ]);
            row.max_height(1);
            table.add_row(row);
        }
        println!("{}", table.to_string().lines().map(str::trim_end).collect::<Vec<_>>().join("\n"));
    }

    /// The Boards: `*` (in My Boards), ID, type, and name.
    pub fn board_table(&self, items: &[crate::core::model::Board], mine: &[u64]) {
        let mut table = new_table();
        table.set_header(["", "ID", "Type", "Name"].map(|h| Cell::new(self.style(Style::new().bold(), h))));
        for b in items {
            let mark = if mine.contains(&b.id) { "*" } else { "" };
            let mut row = comfy_table::Row::from(vec![
                Cell::new(self.style(Style::new().yellow(), mark)),
                Cell::new(self.style(Style::new().cyan(), &b.id.to_string())),
                Cell::new(&b.kind),
                Cell::new(&b.name),
            ]);
            row.max_height(1);
            table.add_row(row);
        }
        println!("{}", table.to_string().lines().map(str::trim_end).collect::<Vec<_>>().join("\n"));
    }

    pub fn settings_table(&self, s: &Settings) {
        let mut table = new_table();
        table.set_header(["Setting", "Value", "Source"].map(|h| Cell::new(self.style(Style::new().bold(), h))));
        for k in KEYS {
            let v = s.get(k).unwrap_or_default();
            let src = format!("{:?}", s.source(k)).to_lowercase();
            table.add_row(vec![Cell::new(*k), Cell::new(v), Cell::new(src)]);
        }
        let boards = if s.my_boards.is_empty() {
            "(empty)".to_string()
        } else {
            s.my_boards
                .iter()
                .map(|b| match &b.name {
                    Some(n) => format!("{} {n}", b.id),
                    None => b.id.to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let src = format!("{:?}", s.source("my_boards")).to_lowercase();
        table.add_row(vec![Cell::new("my_boards"), Cell::new(boards), Cell::new(src)]);
        println!("{}", table.to_string().lines().map(str::trim_end).collect::<Vec<_>>().join("\n"));
        println!("\nFile: {}", s.path.display());
    }
}

/// A borderless table. Colors are added by `Out::style`, so the table must not
/// remove them. On a terminal the table fits the width; in a pipe it is 160 wide.
fn new_table() -> Table {
    let mut table = Table::new();
    table.load_style(presets::NOTHING).set_content_arrangement(ContentArrangement::Dynamic);
    let tty = std::io::stdout().is_terminal();
    if !tty {
        table.force_no_tty().set_width(160);
    }
    table
}

pub fn print_warning(msg: &str) {
    let w = if stderr_color() { "warning:".yellow().bold().to_string() } else { "warning:".into() };
    eprintln!("{w} {msg}");
}

/// Write an error to stderr: text, or one JSON object when `json` is set.
pub fn print_error(e: &Error, json: bool) {
    if matches!(e, Error::Cancelled) {
        eprintln!("Cancelled.");
        return;
    }
    let fields = match e {
        Error::Validation { fields, .. } => fields.clone(),
        _ => Default::default(),
    };
    let mut err = std::io::stderr().lock();
    if json {
        let v = serde_json::json!({ "error": {
            "kind": e.kind(),
            "message": e.to_string(),
            "fields": fields,
            "hint": e.hint(),
        }});
        let _ = writeln!(err, "{v}");
        return;
    }
    let head = if stderr_color() { "error:".red().bold().to_string() } else { "error:".into() };
    let _ = writeln!(err, "{head} {e}");
    for (f, m) in &fields {
        let _ = writeln!(err, "  {f}: {m}");
    }
    if let Some(h) = e.hint() {
        let _ = writeln!(err, "  hint: {h}");
    }
}
