//! Unit tests for the comment Template. No test sends a request.

use super::*;
use crate::core::model::{Text, User};

fn comment(id: &str, who: &str, wiki: &str) -> Comment {
    Comment {
        id: id.into(),
        author: Some(User { name: who.into(), display_name: who.to_uppercase() }),
        created: crate::core::model::parse_jira_date("2026-10-12T09:00:00.000-0700"),
        updated: None,
        body: Text::from_wiki(wiki.into()),
    }
}

fn ctx() -> CommentContext {
    CommentContext {
        key: "BCE-1".into(),
        summary: "Retry on 429".into(),
        comments: vec![
            comment("1", "a", "First."),
            comment("2", "b", "Second with *bold*."),
            comment("3", "c", "Third."),
            comment("4", "d", "Please ask [~jdoe] about {color:red}this{color}."),
        ],
    }
}

/// The text above the scissors line is the body. Replace it.
fn with_body(template: &str, body: &str) -> String {
    let at = template.find(SCISSORS).unwrap();
    format!("{body}\n\n{}", &template[at..])
}

#[test]
fn add_template_shows_the_last_three_comments() {
    let s = CommentSession::add(&ctx(), "", false);
    let t = s.text();
    assert!(t.starts_with("\n\n# ----"), "the body is empty: {t:?}");
    assert!(t.contains("# Add a comment to BCE-1: Retry on 429"));
    assert!(t.contains("# The last comments (3 of 4):"));
    assert!(!t.contains("First."), "only the last 3");
    assert!(t.contains("#   Second with bold."), "display mode: {t}");
    assert!(t.contains("# @d  2026-10-12 09:00"));
    assert!(t.contains("@jdoe"), "a mention shows as @name: {t}");
}

#[test]
fn add_converts_markdown_and_mentions() {
    let mut s = CommentSession::add(&ctx(), "", false);
    let saved = with_body(s.text(), "Hi @jdoe, see **this** and `code`.\n\nmail a@b.com");
    let CommentStep::Send(p) = s.submit(&saved) else { panic!() };
    assert_eq!(p.wiki, "Hi [~jdoe], see *this* and {{code}}.\n\nmail a@b.com");
    assert_eq!(p.id, None);
    assert!(!p.needs_diff(), "a new comment has no diff");
}

#[test]
fn mention_in_code_is_not_converted() {
    let p = to_plan("Run `@jdoe` now, @jdoe", false).unwrap();
    assert_eq!(p.wiki, "Run {{@jdoe}} now, [~jdoe]");
    assert!(to_plan("   ", false).is_none());
    assert_eq!(to_plan("h2. **raw**\n\n", true).unwrap().wiki, "h2. **raw**");
}

#[test]
fn empty_body_cancels() {
    let mut s = CommentSession::add(&ctx(), "", false);
    let text = s.text().to_string();
    assert_eq!(s.submit(&text), CommentStep::Cancel("Cancelled. No comment added.".into()));
    assert_eq!(s.submit(""), CommentStep::Cancel("Cancelled. No comment added.".into()));
}

#[test]
fn prefilled_text_from_the_popup_is_the_body() {
    let mut s = CommentSession::add(&ctx(), "Start of a long text", false);
    assert!(s.text().starts_with("Start of a long text\n"));
    let text = s.text().to_string();
    let CommentStep::Send(p) = s.submit(&text) else { panic!() };
    assert_eq!(p.wiki, "Start of a long text");
}

#[test]
fn edit_of_an_unchanged_comment_sends_nothing() {
    let mut s = CommentSession::edit(&ctx(), "4", false).unwrap();
    let t = s.text().to_string();
    assert!(t.starts_with("Please ask @jdoe about `jira:{color:red}this{color}`."), "{t}");
    assert!(t.contains("# Edit comment 4 by @d on BCE-1"));
    assert!(t.contains("# The comments before this comment (3 of 3):"));
    assert_eq!(s.submit(&t), CommentStep::Cancel("No changes.".into()));
    // Spaces at the line ends and blank lines at the end are not a change.
    let spaced = t.replacen("this{color}`.", "this{color}`.   \n\n", 1);
    assert_eq!(s.submit(&spaced), CommentStep::Cancel("No changes.".into()));
}

#[test]
fn edit_keeps_raw_wiki_pieces_and_shows_a_diff() {
    let mut s = CommentSession::edit(&ctx(), "4", false).unwrap();
    let saved = s.text().replacen("Please ask", "Please, ask", 1);
    let CommentStep::Send(p) = s.submit(&saved) else { panic!() };
    assert_eq!(p.id.as_deref(), Some("4"));
    assert_eq!(p.wiki, "Please, ask [~jdoe] about {color:red}this{color}.");
    assert_eq!(p.old.as_deref(), Some("Please ask [~jdoe] about {color:red}this{color}."));
    assert!(p.needs_diff());
}

