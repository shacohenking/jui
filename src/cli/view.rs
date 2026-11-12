//! `jui view`: show one Ticket in the terminal.

use chrono::Utc;
use owo_colors::{OwoColorize, Style as OStyle};

use super::args::CommentCount;
use crate::core::format::{format_when, human_duration, human_size};
use crate::core::markup::{self, Color, Line};
use crate::core::model::{StatusCategory, Text, Ticket};
use crate::core::settings::DateFormat;

pub struct ViewOpts {
    pub color: bool,
    pub wiki: bool,
    pub comments: CommentCount,
    pub date_format: DateFormat,
}

/// Render the Ticket as text lines.
pub fn render(t: &Ticket, o: &ViewOpts) -> String {
    let mut out = Vec::<String>::new();
    let st = |s: OStyle, text: &str| if o.color { text.style(s).to_string() } else { text.to_string() };
    let dim = OStyle::new().dimmed();
    let bold = OStyle::new().bold();
    let now = Utc::now();
    let when = |d: Option<chrono::DateTime<chrono::FixedOffset>>| {
        d.map(|d| format_when(d, o.date_format, now)).unwrap_or_default()
    };

    // Header block.
    out.push(format!("{}  {}", st(OStyle::new().cyan().bold(), &t.key), st(bold, &t.summary)));
    let status_style = match (t.status.as_str(), t.status_category) {
        ("Blocked", _) => OStyle::new().red(),
        (_, StatusCategory::Done) => OStyle::new().green(),
        (_, StatusCategory::InProgress) => OStyle::new().blue(),
        _ => OStyle::new(),
    };
    let mut meta = vec![st(status_style.bold(), &t.status), t.kind.clone()];
    if let Some(p) = &t.priority {
        meta.push(p.clone());
    }
    out.push(meta.join(&st(dim, " · ")));

    let field = |name: &str, value: String| format!("{} {}", st(dim, &format!("{name:<10}")), value);
    let user = |u: &Option<crate::core::model::User>| {
        u.as_ref().map(|u| format!("{} ({})", u.display_name, u.name)).unwrap_or_else(|| "Unassigned".into())
    };
    out.push(field("Assignee", user(&t.assignee)));
    out.push(field("Reporter", user(&t.reporter)));
    if t.watching || t.watch_count > 0 {
        let you = if t.watching { "you watch · " } else { "" };
        let n = t.watch_count;
        out.push(field("Watchers", format!("{you}{n} watcher{}", if n == 1 { "" } else { "s" })));
    }
    if let Some(p) = &t.parent {
        out.push(field("Parent", format!("{} {}", p.key, p.summary)));
    }
    if let Some(e) = &t.epic {
        out.push(field("Epic", format!("{} {}", e.key, e.name.as_deref().unwrap_or(""))));
    }
    if let Some(s) = &t.sprint {
        out.push(field("Sprint", format!("{} ({})", s.name, s.state.to_lowercase())));
    }
    if let Some(sp) = t.story_points {
        out.push(field("Points", format!("{sp}")));
    }
    if !t.labels.is_empty() {
        out.push(field("Labels", t.labels.join(", ")));
    }
    if !t.components.is_empty() {
        out.push(field("Components", t.components.join(", ")));
    }
    if !t.fix_versions.is_empty() {
        out.push(field("Fix ver.", t.fix_versions.join(", ")));
    }
    out.push(field("Created", when(t.created)));
    out.push(field("Updated", when(t.updated)));
    out.push(field("URL", t.url.clone()));

    // Description.
    out.push(String::new());
    match &t.description {
        Some(d) => out.extend(text(d, o)),
        None => out.push(st(dim, "(no description)")),
    }

    let section = |title: String| format!("\n{}", st(bold, &title));
    if !t.subtasks.is_empty() {
        out.push(section(format!("Sub-tasks ({})", t.subtasks.len())));
        for s in &t.subtasks {
            out.push(format!("  {:<12} {:<20} {}", s.key, s.status, s.summary));
        }
    }
    if !t.links.is_empty() {
        out.push(section(format!("Links ({})", t.links.len())));
        for l in &t.links {
            out.push(format!("  {:<18} {:<12} {:<20} {}", l.relation, l.ticket.key, l.ticket.status, l.ticket.summary));
        }
    }
    if !t.attachments.is_empty() {
        out.push(section(format!("Attachments ({})", t.attachments.len())));
        for a in &t.attachments {
            out.push(format!("  {}  {}", a.filename, st(dim, &human_size(a.size))));
        }
    }
    if !t.worklogs.is_empty() {
        let total: u64 = t.worklogs.iter().map(|w| w.time_spent_seconds).sum();
        out.push(section(format!("Worklogs ({}, total {})", t.worklogs.len(), human_duration(total))));
        for w in &t.worklogs {
            let who = w.author.as_ref().map(|u| u.name.as_str()).unwrap_or("");
            let day = w.started.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
            out.push(format!("  {day}  {:<8} {who}  {}", w.time_spent, w.comment.as_deref().unwrap_or("")));
        }
    }

    // Comments.
    let n = match o.comments {
        CommentCount::All => t.comments.len(),
        CommentCount::Last(n) => n.min(t.comments.len()),
    };
    if t.comment_total > 0 {
        let shown = if n < t.comments.len() || (t.comment_total as usize) > t.comments.len() {
            format!("Comments ({n} of {}, --comments all for all)", t.comment_total)
        } else {
            format!("Comments ({})", t.comment_total)
        };
        out.push(section(shown));
        out.push(comment_lines(&t.comments[t.comments.len() - n..], o));
    }
    out.join("\n")
}

