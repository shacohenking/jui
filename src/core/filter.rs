//! Saved filters: the rules to find a filter from a command-line argument.
//! See ticket 21 "Build: search and saved filters".
//!
//! - A number is a filter ID. jui gets that filter. It need not be a favourite.
//! - Other text is a name. jui finds it in the favourite filters, because Jira Data
//!   Center cannot search all filters by name.
//!
//! This module has no I/O.

use super::error::{Error, Result};
use super::model::Filter;

/// A filter argument from the command line or the TUI popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterArg {
    Id(String),
    Name(String),
}

impl FilterArg {
    pub fn parse(s: &str) -> Self {
        let t = s.trim();
        if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
            FilterArg::Id(t.to_string())
        } else {
            FilterArg::Name(t.to_string())
        }
    }
}

/// The result of a match of a name against the favourite filters.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    One(Filter),
    None,
    Many(Vec<Filter>),
}

/// True when the filter name contains the text (ignores case). The TUI popup uses it too.
pub fn name_matches(f: &Filter, text: &str) -> bool {
    let t = text.trim().to_lowercase();
    t.is_empty() || f.name.to_lowercase().contains(&t) || f.id == t
}

/// Find a filter by name:
/// 1. A name that is the same as the text (ignores case) wins.
/// 2. Else, the names that start with the text.
/// 3. Else, the names that contain the text.
///
/// One result is the match. More results ask on a terminal, and stop in a script.
pub fn pick(filters: &[Filter], text: &str) -> Pick {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Pick::None;
    }
    if let Some(f) = filters.iter().find(|f| f.name.to_lowercase() == t) {
        return Pick::One(f.clone());
    }
    let starts: Vec<&Filter> = filters.iter().filter(|f| f.name.to_lowercase().starts_with(&t)).collect();
    let found = if starts.is_empty() {
        filters.iter().filter(|f| f.name.to_lowercase().contains(&t)).collect()
    } else {
        starts
    };
    match found.as_slice() {
        [] => Pick::None,
        [one] => Pick::One((*one).clone()),
        many => Pick::Many(many.iter().map(|f| (*f).clone()).collect()),
    }
}

/// The text for one filter in a list, for example `117549  BCE board`.
pub fn label(f: &Filter) -> String {
    format!("{}  {}", f.id, f.name)
}

/// The error for "no favourite filter matches".
pub fn no_match(text: &str, favourites: usize) -> Error {
    if favourites == 0 {
        return no_favourites();
    }
    Error::input_hint(
        format!("No favourite filter matches `{text}`."),
        "Use `jui filter list` to see your favourite filters, or give the filter ID.",
    )
}

/// The error for "you have no favourite filters".
pub fn no_favourites() -> Error {
    Error::input_hint(
        "You have no favourite filters.",
        "Give the filter ID (the number in the filter URL), or mark the filter as a favourite (the star) in Jira.",
    )
}

/// The error for "more than one filter matches" in a script.
pub fn many_match(text: &str, filters: &[Filter]) -> Error {
    let names: Vec<String> = filters.iter().take(8).map(|f| format!("{} ({})", f.name, f.id)).collect();
    Error::input_hint(
        format!("`{text}` matches {} favourite filters.", filters.len()),
        format!("Use the filter ID. Matches: {}.", names.join(", ")),
    )
}

/// Asks the user to select one filter on a terminal. Returns the index.
pub type Chooser<'a> = &'a mut dyn FnMut(&[Filter]) -> Result<usize>;

/// Make a `Pick` into one filter, or an error. `choose` is `None` in a script.
pub fn resolve(pick: Pick, text: &str, favourites: usize, choose: Option<Chooser<'_>>) -> Result<Filter> {
    match pick {
        Pick::One(f) => Ok(f),
        Pick::None => Err(no_match(text, favourites)),
        Pick::Many(fs) => match choose {
            Some(c) => {
                let i = c(&fs)?;
                fs.into_iter().nth(i).ok_or(Error::Cancelled)
            }
            None => Err(many_match(text, &fs)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(id: &str, name: &str) -> Filter {
        Filter {
            id: id.into(),
            name: name.into(),
            owner: None,
            jql: format!("project = {name}"),
            description: None,
            favourite: true,
            url: String::new(),
        }
    }

    #[test]
    fn parse_id_or_name() {
        assert_eq!(FilterArg::parse(" 117549 "), FilterArg::Id("117549".into()));
        assert_eq!(FilterArg::parse("BCE board"), FilterArg::Name("BCE board".into()));
        assert_eq!(FilterArg::parse("12a"), FilterArg::Name("12a".into()));
    }

    #[test]
    fn exact_name_then_prefix_then_part() {
        let fs = vec![f("1", "BCE bugs"), f("2", "BCE bugs open"), f("3", "My open work")];
        assert_eq!(pick(&fs, "bce BUGS"), Pick::One(fs[0].clone()));
        assert_eq!(pick(&fs, "my"), Pick::One(fs[2].clone()));
        assert_eq!(pick(&fs, "work"), Pick::One(fs[2].clone()));
        assert_eq!(pick(&fs, "bce"), Pick::Many(vec![fs[0].clone(), fs[1].clone()]));
        // "open" is in two names, but no name starts with it.
        assert_eq!(pick(&fs, "open"), Pick::Many(vec![fs[1].clone(), fs[2].clone()]));
        assert_eq!(pick(&fs, "zz"), Pick::None);
        assert_eq!(pick(&fs, " "), Pick::None);
    }

    #[test]
    fn resolve_asks_on_a_terminal_and_stops_in_a_script() {
        let two = vec![f("1", "A one"), f("2", "A two")];
        let mut choose = |_: &[Filter]| Ok(1);
        assert_eq!(resolve(Pick::Many(two.clone()), "a", 2, Some(&mut choose)).unwrap().id, "2");
        let e = resolve(Pick::Many(two), "a", 2, None).unwrap_err();
        assert_eq!(e.to_string(), "`a` matches 2 favourite filters.");
        assert!(e.hint().unwrap().contains("A one (1), A two (2)"));
        let e = resolve(Pick::None, "zz", 0, None).unwrap_err();
        assert_eq!(e.to_string(), "You have no favourite filters.");
        let e = resolve(Pick::None, "zz", 3, None).unwrap_err();
        assert_eq!(e.to_string(), "No favourite filter matches `zz`.");
    }

    #[test]
    fn name_match_ignores_case_and_finds_the_id() {
        let x = f("117549", "BCE board");
        assert!(name_matches(&x, ""));
        assert!(name_matches(&x, "board"));
        assert!(name_matches(&x, "117549"));
        assert!(!name_matches(&x, "sprint"));
    }
}
