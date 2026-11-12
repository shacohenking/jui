//! The comment Template and the add and edit logic. See ticket 08 "Template format for
//! create and edit", part 5, and ticket 09 "Edit policy for Markdown text and wiki markup".
//!
//! A comment Template has no header. The body is the comment in Markdown (or raw wiki
//! markup with `--wiki`). The reference part below the scissors line shows the key, the
//! summary, and the last 3 comments, for context. jui ignores it.
//!
//! All functions are pure. A `CommentSession` keeps the state of one add or edit. The CLI
//! and the TUI drive the same session; they only start the editor and send the requests.

use super::error::Error;
use super::markup;
use super::model::{Comment, CommentContext};
use super::template::{self as tpl, SCISSORS, TemplateError, content, norm};

/// The number of earlier comments that the reference part shows.
pub const CONTEXT_COMMENTS: usize = 3;
/// The maximum number of lines of each earlier comment in the reference part.
const CONTEXT_LINES: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentMode {
    /// `wiki`: the body is raw wiki markup (`comment add --wiki`).
    Add { wiki: bool },
    /// Edit comment `id`. `old` is its wiki text. `wiki`: the body is raw wiki markup.
    Edit { id: String, old: String, wiki: bool },
}

/// The request that jui sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentPlan {
    /// `None` for a new comment.
    pub id: Option<String>,
    /// The new body in wiki markup.
    pub wiki: String,
    /// The old body in wiki markup (edit only), for the diff.
    pub old: Option<String>,
    /// Losses that the Markdown-to-wiki conversion reports.
    pub warnings: Vec<String>,
}

impl CommentPlan {
    /// True when jui must show a diff: the change replaces existing text.
    pub fn needs_diff(&self) -> bool {
        self.old.as_deref().is_some_and(|o| !o.trim().is_empty())
    }
}

/// What the caller must do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentStep {
    /// Stop with no request. The text is the message.
    Cancel(String),
    /// Open the editor again with `CommentSession::text()`. It has the error lines.
    Reopen,
    /// Stop and write a Draft. The text is the reason.
    Stop(String),
    Send(CommentPlan),
}

/// The state of one comment add or edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentSession {
    pub key: String,
    pub mode: CommentMode,
    /// The text for the next editor start.
    shown: String,
    errors: Vec<TemplateError>,
    /// The last text that the user saved.
    saved: Option<String>,
    /// Edit: the body that jui wrote (normalized).
    base_body: String,
}

impl CommentSession {
    /// Start a new comment. `text` is the first body (Markdown), for example from `-m`
    /// or from the TUI popup. It can be empty.
    pub fn add(ctx: &CommentContext, text: &str, wiki: bool) -> Self {
        let shown = template(ctx, None, text, wiki);
        CommentSession { key: ctx.key.clone(), mode: CommentMode::Add { wiki }, shown, errors: vec![], saved: None, base_body: String::new() }
    }

    /// Start the edit of comment `id`. In raw wiki mode, the body is the wiki markup.
    pub fn edit(ctx: &CommentContext, id: &str, wiki: bool) -> Result<Self, Error> {
        let c = ctx.find(id).ok_or_else(|| Error::NotFound(format!("Comment {id} on {}", ctx.key)))?;
        let body = if wiki { norm(&c.body.wiki) } else { norm(&markup::wiki_to_markdown(&c.body.wiki).text) };
        let shown = template(ctx, Some(c), &body, wiki);
        let mode = CommentMode::Edit { id: id.to_string(), old: c.body.wiki.clone(), wiki };
        Ok(CommentSession { key: ctx.key.clone(), mode, shown, errors: vec![], saved: None, base_body: body })
    }

    /// The text for the editor.
    pub fn text(&self) -> &str {
        &self.shown
    }

    pub fn errors(&self) -> &[TemplateError] {
        &self.errors
    }

    /// The text for a Draft: the last text that the user saved, or the Template.
    pub fn draft_text(&self) -> &str {
        self.saved.as_deref().unwrap_or(&self.shown)
    }

    /// The name part of the temporary file and of the Draft.
    pub fn label(&self) -> String {
        match &self.mode {
            CommentMode::Add { .. } => format!("{}-comment", self.key),
            CommentMode::Edit { id, .. } => format!("{}-comment-{id}", self.key),
        }
    }