/// The lines of comments: the author, the date, the ID, and the text.
fn comment_lines(comments: &[crate::core::model::Comment], o: &ViewOpts) -> String {
    let st = |s: OStyle, text: &str| if o.color { text.style(s).to_string() } else { text.to_string() };
    let dim = OStyle::new().dimmed();
    let now = Utc::now();
    let mut out = Vec::new();
    for c in comments {
        let who = c.author.as_ref().map(|u| u.name.as_str()).unwrap_or("?");
        let when = c.created.map(|d| format_when(d, o.date_format, now)).unwrap_or_default();
        let edited = match (c.created, c.updated) {
            (Some(a), Some(b)) if b > a => "edited, ",
            _ => "",
        };
        out.push(String::new());
        out.push(format!(
            "  {}  {}  {}",
            st(OStyle::new().cyan(), &format!("@{who}")),
            st(dim, &format!("#{}", c.id)),
            st(dim, &format!("{edited}{when}"))
        ));
        for l in text(&c.body, o) {
            out.push(format!("    {l}"));
        }
    }
    out.join("\n")
}

/// `jui comment list`: all comments, oldest first.
pub fn render_comments(comments: &[crate::core::model::Comment], color: bool, wiki: bool, date_format: DateFormat) -> String {
    let o = ViewOpts { color, wiki, comments: CommentCount::All, date_format };
    let title = format!("Comments ({})", comments.len());
    let title = if color { title.style(OStyle::new().bold()).to_string() } else { title };
    format!("{title}{}", comment_lines(comments, &o))
}

fn text(t: &Text, o: &ViewOpts) -> Vec<String> {
    if o.wiki {
        return t.wiki.lines().map(str::to_string).collect();
    }
    markup::display(&t.wiki).iter().map(|l| styled(l, o.color)).collect()
}

/// Apply display styles as terminal codes.
pub fn styled(l: &Line, color: bool) -> String {
    if !color {
        return l.plain();
    }
    l.spans
        .iter()
        .map(|s| {
            let mut st = OStyle::new();
            let x = s.style;
            if x.bold {
                st = st.bold();
            }
            if x.italic {
                st = st.italic();
            }
            if x.underline {
                st = st.underline();
            }
            if x.strike {
                st = st.strikethrough();
            }
            if x.dim {
                st = st.dimmed();
            }
            st = match x.color {
                Some(Color::Red) => st.red(),
                Some(Color::Green) => st.green(),
                Some(Color::Yellow) => st.yellow(),
                Some(Color::Blue) => st.blue(),
                Some(Color::Magenta) => st.magenta(),
                Some(Color::Cyan) => st.cyan(),
                Some(Color::Gray) => st.bright_black(),
                None => st,
            };
            s.text.style(st).to_string()
        })
        .collect()
}