#[test]
fn edit_in_wiki_mode_is_not_converted() {
    let mut s = CommentSession::edit(&ctx(), "2", true).unwrap();
    assert!(s.text().starts_with("Second with *bold*.\n"));
    assert!(s.text().contains("raw Jira wiki markup"));
    let saved = with_body(s.text(), "h2. **Not** Markdown");
    let CommentStep::Send(p) = s.submit(&saved) else { panic!() };
    assert_eq!(p.wiki, "h2. **Not** Markdown");
    assert_eq!(s.command(), "comment edit BCE-1 2 --wiki");
}

#[test]
fn edit_of_an_unknown_comment_is_not_found() {
    let e = CommentSession::edit(&ctx(), "99", false).unwrap_err();
    assert_eq!(e.to_string(), "Comment 99 on BCE-1 not found.");
}

#[test]
fn a_header_is_an_error_and_an_unchanged_save_stops() {
    let mut s = CommentSession::add(&ctx(), "", false);
    let saved = with_body(s.text(), "+++\nx = 1\n+++\nText");
    assert_eq!(s.submit(&saved), CommentStep::Reopen);
    assert!(s.text().starts_with("# ERROR: A comment has no header."), "{}", s.text());
    let again = s.text().to_string();
    assert!(matches!(s.submit(&again), CommentStep::Stop(_)));
    assert!(s.draft_text().contains("Text"));
}

#[test]
fn a_jira_validation_error_reopens_with_error_lines() {
    let mut s = CommentSession::add(&ctx(), "", false);
    let saved = with_body(s.text(), "Text");
    assert!(matches!(s.submit(&saved), CommentStep::Send(_)));
    let e = Error::Validation { fields: [("comment".to_string(), "Too long.".to_string())].into(), messages: vec![] };
    assert_eq!(s.rejected(e).unwrap(), CommentStep::Reopen);
    assert!(s.text().starts_with("# ERROR: comment: Too long.\nText"), "{}", s.text());
    // A changed text after the error lines is sent.
    let fixed = s.text().replace("Text", "Shorter");
    let CommentStep::Send(p) = s.submit(&fixed) else { panic!() };
    assert_eq!(p.wiki, "Shorter");
    assert!(s.rejected(Error::Timeout(3)).is_err(), "other errors come back");
}

#[test]
fn reopen_note_is_not_an_error() {
    let mut s = CommentSession::edit(&ctx(), "3", false).unwrap();
    let saved = with_body(s.text(), "Changed.");
    assert!(matches!(s.submit(&saved), CommentStep::Send(_)));
    s.reopen_note("You did not send the change.");
    assert!(s.text().starts_with("# ERROR: You did not send the change.\nChanged."));
    let again = s.text().to_string();
    assert!(matches!(s.submit(&again), CommentStep::Send(_)), "the same text gives the same request");
}

#[test]
fn labels_and_first_line() {
    let c = ctx();
    let s = CommentSession::edit(&c, "4", false).unwrap();
    assert_eq!(s.label(), "BCE-1-comment-4");
    assert_eq!(s.command(), "comment edit BCE-1 4");
    assert_eq!(CommentSession::add(&c, "", false).label(), "BCE-1-comment");
    assert_eq!(first_line(&c.comments[1], 100), "Second with bold.");
    assert_eq!(first_line(&c.comments[1], 7), "Second…");
}

#[test]
fn add_in_wiki_mode_is_not_converted() {
    let mut s = CommentSession::add(&ctx(), "", true);
    assert!(s.text().contains("raw Jira wiki markup"));
    let CommentStep::Send(p) = s.submit(&with_body(s.text(), "{code}x{code} **y**")) else { panic!() };
    assert_eq!(p.wiki, "{code}x{code} **y**");
    assert_eq!(s.command(), "comment add BCE-1 --wiki");
}

#[test]
fn a_comment_that_changed_in_jira_is_found() {
    let c = ctx();
    let mut s = CommentSession::edit(&c, "3", false).unwrap();
    let CommentStep::Send(p) = s.submit(&with_body(s.text(), "New.")) else { panic!() };
    assert_eq!(changed_in_jira("BCE-1", &p, &c.comments[2]), None);
    let other = comment("3", "c", "Third, changed by c.");
    assert_eq!(changed_in_jira("BCE-1", &p, &other).as_deref(), Some("Comment 3 on BCE-1 changed in Jira while you edited it."));
}
