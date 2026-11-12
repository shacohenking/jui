//! Find the pieces that jui protects from `amdc`, and replace them with tokens.

use super::{Piece, token};

/// Block macros that `amdc` converts well, or that jui handles in another way.
const KNOWN_MACROS: &[&str] = &["code", "noformat", "panel", "quote", "color"];

struct Out {
    text: String,
    pieces: Vec<Piece>,
}

impl Out {
    fn push(&mut self, p: Piece) -> String {
        self.pieces.push(p);
        token(self.pieces.len() - 1)
    }
    /// A block token on its own line, with blank lines around it.
    fn block(&mut self, p: Piece) {
        let t = self.push(p);
        if !self.text.is_empty() && !self.text.ends_with("\n\n") {
            self.text.push_str(if self.text.ends_with('\n') { "\n" } else { "\n\n" });
        }
        self.text.push_str(&t);
        self.text.push_str("\n\n");
    }
    fn line(&mut self, l: &str) {
        self.text.push_str(l);
        self.text.push('\n');
    }
}

// ---------------------------------------------------------------- wiki

fn is_macro_line(trimmed: &str) -> Option<(&str, &str)> {
    // `{name}` or `{name:params}` alone on the line.
    let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?;
    if inner.contains('{') || inner.contains('}') {
        return None;
    }
    let (name, params) = inner.split_once(':').unwrap_or((inner, ""));
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    Some((name, params))
}

fn find_close(lines: &[&str], from: usize, close: &str) -> Option<usize> {
    (from..lines.len()).find(|&j| lines[j].contains(close))
}

fn simple_code_open(trimmed: &str) -> bool {
    match is_macro_line(trimmed) {
        Some(("code", "")) => true,
        Some(("code", lang)) => lang.chars().all(|c| c.is_ascii_alphanumeric() || "+#-_".contains(c)),
        _ => false,
    }
}

