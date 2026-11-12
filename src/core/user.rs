//! The user rules for `assign`, `-a`, and the watcher commands. See ticket 05
//! "jui CLI command tree", part 6:
//!
//! - `me` is the current user.
//! - `none` removes the assignee (only for assign).
//! - Other text: jui searches the users. One match: jui uses it. More than one match: on a
//!   terminal, jui asks; in a script, jui stops with an error.
//!
//! This module has no I/O. The CLI and the TUI give it the search results.

use super::error::{Error, Result};
use super::model::User;

/// A user argument from the command line or a picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserArg {
    /// `me`: the current user.
    Me,
    /// `none`: no user (unassigned).
    Nobody,
    /// A text to search for.
    Text(String),
}

impl UserArg {
    pub fn parse(s: &str) -> Self {
        let t = s.trim();
        match t.to_lowercase().as_str() {
            "me" | "@me" => UserArg::Me,
            "none" | "nobody" | "unassigned" | "-" => UserArg::Nobody,
            // `@jdoe` is the same as `jdoe`.
            _ => UserArg::Text(t.trim_start_matches('@').to_string()),
        }
    }
}

/// The assignee to set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignee {
    Me,
    Nobody,
    User(User),
}

impl Assignee {
    /// The text for messages, for example "Jane Doe (jdoe)".
    pub fn label(&self) -> String {
        match self {
            Assignee::Me => "you".into(),
            Assignee::Nobody => "nobody".into(),
            Assignee::User(u) => u.label(),
        }
    }
}

/// The result of a match of a text against the search results.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    One(User),
    None,
    Many(Vec<User>),
}

/// Find the user for a text in the search results.
///
/// The server already filters the users (it finds the text at the start of the username,
/// the display name, or the email). The rules here:
/// 1. A username that is the same as the text (ignores case) wins.
/// 2. Else, one display name that is the same as the text wins.
/// 3. Else, one result is the match, and more results ask.
pub fn pick(users: &[User], text: &str) -> Pick {
    let t = text.trim().to_lowercase();
    if let Some(u) = users.iter().find(|u| u.name.to_lowercase() == t) {
        return Pick::One(u.clone());
    }
    let same_name: Vec<&User> = users.iter().filter(|u| u.display_name.to_lowercase() == t).collect();
    if let [one] = same_name.as_slice() {
        return Pick::One((*one).clone());
    }
    match users {
        [] => Pick::None,
        [one] => Pick::One(one.clone()),
        many => Pick::Many(many.to_vec()),
    }
}

/// The maximum number of users in a "Use one of" hint.
const HINT_USERS: usize = 8;

/// The error for "no user matches". `what` is, for example, "assignable user for BCE-1".
pub fn no_match(text: &str, what: &str) -> Error {
    Error::input_hint(
        format!("No {what} matches `{text}`."),
        "Use the username, a part of the name, `me`, or `none`.",
    )
}

/// The error for "more than one user matches" in a script.
pub fn many_match(text: &str, users: &[User]) -> Error {
    let mut names: Vec<String> = users.iter().take(HINT_USERS).map(|u| u.label()).collect();
    if users.len() > HINT_USERS {
        names.push(format!("and {} more", users.len() - HINT_USERS));
    }
    Error::input_hint(
        format!("`{text}` matches {} users.", users.len()),
        format!("Use the username. Matches: {}.", names.join(", ")),
    )
}

/// Asks the user to select one of the users on a terminal. Returns the index.
pub type Chooser<'a> = &'a mut dyn FnMut(&[User]) -> Result<usize>;

/// Make a `Pick` into one user, or an error. `choose` asks the user on a terminal; it is
/// `None` in a script.
pub fn resolve(
    pick: Pick,
    text: &str,
    what: &str,
    choose: Option<Chooser<'_>>,
) -> Result<User> {
    match pick {
        Pick::One(u) => Ok(u),
        Pick::None => Err(no_match(text, what)),
        Pick::Many(users) => match choose {
            Some(f) => {
                let i = f(&users)?;
                users.into_iter().nth(i).ok_or(Error::Cancelled)
            }
            None => Err(many_match(text, &users)),
        },
    }
}

/// Keep the most recent users first, with no duplicates, and at most `max`.
pub fn remember(recent: &mut Vec<User>, u: User, max: usize) {
    recent.retain(|r| r.name != u.name);
    recent.insert(0, u);
    recent.truncate(max);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(name: &str, display: &str) -> User {
        User { name: name.into(), display_name: display.into() }
    }

    #[test]
    fn parse_me_none_and_text() {
        assert_eq!(UserArg::parse("me"), UserArg::Me);
        assert_eq!(UserArg::parse(" ME "), UserArg::Me);
        assert_eq!(UserArg::parse("none"), UserArg::Nobody);
        assert_eq!(UserArg::parse("-"), UserArg::Nobody);
        assert_eq!(UserArg::parse("@jdoe"), UserArg::Text("jdoe".into()));
        assert_eq!(UserArg::parse("Jane Doe"), UserArg::Text("Jane Doe".into()));
    }

    #[test]
    fn exact_username_wins_over_more_matches() {
        let users = vec![u("jdoe2", "Jane Doe"), u("jdoe", "John Doe")];
        assert_eq!(pick(&users, "JDOE"), Pick::One(u("jdoe", "John Doe")));
    }

    #[test]
    fn one_exact_display_name_wins() {
        let users = vec![u("jdoe", "Jane Doe"), u("jdoe2", "Jane Doering")];
        assert_eq!(pick(&users, "jane doe"), Pick::One(u("jdoe", "Jane Doe")));
    }

    #[test]
    fn one_result_matches_and_more_results_ask() {
        assert_eq!(pick(&[u("jdoe", "Jane Doe")], "jan"), Pick::One(u("jdoe", "Jane Doe")));
        assert_eq!(pick(&[], "x"), Pick::None);
        let two = vec![u("jdoe", "Jane Doe"), u("jroe", "Jane Roe")];
        assert_eq!(pick(&two, "jane"), Pick::Many(two.clone()));
    }

    #[test]
    fn resolve_asks_on_a_terminal_and_stops_in_a_script() {
        let two = vec![u("jdoe", "Jane Doe"), u("jroe", "Jane Roe")];
        let mut choose = |_: &[User]| Ok(1);
        let got = resolve(Pick::Many(two.clone()), "jane", "user", Some(&mut choose)).unwrap();
        assert_eq!(got.name, "jroe");
        let e = resolve(Pick::Many(two), "jane", "user", None).unwrap_err();
        assert_eq!(e.to_string(), "`jane` matches 2 users.");
        assert!(e.hint().unwrap().contains("Jane Doe (jdoe), Jane Roe (jroe)"));
        let e = resolve(Pick::None, "zz", "assignable user for BCE-1", None).unwrap_err();
        assert_eq!(e.to_string(), "No assignable user for BCE-1 matches `zz`.");
    }

    #[test]
    fn remember_keeps_recent_first_with_no_duplicates() {
        let mut r = vec![u("a", "A"), u("b", "B")];
        remember(&mut r, u("b", "B"), 2);
        assert_eq!(r.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        remember(&mut r, u("c", "C"), 2);
        assert_eq!(r.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["c", "b"]);
    }
}
