//! `jui filter list|view|run`. See ticket 21 "Build: search and saved filters".

use std::io::{BufRead, Write};

use super::prompt;
use crate::core::filter::{self as fl, FilterArg};
use crate::core::model::Filter;
use crate::core::{Jira, Result};

/// Where the questions come from. `input` is `None` in a script.
pub struct Ask<'a, 'b> {
    pub input: Option<&'a mut (dyn BufRead + 'b)>,
    pub out: &'a mut (dyn Write + 'b),
}

/// Find a filter from an argument: an ID gets that filter; a name searches the favourite filters.
pub async fn find(j: &Jira, arg: &str, ask: &mut Ask<'_, '_>) -> Result<Filter> {
    match FilterArg::parse(arg) {
        FilterArg::Id(id) => j.filter(&id).await,
        FilterArg::Name(text) => {
            let favs = j.favourite_filters().await?;
            let pick = fl::pick(&favs, &text);
            let title = format!("More than one favourite filter matches `{text}`:");
            let out = &mut *ask.out;
            let mut choose = ask.input.as_deref_mut().map(|input| {
                move |fs: &[Filter]| {
                    let items: Vec<String> = fs.iter().map(fl::label).collect();
                    prompt::select(input, out, &title, &items)
                }
            });
            fl::resolve(pick, &text, favs.len(), choose.as_mut().map(|f| f as fl::Chooser<'_>))
        }
    }
}

/// The text of `jui filter view`.
pub fn render(f: &Filter) -> String {
    let mut s = format!("{}  {}\n", f.id, f.name);
    if let Some(o) = &f.owner {
        s.push_str(&format!("Owner      {o}\n"));
    }
    s.push_str(&format!("Favourite  {}\n", if f.favourite { "yes" } else { "no" }));
    if let Some(d) = &f.description {
        s.push_str(&format!("About      {d}\n"));
    }
    s.push_str(&format!("URL        {}\n\n{}", f.url, f.jql));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_shows_the_fields_and_the_jql() {
        let f = Filter {
            id: "7".into(),
            name: "Open bugs".into(),
            owner: Some("jdoe".into()),
            jql: "type = Bug".into(),
            description: None,
            favourite: true,
            url: "https://jira.example/issues/?filter=7".into(),
        };
        let text = render(&f);
        assert!(text.starts_with("7  Open bugs\nOwner      jdoe\nFavourite  yes\nURL "), "{text}");
        assert!(text.ends_with("\n\ntype = Bug"));
    }
}
