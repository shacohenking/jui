//! `jui assign`, `jui ticket watch|unwatch|watchers`, and the user rules for `-a`.
//! See ticket 05 "jui CLI command tree", part 6, and ticket 20 "Build: assign, watch, and copy".

use std::io::{BufRead, Write};

use super::prompt;
use crate::core::jira::AssignScope;
use crate::core::model::User;
use crate::core::user::{self as us, Assignee, UserArg};
use crate::core::{Error, Jira, Result};

/// The number of users that a search gets. More matches than this ask for a longer text.
pub const SEARCH_MAX: u32 = 20;

/// Where the questions come from. `input` is `None` in a script.
pub struct Ask<'a, 'b> {
    pub input: Option<&'a mut (dyn BufRead + 'b)>,
    pub out: &'a mut (dyn Write + 'b),
}

impl Ask<'_, '_> {
    fn chooser(&mut self, text: &str) -> Option<impl FnMut(&[User]) -> Result<usize> + '_> {
        let title = format!("More than one user matches `{text}`:");
        let out = &mut *self.out;
        self.input.as_deref_mut().map(move |input| {
            move |users: &[User]| {
                let items: Vec<String> = users.iter().map(User::label).collect();
                prompt::select(input, out, &title, &items)
            }
        })
    }
}

/// Find the assignee for an argument: `me`, `none`, or a search in the assignable users.
pub async fn find_assignee(j: &Jira, scope: &AssignScope, arg: &str, ask: &mut Ask<'_, '_>) -> Result<Assignee> {
    match UserArg::parse(arg) {
        UserArg::Me => Ok(Assignee::Me),
        UserArg::Nobody => Ok(Assignee::Nobody),
        UserArg::Text(t) if t.is_empty() => Err(us::no_match(arg, &scope.what())),
        UserArg::Text(t) => {
            let users = j.assignable_users(scope, &t, SEARCH_MAX).await?;
            let pick = us::pick(&users, &t);
            let mut choose = ask.chooser(&t);
            let u = us::resolve(pick, &t, &scope.what(), choose.as_mut().map(|f| f as us::Chooser<'_>))?;
            Ok(Assignee::User(u))
        }
    }
}

/// Find a watcher: `me` (the value `None`) or a search in all users. `none` is not valid.
pub async fn find_watcher(j: &Jira, arg: Option<&str>, ask: &mut Ask<'_, '_>) -> Result<Option<User>> {
    let Some(arg) = arg else { return Ok(None) };
    match UserArg::parse(arg) {
        UserArg::Me => Ok(None),
        UserArg::Nobody => Err(Error::input_hint("`none` is not a watcher.", "Give a username, or no user for yourself.")),
        UserArg::Text(t) => {
            let users = j.search_users(&t, SEARCH_MAX).await?;
            let pick = us::pick(&users, &t);
            let mut choose = ask.chooser(&t);
            us::resolve(pick, &t, "user", choose.as_mut().map(|f| f as us::Chooser<'_>)).map(Some)
        }
    }
}

/// `jui assign <key> <user>`. Returns the assignee that jui set.
pub async fn assign(j: &Jira, key: &str, arg: &str, ask: &mut Ask<'_, '_>) -> Result<Assignee> {
    let who = find_assignee(j, &AssignScope::Ticket(key.to_string()), arg, ask).await?;
    j.assign(key, &who).await?;
    Ok(who)
}

/// The `-a` value of `new` and `edit` as a username for the Template: `me` and `none` stay,
/// and other text becomes the username of the one matching user.
pub async fn assignee_flag(j: &Jira, scope: &AssignScope, arg: &str, ask: &mut Ask<'_, '_>) -> Result<String> {
    Ok(match find_assignee(j, scope, arg, ask).await? {
        Assignee::Me => "me".into(),
        Assignee::Nobody => "none".into(),
        Assignee::User(u) => u.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooser_is_none_in_a_script() {
        let mut out = Vec::new();
        let mut ask = Ask { input: None, out: &mut out };
        assert!(ask.chooser("x").is_none());
    }

    #[test]
    fn chooser_asks_with_labels() {
        let mut out = Vec::new();
        let mut input = "2\n".as_bytes();
        let mut ask = Ask { input: Some(&mut input), out: &mut out };
        let mut f = ask.chooser("jane").unwrap();
        let users = vec![
            User { name: "jdoe".into(), display_name: "Jane Doe".into() },
            User { name: "jroe".into(), display_name: "Jane Roe".into() },
        ];
        assert_eq!(f(&users).unwrap(), 1);
        drop(f);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("More than one user matches `jane`:"));
        assert!(text.contains("2) Jane Roe (jroe)"));
    }
}