    /// The `jui` command that uses a Draft of this session again (without `--from`).
    pub fn command(&self) -> String {
        match &self.mode {
            CommentMode::Add { wiki: true } => format!("comment add {} --wiki", self.key),
            CommentMode::Add { .. } => format!("comment add {}", self.key),
            CommentMode::Edit { id, wiki: true, .. } => format!("comment edit {} {id} --wiki", self.key),
            CommentMode::Edit { id, .. } => format!("comment edit {} {id}", self.key),
        }
    }

    /// The comment ID (edit only).
    pub fn id(&self) -> Option<&str> {
        match &self.mode {
            CommentMode::Add { .. } => None,
            CommentMode::Edit { id, .. } => Some(id),
        }
    }

    /// Check the text that the user saved, or a text from `--from`.
    pub fn submit(&mut self, saved: &str) -> CommentStep {
        if !self.errors.is_empty() && content(saved) == content(&self.shown) {
            self.saved = Some(saved.to_string());
            return CommentStep::Stop("The Template is not valid, and you did not change it.".into());
        }
        let parsed = match tpl::parse(saved) {
            Ok(p) => p,
            Err(e) => return self.reopen(saved, vec![e]),
        };
        if !parsed.header.is_empty() {
            let e = TemplateError { key: None, message: "A comment has no header. Remove the +++ lines and the lines between them.".into() };
            return self.reopen(saved, vec![e]);
        }
        let body = parsed.body;
        if body.trim().is_empty() {
            return CommentStep::Cancel(match &self.mode {
                CommentMode::Add { .. } => "Cancelled. No comment added.".into(),
                CommentMode::Edit { id, .. } => format!("Cancelled. Comment {id} did not change."),
            });
        }
        self.saved = Some(saved.to_string());
        let plan = match &self.mode {
            CommentMode::Add { wiki } => to_plan(&body, *wiki).expect("the body is not empty"),
            CommentMode::Edit { id, old, wiki } => {
                if norm(&body) == self.base_body {
                    return CommentStep::Cancel("No changes.".into());
                }
                let (new, warnings) = if *wiki {
                    (norm(&body), vec![])
                } else {
                    let c = markup::markdown_to_wiki(&body);
                    (c.text, c.warnings)
                };
                if norm(&new) == norm(old) {
                    return CommentStep::Cancel("No changes.".into());
                }
                CommentPlan { id: Some(id.clone()), wiki: new, old: Some(old.clone()), warnings }
            }
        };
        self.errors.clear();
        CommentStep::Send(plan)
    }

    fn reopen(&mut self, saved: &str, errs: Vec<TemplateError>) -> CommentStep {
        self.saved = Some(saved.to_string());
        self.shown = errors_on_top(saved, &errs);
        self.errors = errs;
        CommentStep::Reopen
    }

    /// Open the editor again with a note at the top (after `n` in the diff question).
    pub fn reopen_note(&mut self, note: &str) {
        let saved = self.saved.clone().unwrap_or_else(|| self.shown.clone());
        self.shown = errors_on_top(&saved, &[TemplateError { key: None, message: note.into() }]);
        self.errors.clear();
    }

    /// Jira did not accept the request. A validation error opens the Template again with
    /// error lines. Other errors come back unchanged.
    pub fn rejected(&mut self, e: Error) -> Result<CommentStep, Error> {
        let Error::Validation { fields, messages } = e else { return Err(e) };
        let mut errs: Vec<TemplateError> = messages.into_iter().map(|m| TemplateError { key: None, message: m }).collect();
        errs.extend(fields.into_iter().map(|(k, m)| TemplateError { key: Some(k), message: m }));
        if errs.is_empty() {
            errs.push(TemplateError { key: None, message: "Jira did not accept the comment.".into() });
        }
        let saved = self.saved.clone().unwrap_or_else(|| self.shown.clone());
        Ok(self.reopen(&saved, errs))
    }
}

