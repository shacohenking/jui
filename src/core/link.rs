//! Links: the relation that the user types, the direction, and the request body.
//!
//! A Link type has two relation texts, for example "blocks" (outward) and "is blocked by"
//! (inward). The user writes `A <relation> B`, for example `BCE-1 blocks BCE-2`.
//!
//! **The direction rule.** A GET check on the real server (Link 3179355) found:
//! BCE-60589 shows "blocks BCE-57795", and `GET /rest/api/2/issueLink/3179355` gives
//! `inwardIssue` = BCE-60589 and `outwardIssue` = BCE-57795. Thus the Ticket that has the
//! outward text is the `inwardIssue`. jui uses the same rule for the POST body. The user
//! must check it with one manual test (ticket 24).
//!
//! All functions are pure.

use serde_json::{Value, json};

use super::error::Error;
use super::model::{Link, LinkType};

/// One relation that the user can select: a Link type and a side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub link_type: LinkType,
    /// True: the outward text ("blocks"). False: the inward text ("is blocked by").
    pub outward: bool,
}

impl Relation {
    /// The relation text, for example "blocks".
    pub fn text(&self) -> &str {
        if self.outward { &self.link_type.outward } else { &self.link_type.inward }
    }

    /// The text for a list: "blocks (Blocks)".
    pub fn label(&self) -> String {
        format!("{} ({})", self.text(), self.link_type.name)
    }
}

/// All relations of the Link types. A type with the same text on both sides gives one relation.
/// The order: the types in the server order, outward first.
pub fn relations(types: &[LinkType]) -> Vec<Relation> {
    let mut out = Vec::new();
    for t in types {
        out.push(Relation { link_type: t.clone(), outward: true });
        if !t.inward.eq_ignore_ascii_case(&t.outward) {
            out.push(Relation { link_type: t.clone(), outward: false });
        }
    }
    out
}

/// True when the relation matches a filter text (part of the text or of the type name).
pub fn relation_matches(r: &Relation, filter: &str) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty() || r.text().to_lowercase().contains(&f) || r.link_type.name.to_lowercase().contains(&f)
}

/// The result of a relation search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    One(Relation),
    /// More than one relation matches. The caller asks on a terminal, or stops in a script.
    Many(Vec<Relation>),
    None,
}

/// Find a relation by the text of the user. The order of the rules:
/// 1. The exact relation text ("blocks", "is blocked by"). Case is ignored.
/// 2. The exact type name ("Blocks"): the outward side.
/// 3. A prefix of the relation texts ("is bl"), or of the type names ("dup": the outward side).
pub fn find(types: &[LinkType], text: &str) -> Found {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return Found::None;
    }
    let all = relations(types);
    let exact: Vec<&Relation> = all.iter().filter(|r| r.text().to_lowercase() == t).collect();
    if let [one] = exact.as_slice() {
        return Found::One((*one).clone());
    }
    if exact.len() > 1 {
        return Found::Many(exact.into_iter().cloned().collect());
    }
    if let Some(r) = all.iter().find(|r| r.outward && r.link_type.name.to_lowercase() == t) {
        return Found::One(r.clone());
    }
    // A prefix of a relation text, or of a type name (the outward side of that type).
    let mut pre: Vec<Relation> = Vec::new();
    for r in &all {
        let hit = r.text().to_lowercase().starts_with(&t) || (r.outward && r.link_type.name.to_lowercase().starts_with(&t));
        if hit && !pre.contains(r) {
            pre.push(r.clone());
        }
    }
    match pre.len() {
        0 => Found::None,
        1 => Found::One(pre.into_iter().next().expect("one")),
        _ => Found::Many(pre),
    }
}

/// The error for a relation that matches nothing, or more than one relation in a script.
pub fn not_found(text: &str, types: &[LinkType], many: &[Relation]) -> Error {
    let list: Vec<String> = if many.is_empty() { relations(types).iter().map(|r| r.text().to_string()).collect() } else { many.iter().map(|r| r.text().to_string()).collect() };
    let what = if many.is_empty() { format!("No Link relation matches `{text}`.") } else { format!("`{text}` matches more than one Link relation.") };
    Error::input_hint(what, format!("Use one of: {}. `jui link types` shows all.", dedup(list).join(", ")))
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// A Link to add: `from <relation> to`, for example `BCE-1 blocks BCE-2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPlan {
    pub from: String,
    pub relation: Relation,
    pub to: String,
    /// An optional comment in wiki markup. Jira adds it as a comment of the Link request.
    pub comment: Option<String>,
}

impl LinkPlan {
    pub fn new(from: &str, relation: Relation, to: &str, comment: Option<String>) -> Result<Self, Error> {
        if from.eq_ignore_ascii_case(to) {
            return Err(Error::input(format!("{from} cannot have a Link to itself.")));
        }
        Ok(LinkPlan { from: from.to_string(), relation, to: to.to_string(), comment: comment.filter(|c| !c.trim().is_empty()) })
    }

    /// The `inwardIssue` and the `outwardIssue` keys (see the direction rule above).
    pub fn sides(&self) -> (&str, &str) {
        if self.relation.outward { (&self.from, &self.to) } else { (&self.to, &self.from) }
    }

    /// The body of `POST /rest/api/2/issueLink`.
    pub fn body(&self) -> Value {
        let (inward, outward) = self.sides();
        let mut b = json!({
            "type": { "name": self.relation.link_type.name },
            "inwardIssue": { "key": inward },
            "outwardIssue": { "key": outward },
        });
        if let Some(c) = &self.comment {
            b["comment"] = json!({ "body": c });
        }
        b
    }