/// Protect the wiki elements that have no clean Markdown form.
pub(crate) fn protect_wiki(text: &str) -> (String, Vec<Piece>) {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Out { text: String::new(), pieces: Vec::new() };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();

        // {code} and {code:lang}: amdc converts them. Copy unchanged, no inline scan.
        if simple_code_open(t) {
            let end = find_close(&lines, i + 1, "{code}").unwrap_or(lines.len() - 1);
            for l in &lines[i..=end] {
                out.line(l);
            }
            i = end + 1;
            continue;
        }
        // {code} or {noformat} tags in the middle of a line: keep all lines up to the
        // closing tag raw, because amdc parses them in a different way than Jira.
        if !t.starts_with("{code") && !t.starts_with("{noformat") && (line.contains("{code") || line.contains("{noformat}")) {
            let tag = if line.contains("{noformat}") { "{noformat}" } else { "{code" };
            let mut count = line.matches(tag).count();
            let mut end = i;
            while count % 2 == 1 && end + 1 < lines.len() {
                end += 1;
                count += lines[end].matches(tag).count();
            }
            out.block(Piece::Raw { raw: lines[i..=end].join("\n"), block: true });
            i = end + 1;
            continue;
        }
        // Any other {code...} at the start of a line (parameters such as
        // `title=`, or code on the same line as the tag): keep it raw.
        if t.starts_with("{code") {
            let after_open = t.find('}').map(|p| &t[p + 1..]).unwrap_or("");
            let end = if after_open.contains("{code}") {
                i
            } else {
                find_close(&lines, i + 1, "{code}").unwrap_or(lines.len() - 1)
            };
            out.block(Piece::Raw { raw: lines[i..=end].join("\n"), block: true });
            i = end + 1;
            continue;
        }
        // {quote} blocks: amdc writes them back as `bq.`. Keep them raw.
        if is_macro_line(t) == Some(("quote", ""))
            && let Some(end) = find_close(&lines, i + 1, "{quote}")
        {
            out.block(Piece::Raw { raw: lines[i..=end].join("\n"), block: true });
            i = end + 1;
            continue;
        }
        // {noformat} with text on the same line (`{noformat}text` or `{noformat}a{noformat}`):
        // keep all lines up to the closing tag raw.
        if t.starts_with("{noformat}") && t != "{noformat}" {
            let mut count = line.matches("{noformat}").count();
            let mut end = i;
            while count % 2 == 1 && end + 1 < lines.len() {
                end += 1;
                count += lines[end].matches("{noformat}").count();
            }
            out.block(Piece::Raw { raw: lines[i..=end].join("\n"), block: true });
            i = end + 1;
            continue;
        }
        // {noformat}: keep the macro. Show it as a ```noformat block.
        if is_macro_line(t) == Some(("noformat", ""))
            && let Some(end) = find_close(&lines, i + 1, "{noformat}").filter(|&e| lines[e].trim() == "{noformat}")
        {
            let body = lines[i + 1..end].join("\n");
            out.block(Piece::Literal {
                wiki: lines[i..=end].join("\n"),
                markdown: format!("```noformat\n{body}\n```"),
            });
            i = end + 1;
            continue;
        }
        // {panel}, and block macros that jui does not know, with a closing tag.
        if let Some((name, _)) = is_macro_line(t).or_else(|| panel_open(t)) {
            let known = KNOWN_MACROS.contains(&name) && name != "panel";
            let close = format!("{{{name}}}");
            if !known && let Some(end) = find_close(&lines, i + 1, &close) {
                out.block(Piece::Raw { raw: lines[i..=end].join("\n"), block: true });
                i = end + 1;
                continue;
            }
        }
        // Tables with no header row: GFM cannot show them. A table with a header
        // row is copied unchanged.
        if t.starts_with("||") {
            let mut end = i;
            while end + 1 < lines.len() && lines[end + 1].trim_start().starts_with('|') {
                end += 1;
            }
            let rows = &lines[i..=end];
            // amdc writes a table back as `||A||` and `|x|`. Convert only a table that
            // is already in that form and has simple cells; keep others raw.
            if rows.iter().all(|r| simple_table_row(r)) {
                for l in rows {
                    let protected = protect_wiki_inline(l, &mut out);
                    out.line(&protected);
                }
            } else {
                out.block(Piece::Raw { raw: rows.join("\n"), block: true });
            }
            i = end + 1;
            continue;
        }
        if t.starts_with('|') {
            let mut end = i;
            while end + 1 < lines.len() && lines[end + 1].trim_start().starts_with('|') {
                end += 1;
            }
            out.block(Piece::Raw { raw: lines[i..=end].join("\n"), block: true });
            i = end + 1;
            continue;
        }
        // {color} that continues on later lines.
        if let Some(open) = line.rfind("{color:")
            && !line[open..].contains("{color}")
            && let Some(end) = find_close(&lines, i + 1, "{color}")
        {
            out.block(Piece::Raw { raw: lines[i..=end].join("\n"), block: true });
            i = end + 1;
            continue;
        }

        // `- item` lists (Jira shows them as lists). amdc writes them back as
        // `* item`, so keep the whole run of lines raw.
        if line.starts_with("- ") {
            let mut end = i;
            while end + 1 < lines.len() && (lines[end + 1].starts_with("- ") || lines[end + 1].starts_with("-- ")) {
                end += 1;
            }
            out.block(Piece::Raw { raw: lines[i..=end].join("\n"), block: true });
            i = end + 1;
            continue;
        }
        let protected = protect_wiki_inline(line, &mut out);
        out.line(&protected);
        i += 1;
    }
    if out.text.ends_with('\n') && !text.ends_with('\n') {
        out.text.pop();
    }
    (out.text, out.pieces)
}

/// A table row with no spaces next to the `|` separators, no links (their `|`
/// breaks GFM cells), and no macros.
fn simple_table_row(row: &str) -> bool {
    let r = row.trim_end();
    r.starts_with('|')
        && r.ends_with('|')
        && !r.contains("| ")
        && !r.contains(" |")
        && !r.contains('[')
        && !r.contains('{')
}

