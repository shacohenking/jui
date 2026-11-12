//! Conversion between Jira wiki markup and Markdown.
//! See ticket 09 "Edit policy for Markdown text and wiki markup" and ADR 0001.
//!
//! - Edit mode (`wiki_to_markdown`, `markdown_to_wiki`): Markdown with Raw wiki
//!   pieces. Elements with no clean Markdown form stay as the original wiki text,
//!   in a fenced `jira-wiki` block or a `jira:` code span. They go back unchanged.
//! - Display mode (`display`): styled lines for the CLI and the TUI.
//!
//! All use of `amdc` is in this module, so jui can replace it.

mod display;
mod scan;

pub use display::{Color, Line, Span, Style, display};

use amdc::format::{Format, Options};

/// The result of a conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub text: String,
    /// Losses that the conversion reports, for example "table column alignment".
    pub warnings: Vec<String>,
}

/// A piece of text that jui protects from `amdc` during a conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Piece {
    /// Raw wiki text shown as a `jira:` span or a `jira-wiki` block.
    Raw { raw: String, block: bool },
    /// A mention `[~name]`, shown as `@name`.
    Mention(String),
    /// Text that must pass through unchanged (for example `--`).
    Literal { wiki: String, markdown: String },
}

const TOKEN_PREFIX: &str = "JUIRAW";
const TOKEN_SUFFIX: &str = "Q";

fn token(i: usize) -> String {
    format!("{TOKEN_PREFIX}{i}{TOKEN_SUFFIX}")
}

/// Replace tokens in `text` with the output of `f`. Longer indexes first, so
/// `JUIRAW1Q` does not match inside `JUIRAW11Q`.
fn substitute(text: &str, pieces: &[Piece], f: impl Fn(&Piece) -> String) -> String {
    let mut out = text.to_string();
    for (i, p) in pieces.iter().enumerate().rev() {
        out = out.replace(&token(i), &f(p));
    }
    out
}

fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn warnings(ws: &[amdc::error::Warning]) -> Vec<String> {
    ws.iter()
        .map(|w| match &w.detail {
            Some(d) => format!("{:?}: {} ({d})", w.kind, w.construct).to_lowercase(),
            None => format!("{:?}: {}", w.kind, w.construct).to_lowercase(),
        })
        .collect()
}

/// Convert wiki markup to Markdown with Raw wiki pieces (edit mode).
pub fn wiki_to_markdown(wiki: &str) -> Converted {
    let wiki = normalize(wiki);
    if wiki.contains(TOKEN_PREFIX) {
        // The text contains the token text. Do not risk a wrong substitution.
        return Converted { text: raw_block(&wiki), warnings: vec!["kept the full text as raw wiki markup".into()] };
    }
    let (protected, pieces) = scan::protect_wiki(&wiki);
    let out = match amdc::convert(&protected, Format::JiraWiki, Format::Gfm, &Options::default()) {
        Ok(o) => o,
        Err(e) => {
            return Converted { text: raw_block(&wiki), warnings: vec![format!("not converted: {e}")] };
        }
    };
    let protected_out = merge_adjacent_spans(&out.text, &pieces);
    let (merged_text, merged_pieces) = (&protected_out.0, &protected_out.1);
    let text = substitute(merged_text, merged_pieces, |p| match p {
        Piece::Raw { raw, block: true } => raw_block(raw),
        Piece::Raw { raw, block: false } => raw_span(raw),
        Piece::Mention(name) => format!("@{name}"),
        Piece::Literal { markdown, .. } => markdown.clone(),
    });
    // amdc writes `adf:` payloads for elements that it cannot show. Report them.
    let mut ws: Vec<String> = warnings(&out.warnings).into_iter().filter(|w| !w.starts_with("escaped")).collect();
    if text.contains("`adf:") || text.contains("```adf:") {
        ws.push("some elements are shown as adf payloads".into());
    }
    Converted { text: text.trim_end().to_string(), warnings: ws }
}

/// Convert Markdown with Raw wiki pieces to wiki markup.
pub fn markdown_to_wiki(markdown: &str) -> Converted {
    let md = normalize(markdown);
    if md.contains(TOKEN_PREFIX) {
        return Converted { text: md, warnings: vec!["the text contains JUIRAW and was sent unchanged".into()] };
    }
    let (protected, pieces) = scan::protect_markdown(&md);
    let out = match amdc::convert(&protected, Format::Gfm, Format::JiraWiki, &Options::default()) {
        Ok(o) => o,
        Err(e) => return Converted { text: md, warnings: vec![format!("not converted: {e}")] },
    };
    // amdc escapes dashes that Jira does not need escaped.
    let fixed = out.text.replace("\\-\\-", "--").replace("\\--", "--");
    let fixed = unescape_bang(&fixed);
    let text = substitute(&fixed, &pieces, |p| match p {
        Piece::Raw { raw, .. } => raw.clone(),
        Piece::Mention(name) => format!("[~{name}]"),
        Piece::Literal { wiki, .. } => wiki.clone(),
    });
    Converted { text: text.trim_end().to_string(), warnings: warnings(&out.warnings) }
}

/// Join inline Raw wiki pieces that touch, for example `{panel:x}{panel}`. Two `jira:`
/// spans side by side give ``` `a``b` ```, and Markdown reads the double backtick as one
/// code span. One span with the joined wiki text has the same result, and it goes back
/// unchanged.
fn merge_adjacent_spans(text: &str, pieces: &[Piece]) -> (String, Vec<Piece>) {
    let mut pieces = pieces.to_vec();
    let mut text = text.to_string();
    loop {
        let mut changed = false;
        for i in 0..pieces.len() {
            for j in 0..pieces.len() {
                if i == j {
                    continue;
                }
                let (Piece::Raw { raw: a, block: false }, Piece::Raw { raw: b, block: false }) = (&pieces[i], &pieces[j]) else {
                    continue;
                };
                let pair = format!("{}{}", token(i), token(j));
                if !text.contains(&pair) {
                    continue;
                }
                let joined = format!("{a}{b}");
                text = text.replacen(&pair, &token(i), 1);
                pieces[i] = Piece::Raw { raw: joined, block: false };
                changed = true;
                break;
            }
            if changed {
                break;
            }
        }
        if !changed {
            return (text, pieces);
        }
    }
}

/// Remove the escape of a `!` that has a space, a line end, or the text end after it.
/// In wiki markup, `!` starts an image only when a non-space character follows it, so
/// such a `!` cannot start an image. A `!` with a non-space character after it keeps its
/// escape; thus no image can start, and a `!` that could end an image has no start.
fn unescape_bang(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && chars.get(i + 1) == Some(&'!') && chars.get(i + 2).is_none_or(|c| c.is_whitespace()) {
            out.push('!');
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A fenced `jira-wiki` block. The fence is longer than any backtick run in the text.
fn raw_block(raw: &str) -> String {
    let fence = "`".repeat(longest_run(raw, '`').max(2) + 1);
    format!("{fence}jira-wiki\n{raw}\n{fence}")
}

/// A `jira:` code span. Multi-line text becomes a block.
fn raw_span(raw: &str) -> String {
    if raw.contains('\n') {
        return raw_block(raw);
    }
    let ticks = "`".repeat(longest_run(raw, '`') + 1);
    let pad = if raw.ends_with('`') { " " } else { "" };
    format!("{ticks}jira:{raw}{pad}{ticks}")
}

fn longest_run(s: &str, c: char) -> usize {
    let (mut best, mut cur) = (0, 0);
    for ch in s.chars() {
        if ch == c {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

#[cfg(test)]
mod tests;
