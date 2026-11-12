//! Markup tests with made-up texts. No real Ticket text.

use super::*;

/// wiki -> Markdown -> wiki must keep these texts byte-exact.
const EXACT_ROUND_TRIP: &[&str] = &[
    "[~jdoe] please look.",
    "Ask @ and [~a.b_c-d] now.",
    "The result is {color:#de350b}Failed{color} today.",
    "See [^server.log] for details.",
    "!screen.png|width=600,height=300!",
    "Text with +under+ and ^sup^ and ~sub~ and ??cite??.",
    "{panel:title=Result|bgColor=#DEEBFF}\nText in a panel.\n{panel}",
    "{toc}",
    "Jump {anchor:x} here.",
    "a -- b --- c",
    "{noformat}\nraw  text\n  indented\n{noformat}",
    "{code:title=Main.java|linenumbers=true}\nint x;\n{code}",
    "|no|header|\n|row|two|",
    "{color:red}line one\nline two{color}",
    "{expand:More}\nhidden\n{expand}",
];

/// MVP elements: the converted text must be equal to the input.
const SIMPLE_ROUND_TRIP: &[&str] = &[
    "h3. Steps",
    "*bold* and _italic_ and {{mono}}",
    "* one\n* two\n** nested",
    "# first\n# second",
    "{code:java}\nint x = 1;\n{code}",
    "[the guide|https://example.com/guide]",
    "||A||B||\n|1|2|",
    "Ticket ABC-123 and emoticon (/) stay.",
];

#[test]
fn raw_pieces_round_trip_byte_exact() {
    for s in EXACT_ROUND_TRIP {
        let md = wiki_to_markdown(s);
        let back = markdown_to_wiki(&md.text);
        assert_eq!(back.text, *s, "\nmarkdown was:\n{}", md.text);
    }
}

#[test]
fn mvp_elements_round_trip() {
    for s in SIMPLE_ROUND_TRIP {
        let md = wiki_to_markdown(s);
        let back = markdown_to_wiki(&md.text);
        assert_eq!(back.text, *s, "\nmarkdown was:\n{}", md.text);
    }
}

#[test]
fn edit_mode_forms() {
    let md = |s: &str| wiki_to_markdown(s).text;
    assert_eq!(md("[~jdoe] hi"), "@jdoe hi");
    assert_eq!(md("{color:#de350b}Failed{color}"), "`jira:{color:#de350b}Failed{color}`");
    assert_eq!(md("[^server.log]"), "`jira:[^server.log]`");
    assert_eq!(md("!a.png!"), "![](a.png)");
    assert_eq!(md("h3. Steps\n* *x* and {{y}}"), "### Steps\n\n- **x** and `y`");
    assert_eq!(
        md("{panel:title=R}\nT\n{panel}"),
        "```jira-wiki\n{panel:title=R}\nT\n{panel}\n```"
    );
    assert_eq!(md("{noformat}\nraw\n{noformat}"), "```noformat\nraw\n```");
}

#[test]
fn markdown_to_wiki_forms() {
    let w = |s: &str| markdown_to_wiki(s).text;
    assert_eq!(w("### Steps\n\n- **x** and `y`"), "h3. Steps\n\n* *x* and {{y}}");
    assert_eq!(w("Hi @jdoe, see `jira:{color:red}x{color}`."), "Hi [~jdoe], see {color:red}x{color}.");
    assert_eq!(w("```jira-wiki\n{panel}\nP\n{panel}\n```"), "{panel}\nP\n{panel}");
    assert_eq!(w("```noformat\nraw\n```"), "{noformat}\nraw\n{noformat}");
    assert_eq!(w("a -- b and --verbose"), "a -- b and --verbose");
}

#[test]
fn mention_rules() {
    let w = |s: &str| markdown_to_wiki(s).text;
    assert_eq!(w("@jdoe at start"), "[~jdoe] at start");
    assert_eq!(w("cc (@a.b) please"), "cc ([~a.b]) please");
    assert_eq!(w("ask @jdoe."), "ask [~jdoe].");
    assert_eq!(w("mail a@b.com"), "mail a@b.com");
    assert_eq!(w("code `@jdoe` stays"), "code {{@jdoe}} stays");
    assert_eq!(w("```\n@jdoe\n```"), "{code}\n@jdoe\n{code}");
    assert_eq!(w("typed [~jdoe] too"), "typed [~jdoe] too");
    assert_eq!(w("just @ alone"), "just @ alone");
}

#[test]
fn raw_span_with_backticks() {
    let md = wiki_to_markdown("{color:red}a`b{color}").text;
    assert_eq!(md, "``jira:{color:red}a`b{color}``");
    assert_eq!(markdown_to_wiki(&md).text, "{color:red}a`b{color}");
}

#[test]
fn crlf_is_normalized() {
    assert_eq!(wiki_to_markdown("a\r\nb").text, wiki_to_markdown("a\nb").text);
}

#[test]
fn token_text_in_input_is_safe() {
    let md = wiki_to_markdown("contains JUIRAW0Q text");
    assert!(md.text.contains("JUIRAW0Q"));
    assert_eq!(markdown_to_wiki("JUIRAW0Q").text, "JUIRAW0Q");
}

