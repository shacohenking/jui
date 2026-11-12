//! Questions on a terminal. The questions go to stderr, so that stdout stays clean.
//! The functions read from any `BufRead`, so that tests can give the answers.

use std::io::{BufRead, Write};

use crate::core::{Error, Result};

/// Ask the user to select one item from a numbered list. Returns the index.
/// An empty answer or `q` cancels.
pub fn select(input: &mut (impl BufRead + ?Sized), out: &mut (impl Write + ?Sized), title: &str, items: &[String]) -> Result<usize> {
    let _ = writeln!(out, "{title}");
    for (i, it) in items.iter().enumerate() {
        let _ = writeln!(out, "  {:>2}) {it}", i + 1);
    }
    loop {
        let answer = ask(input, out, &format!("Select 1-{} (Enter cancels): ", items.len()))?;
        if answer.is_empty() || answer.eq_ignore_ascii_case("q") {
            return Err(Error::Cancelled);
        }
        if let Ok(n) = answer.parse::<usize>()
            && (1..=items.len()).contains(&n)
        {
            return Ok(n - 1);
        }
        // A unique text match also selects an item.
        let a = answer.to_lowercase();
        let found: Vec<usize> =
            items.iter().enumerate().filter(|(_, it)| it.to_lowercase().starts_with(&a)).map(|(i, _)| i).collect();
        if let [one] = found.as_slice() {
            return Ok(*one);
        }
        let _ = writeln!(out, "Type a number from 1 to {}.", items.len());
    }
}

/// Ask for one line of text. Returns the text with no spaces at the ends.
/// The end of the input is a cancel.
pub fn ask(input: &mut (impl BufRead + ?Sized), out: &mut (impl Write + ?Sized), question: &str) -> Result<String> {
    let _ = write!(out, "{question}");
    let _ = out.flush();
    let mut line = String::new();
    let n = input.read_line(&mut line).map_err(|e| Error::input(format!("Cannot read the answer: {e}")))?;
    if n == 0 {
        let _ = writeln!(out);
        return Err(Error::Cancelled);
    }
    Ok(line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<String> {
        vec!["To Do".into(), "In Progress".into(), "In Pull Request".into()]
    }

    #[test]
    fn select_by_number_or_unique_prefix() {
        let mut out = Vec::new();
        assert_eq!(select(&mut "2\n".as_bytes(), &mut out, "Move to:", &items()).unwrap(), 1);
        assert_eq!(select(&mut "in pull\n".as_bytes(), &mut out, "Move to:", &items()).unwrap(), 2);
        // A bad answer asks again.
        assert_eq!(select(&mut "9\nin\nto\n".as_bytes(), &mut out, "Move to:", &items()).unwrap(), 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("   1) To Do"));
        assert!(text.contains("Type a number from 1 to 3."));
    }

    #[test]
    fn empty_answer_or_end_of_input_cancels() {
        let mut out = Vec::new();
        assert!(matches!(select(&mut "\n".as_bytes(), &mut out, "x", &items()), Err(Error::Cancelled)));
        assert!(matches!(select(&mut "".as_bytes(), &mut out, "x", &items()), Err(Error::Cancelled)));
    }
}