    /// "BCE-1 blocks BCE-2".
    pub fn describe(&self) -> String {
        format!("{} {} {}", self.from, self.relation.text(), self.to)
    }

    /// True when the Ticket `from` already has this Link.
    pub fn exists_in(&self, links: &[Link]) -> bool {
        links.iter().any(|l| l.ticket.key.eq_ignore_ascii_case(&self.to) && l.link_type == self.relation.link_type.name && l.relation == self.relation.text())
    }
}

/// "BCE-1 blocks BCE-2" for an existing Link of `key`.
pub fn describe(key: &str, l: &Link) -> String {
    format!("{key} {} {}", l.relation, l.ticket.key)
}

/// Find the Links of a Ticket by the Link ID, or by the key of the other Ticket.
pub fn select<'a>(links: &'a [Link], arg: &str) -> Vec<&'a Link> {
    let a = arg.trim();
    let by_id: Vec<&Link> = links.iter().filter(|l| l.id == a).collect();
    if !by_id.is_empty() {
        return by_id;
    }
    links.iter().filter(|l| l.ticket.key.eq_ignore_ascii_case(a)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Related, StatusCategory};

    fn t(id: &str, name: &str, inward: &str, outward: &str) -> LinkType {
        LinkType { id: id.into(), name: name.into(), inward: inward.into(), outward: outward.into() }
    }

    fn types() -> Vec<LinkType> {
        vec![
            t("10000", "Blocks", "is blocked by", "blocks"),
            t("10003", "Relates", "relates to", "relates to"),
            t("10002", "Duplicate", "is duplicated by", "duplicates"),
            t("10500", "Parent", "Child", "Parent"),
            t("11100", "Parent-Child", "is child of", "is parent of"),
            t("10800", "is-subtask-of", "is parent of", "is subtask of"),
        ]
    }

    #[test]
    fn relations_skip_the_second_side_of_a_symmetric_type() {
        let r = relations(&types());
        assert_eq!(r.len(), 11);
        assert_eq!(r.iter().filter(|r| r.link_type.name == "Relates").count(), 1);
        assert_eq!(r[1].label(), "is blocked by (Blocks)");
    }

    #[test]
    fn find_exact_text_then_type_name_then_prefix() {
        let ts = types();
        let one = |s: &str| match find(&ts, s) {
            Found::One(r) => (r.link_type.name, r.outward),
            f => panic!("{s}: {f:?}"),
        };
        assert_eq!(one("blocks"), ("Blocks".into(), true));
        assert_eq!(one("IS BLOCKED BY"), ("Blocks".into(), false));
        assert_eq!(one("Duplicate"), ("Duplicate".into(), true), "a type name is the outward side");
        assert_eq!(one("relates"), ("Relates".into(), true));
        assert_eq!(one("dup"), ("Duplicate".into(), true));
        // "is parent of" is a text of two types.
        assert!(matches!(find(&ts, "is parent of"), Found::Many(v) if v.len() == 2));
        assert!(matches!(find(&ts, "is"), Found::Many(_)));
        assert_eq!(find(&ts, "nothing"), Found::None);
        assert_eq!(find(&ts, " "), Found::None);
    }

    #[test]
    fn the_ticket_with_the_outward_text_is_the_inward_issue() {
        let ts = types();
        let Found::One(blocks) = find(&ts, "blocks") else { panic!() };
        let p = LinkPlan::new("BCE-60589", blocks, "BCE-57795", None).unwrap();
        assert_eq!(p.body(), json!({"type": {"name": "Blocks"}, "inwardIssue": {"key": "BCE-60589"}, "outwardIssue": {"key": "BCE-57795"}}));
        assert_eq!(p.describe(), "BCE-60589 blocks BCE-57795");
        let Found::One(by) = find(&ts, "is blocked by") else { panic!() };
        let p = LinkPlan::new("BCE-2", by, "BCE-1", Some("Why".into())).unwrap();
        assert_eq!(p.sides(), ("BCE-1", "BCE-2"));
        assert_eq!(p.body()["comment"], json!({"body": "Why"}));
        assert!(LinkPlan::new("BCE-1", find_one(&ts, "blocks"), "bce-1", None).is_err());
    }

    fn find_one(ts: &[LinkType], s: &str) -> Relation {
        match find(ts, s) {
            Found::One(r) => r,
            f => panic!("{f:?}"),
        }
    }

    fn link(id: &str, rel: &str, key: &str) -> Link {
        Link {
            id: id.into(),
            link_type: "Blocks".into(),
            relation: rel.into(),
            ticket: Related { key: key.into(), summary: String::new(), status: String::new(), status_category: StatusCategory::ToDo, kind: String::new() },
        }
    }

    #[test]
    fn select_by_id_or_by_the_other_key_and_exists() {
        let ls = vec![link("501", "blocks", "BCE-2"), link("502", "is blocked by", "BCE-2"), link("503", "blocks", "BCE-3")];
        assert_eq!(select(&ls, "503").len(), 1);
        assert_eq!(select(&ls, "bce-2").len(), 2);
        assert!(select(&ls, "BCE-9").is_empty());
        let p = LinkPlan::new("BCE-1", find_one(&types(), "blocks"), "BCE-3", None).unwrap();
        assert!(p.exists_in(&ls));
        let p = LinkPlan::new("BCE-1", find_one(&types(), "is blocked by"), "BCE-3", None).unwrap();
        assert!(!p.exists_in(&ls));
        assert_eq!(describe("BCE-1", &ls[1]), "BCE-1 is blocked by BCE-2");
    }

    #[test]
    fn not_found_lists_the_relations() {
        let e = not_found("x", &types(), &[]);
        assert!(e.hint().unwrap().contains("blocks, is blocked by, relates to"), "{:?}", e.hint());
    }
}