fn panel_open(t: &str) -> Option<(&'static str, &str)> {
    // `{panel:title=A|bgColor=#fff}` can contain characters that `is_macro_line` accepts.
    if t.starts_with("{panel") && t.ends_with('}') && !t[1..].contains('{') {
        Some(("panel", ""))
    } else {
        None
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// An opening delimiter at `i` needs a non-word character (or the start) before it,
/// and a non-space after it.
fn can_open(chars: &[char], i: usize, len: usize) -> bool {
    (i == 0 || !is_word(chars[i - 1])) && chars.get(i + len).is_some_and(|c| !c.is_whitespace())
}

fn can_close(chars: &[char], j: usize, len: usize) -> bool {
    j > 0 && !chars[j - 1].is_whitespace() && chars.get(j + len).is_none_or(|c| !is_word(*c))
}

fn starts_with_at(chars: &[char], i: usize, s: &str) -> bool {
    (i..).zip(s.chars()).all(|(k, c)| chars.get(k) == Some(&c))
}

fn find_from(chars: &[char], from: usize, s: &str) -> Option<usize> {
    (from..chars.len()).find(|&k| starts_with_at(chars, k, s))
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

fn protect_wiki_inline(line: &str, out: &mut Out) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut s = String::new();
    let mut i = 0;
    let slice = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
    while i < chars.len() {
        let c = chars[i];
        // {{monospace}}: copy unchanged.
        if starts_with_at(&chars, i, "{{")
            && let Some(end) = find_from(&chars, i + 2, "}}")
        {
            s.push_str(&slice(i, end + 2));
            i = end + 2;
            continue;
        }
        // [~mention] and [^attachment]
        if c == '['
            && let Some(&next) = chars.get(i + 1)
            && (next == '~' || next == '^')
            && let Some(end) = (i + 2..chars.len()).find(|&k| chars[k] == ']')
        {
            let inner = slice(i + 2, end);
            if next == '~' && !inner.is_empty() && inner.chars().all(is_name_char) {
                s.push_str(&out.push(Piece::Mention(inner)));
                i = end + 1;
                continue;
            }
            if next == '^' && !inner.is_empty() {
                s.push_str(&out.push(Piece::Raw { raw: slice(i, end + 1), block: false }));
                i = end + 1;
                continue;
            }
        }
        // [label|url] where the label has {{monospace}}: amdc escapes the braces on the way
        // back. Keep the link raw.
        if c == '['
            && let Some(bar) = (i + 1..chars.len()).find(|&k| chars[k] == '|' || chars[k] == ']')
            && chars[bar] == '|'
            && slice(i + 1, bar).contains("{{")
            && let Some(end) = (bar + 1..chars.len()).find(|&k| chars[k] == ']' || chars[k].is_whitespace())
            && chars[end] == ']'
        {
            s.push_str(&out.push(Piece::Raw { raw: slice(i, end + 1), block: false }));
            i = end + 1;
            continue;
        }
        // [https://url] (a link with no text): keep it raw so it comes back unchanged.
        if c == '['
            && (starts_with_at(&chars, i + 1, "http://") || starts_with_at(&chars, i + 1, "https://"))
            && let Some(end) = (i + 1..chars.len()).find(|&k| chars[k] == ']' || chars[k] == '|' || chars[k].is_whitespace())
            && chars[end] == ']'
        {
            s.push_str(&out.push(Piece::Raw { raw: slice(i, end + 1), block: false }));
            i = end + 1;
            continue;
        }
        // {color:...}...{color}
        if starts_with_at(&chars, i, "{color:")
            && let Some(end) = find_from(&chars, i + 7, "{color}")
        {
            s.push_str(&out.push(Piece::Raw { raw: slice(i, end + 7), block: false }));
            i = end + 7;
            continue;
        }
        // Other {macro} or {macro:params}
        if c == '{'
            && let Some(end) = (i + 1..chars.len()).find(|&k| chars[k] == '}' || chars[k] == '{')
            && chars[end] == '}'
        {
            let inner = slice(i + 1, end);
            let name = inner.split(':').next().unwrap_or("");
            let valid = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if valid && !matches!(name, "code" | "noformat" | "quote") {
                s.push_str(&out.push(Piece::Raw { raw: slice(i, end + 1), block: false }));
                i = end + 1;
                continue;
            }
        }
        // !image|attributes!
        if c == '!'
            && can_open(&chars, i, 1)
            && let Some(end) = (i + 1..chars.len()).find(|&k| chars[k] == '!' || chars[k].is_whitespace())
            && chars[end] == '!'
            && chars[i + 1..end].contains(&'|')
        {
            s.push_str(&out.push(Piece::Raw { raw: slice(i, end + 1), block: false }));
            i = end + 1;
            continue;
        }
        // +underline+, ^sup^, ~sub~, ??cite??
        let delim = match c {
            '+' | '^' | '~' => Some(c.to_string()),
            '?' if chars.get(i + 1) == Some(&'?') => Some("??".to_string()),
            _ => None,
        };
        if let Some(d) = delim {
            let n = d.chars().count();
            if can_open(&chars, i, n) {
                let close = (i + n + 1..chars.len()).find(|&k| starts_with_at(&chars, k, &d) && can_close(&chars, k, n));
                if let Some(end) = close {
                    s.push_str(&out.push(Piece::Raw { raw: slice(i, end + n), block: false }));
                    i = end + n;
                    continue;
                }
            }
        }
        // -- and --- (Jira shows them as dashes; amdc drops them). Not a ---- rule.
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            let mut end = i;
            while end < chars.len() && chars[end] == '-' {
                end += 1;
            }
            let run = end - i;
            if run <= 3 {
                let lit = "-".repeat(run);
                s.push_str(&out.push(Piece::Literal { wiki: lit.clone(), markdown: lit }));
            } else {
                s.push_str(&slice(i, end));
            }
            i = end;
            continue;
        }
        s.push(c);
        i += 1;
    }
    s
}

// ---------------------------------------------------------------- markdown

fn fence_open(line: &str) -> Option<(char, usize, String)> {
    let t = line.trim_start();
    let c = t.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = t.chars().take_while(|&x| x == c).count();
    if n < 3 {
        return None;
    }
    let info = t[n..].trim().to_string();
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info))
}

