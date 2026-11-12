//! Display mode: wiki markup to styled lines, for the CLI `view` and the TUI.
//! The output has no terminal codes. The CLI and the TUI apply the styles.

use amdc::format::Format;
use amdc::ir::{Block, Inline, Mark, Media, PanelKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub dim: bool,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub spans: Vec<Span>,
}

impl Line {
    fn push(&mut self, text: impl Into<String>, style: Style) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.spans.last_mut()
            && last.style == style
        {
            last.text.push_str(&text);
            return;
        }
        self.spans.push(Span { text, style });
    }

    /// The text with no styles.
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// Convert wiki markup to styled lines. If the parse fails, the raw text is shown.
pub fn display(wiki: &str) -> Vec<Line> {
    let wiki = fix_for_display(&split_code_tags(&super::normalize(wiki)));
    let Ok(doc) = amdc::parse(&wiki, Format::JiraWiki) else {
        return wiki.lines().map(|l| Line { spans: vec![Span { text: l.into(), style: Style::default() }] }).collect();
    };
    let mut r = Renderer { lines: Vec::new() };
    r.blocks(&doc.content, "", "");
    while r.lines.last().is_some_and(|l| l.spans.is_empty()) {
        r.lines.pop();
    }
    r.lines
}

/// Repair two amdc parse defects, for display only:
/// - An empty panel on one line (`{panel:title=T}{panel}`) becomes a panel block.
/// - A link inside a table row (`|[text|url]|`): amdc splits the cell at the `|` of
///   the link. jui writes the link as `[text>url]`, a form that amdc keeps in one cell,
///   and restores it after the parse.
fn fix_for_display(wiki: &str) -> String {
    wiki.split('\n')
        .map(|line| {
            let t = line.trim();
            if t.starts_with("{panel") && t.ends_with("{panel}") && t.len() > 7 && t.matches('{').count() == 2 {
                let open = &t[..t.len() - 7];
                return format!("{open}\n{{panel}}");
            }
            if t.starts_with('|') {
                return protect_table_links(line);
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Markers (private-use characters) that replace `[`, `|`, and `]` of a link
/// inside a table cell, so that amdc does not split the cell.
const LINK_OPEN: char = '\u{E010}';
const LINK_BAR: char = '\u{E011}';
const LINK_CLOSE: char = '\u{E012}';

fn protect_table_links(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '['
            && let Some(len) = chars[i + 1..].iter().position(|&c| c == ']')
            && chars[i + 1..i + 1 + len].contains(&'|')
        {
            out.push(LINK_OPEN);
            for &c in &chars[i + 1..i + 1 + len] {
                out.push(if c == '|' { LINK_BAR } else { c });
            }
            out.push(LINK_CLOSE);
            i += len + 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// After the parse: write the protected links in a cell text.
fn restore_table_link(text: &str, line: &mut Line, base: Style) {
    let mut rest = text;
    while let Some(open) = rest.find(LINK_OPEN) {
        let after = &rest[open + LINK_OPEN.len_utf8()..];
        let Some(close) = after.find(LINK_CLOSE) else { break };
        let inner = &after[..close];
        let (label, href) = inner.split_once(LINK_BAR).unwrap_or((inner, ""));
        line.push(&rest[..open], base);
        line.push(label.replace(LINK_BAR, "|"), Style { underline: true, color: Some(Color::Blue), ..base });
        if !href.is_empty() {
            line.push(format!(" ({})", href.replace(LINK_BAR, "|")), Style { dim: true, ..base });
        }
        rest = &after[close + LINK_CLOSE.len_utf8()..];
    }
    line.push(rest, base);
}

/// Jira treats `{code}` and `{noformat}` tags as block tags at any place in a line.
/// amdc needs them on their own lines. Only for display: edit mode keeps the text.
fn split_code_tags(wiki: &str) -> String {
    let mut out = String::with_capacity(wiki.len());
    let mut rest = wiki;
    let mut inside: Option<&str> = None; // the tag name of the open block
    while !rest.is_empty() {
        let next = ["{code", "{noformat}"]
            .iter()
            .filter(|t| inside.is_none_or(|open| t.starts_with(&format!("{{{open}"))))
            .filter_map(|t| rest.find(t).map(|p| (p, *t)))
            .min_by_key(|(p, _)| *p);
        let Some((pos, tag)) = next else {
            out.push_str(rest);
            break;
        };
        let Some(close) = rest[pos..].find('}') else {
            out.push_str(rest);
            break;
        };
        let full = &rest[pos..pos + close + 1];
        // `{code:...}` tags must be `{code}` or `{code:` to be a tag.
        let is_tag = full == "{code}" || full.starts_with("{code:") || full == "{noformat}";
        out.push_str(&rest[..pos]);
        if is_tag {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(full);
            let after = &rest[pos + full.len()..];
            if !after.is_empty() && !after.starts_with('\n') {
                out.push('\n');
            }
            inside = match inside {
                None => Some(if tag == "{noformat}" { "noformat" } else { "code" }),
                Some(_) => None,
            };
        } else {
            out.push_str(full);
        }
        rest = &rest[pos + full.len()..];
    }
    out
}

struct Renderer {
    lines: Vec<Line>,
}

fn hex_color(hex: &str) -> Option<Color> {
    let h = hex.trim_start_matches('#').to_lowercase();
    let named = match h.as_str() {
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "blue" => Some(Color::Blue),
        "yellow" | "orange" => Some(Color::Yellow),
        "purple" | "magenta" => Some(Color::Magenta),
        "cyan" | "teal" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if h.len() != 6 {
        return None;
    }
    let v = |s: &str| u8::from_str_radix(s, 16).ok().map(i32::from);
    let (r, g, b) = (v(&h[0..2])?, v(&h[2..4])?, v(&h[4..6])?);
    let max = r.max(g).max(b);
    if max - r.min(g).min(b) < 40 {
        return Some(Color::Gray);
    }
    Some(if r == max && g > 150 {
        Color::Yellow
    } else if r == max && b > 150 {
        Color::Magenta
    } else if r == max {
        Color::Red
    } else if g == max && b > 150 {
        Color::Cyan
    } else if g == max {
        Color::Green
    } else {
        Color::Blue
    })
}

fn media_label(m: &Media) -> String {
    let name = m.url.as_deref().or(m.alt.as_deref()).or(m.id.as_deref()).unwrap_or("?");
    format!("[image: {name}]")
}

impl Renderer {
    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }

    /// Render blocks. `first` is the prefix for the first line, `rest` for the others.
    fn blocks(&mut self, blocks: &[Block], first: &str, rest: &str) {
        let mut prefix = first.to_string();
        for (n, b) in blocks.iter().enumerate() {
            if n > 0 && !matches!(b, Block::BulletList(_) | Block::OrderedList { .. }) && rest.is_empty() {
                self.blank();
            }
            self.block(b, &prefix, rest);
            prefix = rest.to_string();
        }
    }

    fn inline_lines(&mut self, inlines: &[Inline], first: &str, rest: &str, base: Style) {
        let mut line = Line::default();
        line.push(first, Style { dim: true, ..Style::default() });
        for i in inlines {
            match i {
                Inline::HardBreak => {
                    self.lines.push(std::mem::take(&mut line));
                    line.push(rest, Style { dim: true, ..Style::default() });
                }
                other => self.inline(other, &mut line, base),
            }
        }
        self.lines.push(line);
    }

    fn inline(&mut self, i: &Inline, line: &mut Line, base: Style) {
        match i {
            Inline::Text { text, marks } => {
                let mut st = base;
                let mut link = None;
                for m in marks {
                    match m {
                        Mark::Strong => st.bold = true,
                        Mark::Em => st.italic = true,
                        Mark::Code => {
                            st.code = true;
                            st.color = Some(Color::Yellow);
                        }
                        Mark::Strike => st.strike = true,
                        Mark::Underline => st.underline = true,
                        Mark::Sub | Mark::Sup => {}
                        Mark::TextColor(c) => st.color = hex_color(c).or(st.color),
                        Mark::Link { href, .. } => link = Some(href.clone()),
                    }
                }
                match link {
                    Some(href) if href.starts_with('^') => {
                        line.push(format!("[attachment: {}]", href.trim_start_matches('^')), Style { color: Some(Color::Cyan), ..base });
                    }
                    Some(href) if href == *text => {
                        line.push(text.clone(), Style { underline: true, color: Some(Color::Blue), ..st });
                    }
                    Some(href) => {
                        line.push(text.clone(), Style { underline: true, color: Some(Color::Blue), ..st });
                        line.push(format!(" ({href})"), Style { dim: true, ..base });
                    }
                    None => line.push(text.clone(), st),
                }
            }
            Inline::HardBreak => {}
            Inline::Image(m) => line.push(media_label(m), Style { color: Some(Color::Cyan), ..base }),
            Inline::Mention { id, text, .. } => {
                let name = id.as_deref().map(|i| format!("@{i}")).unwrap_or_else(|| text.clone());
                line.push(name, Style { color: Some(Color::Cyan), bold: true, ..base });
            }
            Inline::Emoji { short_name, text, .. } => line.push(text.clone().unwrap_or_else(|| short_name.clone()), base),
            Inline::Status { text, .. } => line.push(format!("[{text}]"), Style { bold: true, ..base }),
            Inline::Date { timestamp } => line.push(timestamp.clone(), base),
            Inline::InlineCard { url } => line.push(url.clone(), Style { underline: true, color: Some(Color::Blue), ..base }),
            Inline::FootnoteReference { label } => line.push(format!("[{label}]"), base),
            Inline::Unknown(u) => {
                let shown = if u.kind == "anchor" { String::new() } else { format!("{{{}}}", u.kind) };
                line.push(shown, Style { dim: true, ..base });
            }
        }
    }

    fn block(&mut self, b: &Block, first: &str, rest: &str) {
        let plain = Style::default();
        match b {
            Block::Paragraph(inl) => self.inline_lines(inl, first, rest, plain),
            Block::Heading { content, .. } => {
                self.inline_lines(content, first, rest, Style { bold: true, underline: true, ..plain })
            }
            Block::CodeBlock { language, text, .. } => {
                let mut l = Line::default();
                l.push(first, Style { dim: true, ..plain });
                l.push(format!("┌─ {}", language.as_deref().unwrap_or("code")), Style { dim: true, ..plain });
                self.lines.push(l);
                for t in text.lines() {
                    let mut l = Line::default();
                    l.push(format!("{rest}│ "), Style { dim: true, ..plain });
                    l.push(t, Style { color: Some(Color::Yellow), ..plain });
                    self.lines.push(l);
                }
                let mut l = Line::default();
                l.push(format!("{rest}└─"), Style { dim: true, ..plain });
                self.lines.push(l);
            }
            Block::Blockquote(inner) => {
                self.blocks(inner, &format!("{first}│ "), &format!("{rest}│ "));
            }
            Block::BulletList(items) => {
                for item in items {
                    self.blocks(item, &format!("{rest}• "), &format!("{rest}  "));
                }
                if rest.is_empty() {
                    self.blank();
                }
            }
            Block::OrderedList { start, items } => {
                for (n, item) in items.iter().enumerate() {
                    let marker = format!("{}. ", *start as usize + n);
                    let pad = " ".repeat(marker.chars().count());
                    self.blocks(item, &format!("{rest}{marker}"), &format!("{rest}{pad}"));
                }
                if rest.is_empty() {
                    self.blank();
                }
            }
            Block::TaskList { items, .. } => {
                for t in items {
                    let mark = if t.checked { "☑ " } else { "☐ " };
                    self.inline_lines(&t.content, &format!("{rest}{mark}"), &format!("{rest}  "), plain);
                }
            }
            Block::Table(t) => {
                for row in &t.rows {
                    let mut l = Line::default();
                    l.push(rest, plain);
                    for (n, cell) in row.cells.iter().enumerate() {
                        if n > 0 {
                            l.push(" │ ", Style { dim: true, ..plain });
                        }
                        let st = Style { bold: row.header, ..plain };
                        let mut cl = Line::default();
                        for blk in &cell.content {
                            if let Block::Paragraph(inl) = blk {
                                for i in inl {
                                    match i {
                                        Inline::Text { text, .. } if text.contains(LINK_OPEN) => {
                                            restore_table_link(text, &mut cl, st)
                                        }
                                        _ => self.inline(i, &mut cl, st),
                                    }
                                }
                            }
                        }
                        for sp in cl.spans {
                            l.push(sp.text, sp.style);
                        }
                    }
                    self.lines.push(l);
                }
            }
            Block::Rule => {
                let mut l = Line::default();
                l.push(format!("{rest}────────"), Style { dim: true, ..plain });
                self.lines.push(l);
            }
            Block::Panel { kind, title, content, .. } => {
                let label = title.clone().unwrap_or_else(|| match kind {
                    PanelKind::Info => "Info".into(),
                    PanelKind::Note => "Note".into(),
                    PanelKind::Warning => "Warning".into(),
                    PanelKind::Success => "Success".into(),
                    PanelKind::Error => "Error".into(),
                    PanelKind::Custom => String::new(),
                });
                let mut l = Line::default();
                l.push(format!("{first}╭─ "), Style { dim: true, ..plain });
                l.push(label, Style { bold: true, ..plain });
                self.lines.push(l);
                self.blocks(content, &format!("{rest}│ "), &format!("{rest}│ "));
                let mut l = Line::default();
                l.push(format!("{rest}╰─"), Style { dim: true, ..plain });
                self.lines.push(l);
            }
            Block::Expand { title, content, .. } => {
                let mut l = Line::default();
                l.push(format!("{first}▸ {}", title.as_deref().unwrap_or("")), Style { bold: true, ..plain });
                self.lines.push(l);
                self.blocks(content, &format!("{rest}  "), &format!("{rest}  "));
            }
            Block::MediaSingle { media, .. } => {
                let mut l = Line::default();
                l.push(first, plain);
                l.push(media_label(media), Style { color: Some(Color::Cyan), ..plain });
                self.lines.push(l);
            }
            Block::MediaGroup(ms) => {
                for m in ms {
                    let mut l = Line::default();
                    l.push(rest, plain);
                    l.push(media_label(m), Style { color: Some(Color::Cyan), ..plain });
                    self.lines.push(l);
                }
            }
            Block::FootnoteDefinition { label, content } => {
                self.blocks(content, &format!("{first}[{label}] "), rest);
            }
            Block::RawHtml(h) => {
                let mut l = Line::default();
                l.push(format!("{first}{h}"), Style { dim: true, ..plain });
                self.lines.push(l);
            }
            Block::Unknown(u) => {
                for (n, t) in u.raw.lines().enumerate() {
                    let mut l = Line::default();
                    l.push(if n == 0 { first } else { rest }, plain);
                    l.push(t, Style { dim: true, ..plain });
                    self.lines.push(l);
                }
            }
        }
    }
}