#[test]
fn display_mode() {
    let lines = display("h3. Title\n[~jdoe] see [^log.txt] and !a.png|width=5!\n{panel:title=Note}\nInside\n{panel}");
    let plain: Vec<String> = lines.iter().map(|l| l.plain()).collect();
    assert_eq!(plain[0], "Title");
    assert!(lines[0].spans[0].style.bold);
    assert!(plain.iter().any(|l| l.contains("@jdoe") && l.contains("[attachment: log.txt]") && l.contains("[image: a.png]")), "{plain:?}");
    assert!(plain.iter().any(|l| l.contains("╭─ Note")), "{plain:?}");
    assert!(plain.iter().any(|l| l == "│ Inside"), "{plain:?}");
}

#[test]
fn display_lists_and_code() {
    let plain: Vec<String> = display("* a\n** b\n\n# one\n\n{code:java}\nx\n{code}").iter().map(|l| l.plain()).collect();
    assert!(plain.contains(&"• a".to_string()), "{plain:?}");
    assert!(plain.contains(&"  • b".to_string()), "{plain:?}");
    assert!(plain.contains(&"1. one".to_string()), "{plain:?}");
    assert!(plain.contains(&"┌─ java".to_string()), "{plain:?}");
    assert!(plain.contains(&"│ x".to_string()), "{plain:?}");
}

#[test]
fn display_code_tags_in_line() {
    let plain: Vec<String> = display("Error: {code:java}x = 1{code} after").iter().map(|l| l.plain()).collect();
    assert!(plain.contains(&"┌─ java".to_string()), "{plain:?}");
    assert!(plain.contains(&"│ x = 1".to_string()), "{plain:?}");
    assert!(plain.contains(&"after".to_string()), "{plain:?}");
}

#[test]
fn code_tags_in_line_round_trip() {
    for s in ["Error: {code:java}x = 1{code} after", "see {noformat}\nraw\n{noformat}"] {
        assert_eq!(markdown_to_wiki(&wiki_to_markdown(s).text).text, s);
    }
}

#[test]
fn display_amdc_repairs() {
    let p = |s: &str| display(s).iter().map(|l| l.plain()).collect::<Vec<_>>();
    assert_eq!(p("{panel:title=T|bgColor=#fff}{panel}"), vec!["╭─ T", "╰─"]);
    assert_eq!(p("|A|[x.sh|https://x.example/a.sh] more|"), vec!["A │ x.sh (https://x.example/a.sh) more"]);
}

#[test]
fn display_color() {
    let lines = display("{color:#de350b}Failed{color}");
    assert_eq!(lines[0].spans[0].style.color, Some(Color::Red));
}

#[test]
fn bang_before_a_space_is_not_escaped() {
    let w = |s: &str| markdown_to_wiki(s).text;
    assert_eq!(w("Done!"), "Done!");
    assert_eq!(w("Done! See\n\nnext!"), "Done! See\n\nnext!");
    // A `!` with a non-space character after it can start an image: it keeps the escape.
    assert_eq!(w("!x!"), "\\!x!");
    assert_eq!(w("Wow!!"), "Wow!!");
}

#[test]
fn adjacent_raw_spans_become_one_span() {
    let wiki = "{panel:bgColor=#ffe6cc|title=Note}{panel}\n\nh3. Next";
    let md = wiki_to_markdown(wiki).text;
    assert!(md.starts_with("`jira:{panel:bgColor=#ffe6cc|title=Note}{panel}`"), "{md}");
    assert_eq!(markdown_to_wiki(&md).text, wiki);
    let wiki = "a {color:red}x{color}+u+ b";
    assert_eq!(markdown_to_wiki(&wiki_to_markdown(wiki).text).text, wiki);
}


#[test]
fn noformat_with_text_on_the_tag_line_stays_raw() {
    let wiki = "* Check:\n\n{noformat}bq 'a_b * `c`'\nbq x{noformat}\n\n* Next";
    let md = wiki_to_markdown(wiki).text;
    assert!(md.contains("```jira-wiki\n{noformat}bq 'a_b * `c`'\nbq x{noformat}\n```"), "{md}");
    assert_eq!(markdown_to_wiki(&md).text, wiki);
    // With no blank line before the block, jui adds one. Jira shows the same result.
    let tight = "* Check:\n{noformat}x{noformat}";
    assert_eq!(markdown_to_wiki(&wiki_to_markdown(tight).text).text, "* Check:\n\n{noformat}x{noformat}");
}

#[test]
fn link_with_monospace_label_stays_raw() {
    let wiki = "_x @ [{{721c8a3}}|https://g.example/c/721] · y_";
    let md = wiki_to_markdown(wiki).text;
    assert!(md.contains("`jira:[{{721c8a3}}|https://g.example/c/721]`"), "{md}");
    assert_eq!(markdown_to_wiki(&md).text, wiki);
    // A plain link is still a Markdown link.
    assert_eq!(wiki_to_markdown("[text|https://a.example]").text, "[text](https://a.example)");
}