fn fence_close(line: &str, c: char, n: usize) -> bool {
    let t = line.trim();
    t.chars().count() >= n && t.chars().all(|x| x == c)
}

/// Protect Raw wiki pieces, `{noformat}` blocks, mentions, and dashes in Markdown.
pub(crate) fn protect_markdown(text: &str) -> (String, Vec<Piece>) {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Out { text: String::new(), pieces: Vec::new() };
    let mut i = 0;
    while i < lines.len() {
        if let Some((c, n, info)) = fence_open(lines[i]) {
            let end = (i + 1..lines.len()).find(|&j| fence_close(lines[j], c, n));
            let body_end = end.unwrap_or(lines.len());
            let body = lines[i + 1..body_end].join("\n");
            let last = end.unwrap_or(lines.len() - 1);
            match info.as_str() {
                "jira-wiki" => out.block(Piece::Raw { raw: body, block: true }),
                "noformat" => out.block(Piece::Literal {
                    wiki: format!("{{noformat}}\n{body}\n{{noformat}}"),
                    markdown: lines[i..=last].join("\n"),
                }),
                _ => {
                    for l in &lines[i..=last] {
                        out.line(l);
                    }
                }
            }
            i = last + 1;
            continue;
        }
        // A table delimiter row (`| --- | :-: |`) must stay unchanged.
        let t = lines[i].trim();
        if t.contains('-') && t.contains('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
            out.line(lines[i]);
            i += 1;
            continue;
        }
        let protected = protect_markdown_inline(lines[i], &mut out);
        out.line(&protected);
        i += 1;
    }
    if out.text.ends_with('\n') && !text.ends_with('\n') {
        out.text.pop();
    }
    (out.text, out.pieces)
}

fn protect_markdown_inline(line: &str, out: &mut Out) -> String {
    let chars: Vec<char> = line.chars().collect();
    let slice = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
    let mut s = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Code span: a run of n backticks, closed by a run of exactly n.
        if c == '`' {
            let n = chars[i..].iter().take_while(|&&x| x == '`').count();
            let close = (i + n..chars.len()).find(|&k| {
                chars[k] == '`'
                    && (k == 0 || chars[k - 1] != '`')
                    && chars[k..].iter().take_while(|&&x| x == '`').count() == n
            });
            if let Some(end) = close {
                let content = slice(i + n, end);
                if let Some(raw) = content.strip_prefix("jira:") {
                    let raw = match raw.strip_suffix(' ') {
                        Some(r) if r.ends_with('`') => r,
                        _ => raw,
                    };
                    s.push_str(&out.push(Piece::Raw { raw: raw.to_string(), block: false }));
                } else {
                    s.push_str(&slice(i, end + n));
                }
                i = end + n;
                continue;
            }
            s.push_str(&slice(i, i + n));
            i += n;
            continue;
        }
        // [~name] typed by the user.
        if c == '['
            && chars.get(i + 1) == Some(&'~')
            && let Some(end) = (i + 2..chars.len()).find(|&k| chars[k] == ']')
        {
            let inner = slice(i + 2, end);
            if !inner.is_empty() && inner.chars().all(is_name_char) {
                s.push_str(&out.push(Piece::Mention(inner)));
                i = end + 1;
                continue;
            }
        }
        // @name at the start, or after a space or `(`.
        if c == '@' && (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(') {
            let mut end = i + 1;
            while end < chars.len() && is_name_char(chars[end]) {
                end += 1;
            }
            // A name does not end with `.` or `-` (for example "ask @jdoe.").
            while end > i + 1 && matches!(chars[end - 1], '.' | '-') {
                end -= 1;
            }
            if end > i + 1 {
                s.push_str(&out.push(Piece::Mention(slice(i + 1, end))));
                i = end;
                continue;
            }
        }
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            let mut end = i;
            while end < chars.len() && chars[end] == '-' {
                end += 1;
            }
            let run = end - i;
            // A line of only dashes is a rule or a setext heading: leave it.
            if run <= 3 && line.trim() != slice(i, end) {
                let lit = "-".repeat(run);
                s.push_str(&out.push(Piece::Literal { wiki: lit.clone(), markdown: lit }));
            } else {
                s.push_str(&slice(i, end));
            }
            i = end;
            continue;
        }
        s.push(c);
        i += 1;
    }
    s
}