/// Put error lines at the top of the text. A comment has no header, so the errors cannot
/// go below a `+++` line. jui removes the old error lines first.
fn errors_on_top(text: &str, errs: &[TemplateError]) -> String {
    let mut out: Vec<String> = errs.iter().map(|e| format!("{} {}", tpl::ERROR_PREFIX, e.text())).collect();
    out.extend(text.lines().filter(|l| !l.starts_with(tpl::ERROR_PREFIX)).map(str::to_string));
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// The request for a new comment from a popup or from the command line. `None` when the
/// text is empty. With `wiki`, jui sends the text with no conversion.
pub fn to_plan(text: &str, wiki: bool) -> Option<CommentPlan> {
    let t = norm(text);
    if t.is_empty() {
        return None;
    }
    if wiki {
        return Some(CommentPlan { id: None, wiki: t, old: None, warnings: vec![] });
    }
    let c = markup::markdown_to_wiki(&t);
    Some(CommentPlan { id: None, wiki: c.text, old: None, warnings: c.warnings })
}

/// The message when a comment changed in Jira during the edit. `None` when it did not change.
pub fn changed_in_jira(key: &str, plan: &CommentPlan, now: &Comment) -> Option<String> {
    let old = plan.old.as_deref()?;
    let id = plan.id.as_deref().unwrap_or("?");
    (norm(old) != norm(&now.body.wiki)).then(|| format!("Comment {id} on {key} changed in Jira while you edited it."))
}

/// The first line of a comment in display mode, shorter than `max` characters.
pub fn first_line(c: &Comment, max: usize) -> String {
    let line = markup::display(&c.body.wiki).iter().map(|l| l.plain()).find(|l| !l.trim().is_empty()).unwrap_or_default();
    let line = line.trim();
    if line.chars().count() > max { format!("{}…", line.chars().take(max.saturating_sub(1)).collect::<String>()) } else { line.to_string() }
}

/// The author username of a comment, or `?`.
pub fn author(c: &Comment) -> &str {
    c.author.as_ref().map(|u| u.name.as_str()).unwrap_or("?")
}

/// The Template text. `editing` is the comment that the user edits, or `None` for a new comment.
fn template(ctx: &CommentContext, editing: Option<&Comment>, body: &str, wiki: bool) -> String {
    let mut out: Vec<String> =
        vec![body.to_string(), String::new(), SCISSORS.to_string(), "# jui ignores this line and all lines below it.".into()];
    match editing {
        None => {
            out.push(format!("# Add a comment to {}: {}", ctx.key, ctx.summary));
            if wiki {
                out.push("# Write the comment above the scissors line, in raw Jira wiki markup.".into());
            } else {
                out.push("# Write the comment above the scissors line, in Markdown. @name mentions a user.".into());
            }
            out.push("# Save an empty text to cancel.".into());
        }
        Some(c) => {
            out.push(format!("# Edit comment {} by @{} on {}: {}", c.id, author(c), ctx.key, ctx.summary));
            if wiki {
                out.push("# The text is raw Jira wiki markup. jui sends it with no conversion.".into());
            } else {
                out.push("# The text is Markdown. @name mentions a user.".into());
            }
            out.push("# Save with no changes to cancel. jui shows the change before it sends it.".into());
        }
    }
    // The earlier comments: the last 3, or the 3 before the comment that the user edits.
    let end = match editing {
        Some(c) => ctx.comments.iter().position(|x| x.id == c.id).unwrap_or(ctx.comments.len()),
        None => ctx.comments.len(),
    };
    let start = end.saturating_sub(CONTEXT_COMMENTS);
    let earlier = &ctx.comments[start..end];
    if !earlier.is_empty() {
        out.push("#".into());
        let title = if editing.is_some() { "The comments before this comment" } else { "The last comments" };
        out.push(format!("# {title} ({} of {}):", earlier.len(), end));
        for c in earlier {
            out.push("#".into());
            let when = c.created.map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
            out.push(format!("# @{}  {when}", author(c)));
            let lines: Vec<String> = markup::display(&c.body.wiki).iter().map(|l| l.plain()).collect();
            for l in lines.iter().take(CONTEXT_LINES) {
                out.push(format!("#   {l}").trim_end().to_string());
            }
            if lines.len() > CONTEXT_LINES {
                out.push(format!("#   … ({} more lines)", lines.len() - CONTEXT_LINES));
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests;
