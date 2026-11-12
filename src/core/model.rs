//! Domain types that the CLI and the TUI use, and the conversion from the API layer.
//! `--json` writes these types, so their shape must stay stable.

use chrono::{DateTime, FixedOffset};
use serde::Serialize;

use super::api;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusCategory {
    ToDo,
    InProgress,
    Done,
    Unknown,
}

impl StatusCategory {
    fn from_key(key: &str) -> Self {
        match key {
            "new" => StatusCategory::ToDo,
            "indeterminate" => StatusCategory::InProgress,
            "done" => StatusCategory::Done,
            _ => StatusCategory::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    pub name: String,
    pub display_name: String,
}

impl From<api::UserEntry> for User {
    fn from(u: api::UserEntry) -> Self {
        User { display_name: u.display_name.filter(|d| !d.is_empty()).unwrap_or_else(|| u.name.clone()), name: u.name }
    }
}

impl User {
    /// `Jane Doe (jdoe)`.
    pub fn label(&self) -> String {
        if self.display_name == self.name { self.name.clone() } else { format!("{} ({})", self.display_name, self.name) }
    }
}

/// The watchers of a Ticket.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Watchers {
    pub key: String,
    /// True when the current user watches the Ticket.
    pub watching: bool,
    pub count: u64,
    pub users: Vec<User>,
}

/// A Ticket with the fields that list views show.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TicketSummary {
    pub key: String,
    pub summary: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub status_category: StatusCategory,
    pub priority: Option<String>,
    pub assignee: Option<User>,
    pub updated: Option<DateTime<FixedOffset>>,
    /// The open Sprint of the Ticket (active first, then future). `None` when the Ticket is
    /// in no Sprint, or only in closed Sprints. My Tickets groups the rows by this value.
    pub sprint: Option<SprintRef>,
}

/// A list row in the TUI: the list fields and the description (wiki markup).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListRow {
    pub ticket: TicketSummary,
    pub description: Option<String>,
}

impl From<api::Issue> for ListRow {
    fn from(mut i: api::Issue) -> Self {
        let description = i.fields.description.take().filter(|d| !d.trim().is_empty());
        ListRow { ticket: i.into(), description }
    }
}

/// One page of search results.
#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub start_at: u64,
    /// The total number of results that the query finds.
    pub total: u64,
}

impl<T> Page<T> {
    pub fn has_more(&self) -> bool {
        self.start_at + (self.items.len() as u64) < self.total
    }
}

/// Jira dates look like `2026-10-19T00:30:46.000-0700`.
pub fn parse_jira_date(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z").ok()
}

impl From<api::Issue> for TicketSummary {
    fn from(i: api::Issue) -> Self {
        let f = i.fields;
        let (status, cat) = match f.status {
            Some(s) => {
                let cat = s.status_category.map(|c| StatusCategory::from_key(&c.key)).unwrap_or(StatusCategory::Unknown);
                (s.name, cat)
            }
            None => (String::new(), StatusCategory::Unknown),
        };
        TicketSummary {
            key: i.key,
            summary: f.summary.unwrap_or_default(),
            kind: f.issuetype.map(|t| t.name).unwrap_or_default(),
            status,
            status_category: cat,
            priority: f.priority.map(|p| p.name),
            assignee: f.assignee.map(|u| User { display_name: u.display_name.unwrap_or_else(|| u.name.clone()), name: u.name }),
            updated: f.updated.as_deref().and_then(parse_jira_date),
            sprint: f.sprint.as_ref().and_then(sprint_from_field),
        }
    }
}

/// The keys of the legacy Sprint string. A value continues until the next known key, so a
/// name or a goal that contains a comma stays complete.
const SPRINT_KEYS: &[&str] = &[
    "activatedDate",
    "autoStartStop",
    "completeDate",
    "endDate",
    "goal",
    "id",
    "incompleteIssuesDestinationId",
    "name",
    "rapidViewId",
    "sequence",
    "startDate",
    "state",
    "synced",
];

/// Read one Sprint from the legacy string
/// `com.atlassian.greenhopper.service.sprint.Sprint@994371b[id=43197,name=S,state=CLOSED,...]`.
fn sprint_from_string(s: &str) -> Option<SprintRef> {
    let open = s.find('[')?;
    let close = s.rfind(']').filter(|c| *c > open)?;
    let body = &s[open + 1..close];
    // The start of each `key=` part: at the start of the body, or after a comma.
    let mut starts: Vec<(usize, &str)> = Vec::new();
    for key in SPRINT_KEYS {
        let pat = format!("{key}=");
        if body.starts_with(&pat) {
            starts.push((0, key));
        }
        let comma = format!(",{pat}");
        let mut from = 0;
        while let Some(i) = body[from..].find(&comma) {
            starts.push((from + i + 1, key));
            from += i + 1;
        }
    }
    starts.sort();
    let value = |key: &str| -> Option<&str> {
        let n = starts.iter().position(|(_, k)| *k == key)?;
        let begin = starts[n].0 + key.len() + 1;
        let end = starts.get(n + 1).map(|(i, _)| i - 1).unwrap_or(body.len());
        body.get(begin..end.max(begin))
    };
    Some(SprintRef {
        id: value("id")?.parse().ok()?,
        name: value("name").unwrap_or_default().to_string(),
        state: value("state").unwrap_or_default().to_lowercase(),
    })
}

/// The open Sprint from the Sprint field of a search (`customfield_10020`). The field is a
/// list of legacy strings (Data Center) or of objects. Active Sprints come first, then future
/// Sprints; the lowest ID wins in each state. Closed Sprints are ignored.
pub fn sprint_from_field(v: &serde_json::Value) -> Option<SprintRef> {
    let items = v.as_array()?;
    let sprints = items.iter().filter_map(|i| match i {
        serde_json::Value::String(s) => sprint_from_string(s),
        serde_json::Value::Object(o) => Some(SprintRef {
            id: o.get("id")?.as_u64()?,
            name: o.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
            state: o.get("state").and_then(|n| n.as_str()).unwrap_or_default().to_lowercase(),
        }),
        _ => None,
    });
    sprints.filter(|s| s.is_open()).min_by_key(|s| (!s.is_active(), s.id))
}

// ---------------------------------------------------------------- full Ticket

/// A Ticket with all fields that `view` shows. Text fields have both the
/// original wiki markup and the Markdown (edit mode).
#[derive(Debug, Clone, Serialize)]
pub struct Ticket {
    pub key: String,
    pub url: String,
    pub summary: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    pub status_category: StatusCategory,
    /// The status ID. A Board column contains statuses by ID.
    pub status_id: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<User>,
    pub reporter: Option<User>,
    pub labels: Vec<String>,
    pub components: Vec<String>,
    pub fix_versions: Vec<String>,
    pub story_points: Option<f64>,
    pub created: Option<DateTime<FixedOffset>>,
    pub updated: Option<DateTime<FixedOffset>>,
    pub description: Option<Text>,
    pub parent: Option<Related>,
    pub epic: Option<EpicRef>,
    pub sprint: Option<SprintRef>,
    pub subtasks: Vec<Related>,
    pub links: Vec<Link>,
    pub attachments: Vec<Attachment>,
    pub comments: Vec<Comment>,
    pub comment_total: u64,
    pub worklogs: Vec<Worklog>,
    pub worklog_total: u64,
    /// True when the current user watches the Ticket.
    pub watching: bool,
    pub watch_count: u64,
}

/// A text field: the original wiki markup and the Markdown in edit mode.
#[derive(Debug, Clone, Serialize)]
pub struct Text {
    pub wiki: String,
    pub markdown: String,
}

impl Text {
    pub fn from_wiki(wiki: String) -> Self {
        let markdown = super::markup::wiki_to_markdown(&wiki).text;
        Text { wiki, markdown }
    }
}

/// Another Ticket that this Ticket refers to (parent, Sub-task, Link).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Related {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub status_category: StatusCategory,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Link {
    pub id: String,
    pub link_type: String,
    /// The relation text from this Ticket's side, for example "blocks" or "is blocked by".
    pub relation: String,
    pub ticket: Related,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpicRef {
    pub key: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SprintRef {
    pub id: u64,
    pub name: String,
    pub state: String,
}

impl SprintRef {
    pub fn is_active(&self) -> bool {
        self.state.eq_ignore_ascii_case("active")
    }

    /// Active or future: not closed.
    pub fn is_open(&self) -> bool {
        !self.state.eq_ignore_ascii_case("closed")
    }
}

/// A Sprint with its dates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sprint {
    pub id: u64,
    pub name: String,
    /// `active`, `future`, or `closed`.
    pub state: String,
    pub start: Option<DateTime<FixedOffset>>,
    pub end: Option<DateTime<FixedOffset>>,
    pub complete: Option<DateTime<FixedOffset>>,
    pub goal: Option<String>,
    /// The Board that made the Sprint. A Board can also show Sprints of other Boards.
    pub board_id: Option<u64>,
}

impl Sprint {
    pub fn is_active(&self) -> bool {
        self.state.eq_ignore_ascii_case("active")
    }

    pub fn is_closed(&self) -> bool {
        self.state.eq_ignore_ascii_case("closed")
    }

    pub fn to_ref(&self) -> SprintRef {
        SprintRef { id: self.id, name: self.name.clone(), state: self.state.clone() }
    }
}

impl From<api::Sprint> for Sprint {
    fn from(s: api::Sprint) -> Self {
        Sprint {
            start: date(&s.start_date),
            end: date(&s.end_date),
            complete: date(&s.complete_date),
            goal: s.goal.filter(|g| !g.trim().is_empty()),
            board_id: s.origin_board_id,
            id: s.id,
            name: s.name,
            state: s.state.to_lowercase(),
        }
    }
}

/// An Epic: a Ticket that contains other Tickets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Epic {
    pub key: String,
    /// The Epic Name. When the Epic has no name, it is the summary.
    pub name: String,
    pub summary: String,
    pub done: bool,
}

impl From<api::EpicIssue> for Epic {
    fn from(i: api::EpicIssue) -> Self {
        let summary = i.fields.summary.unwrap_or_default();
        let name = i.fields.epic_name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| summary.trim().to_string());
        // Done when the Epic Status or the workflow status is Done: the two can differ.
        let status_done = i.fields.status.as_ref().and_then(|s| s.status_category.as_ref()).is_some_and(|c| c.key == "done");
        let done = status_done || i.fields.epic_status.and_then(|s| s.value).is_some_and(|v| v.eq_ignore_ascii_case("done"));
        Epic { key: i.key, name, summary, done }
    }
}

impl From<api::Epic> for Epic {
    fn from(e: api::Epic) -> Self {
        let summary = e.summary.unwrap_or_default();
        let name = e.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| summary.trim().to_string());
        Epic { key: e.key, name, summary, done: e.done }
    }
}

/// The data of the Sprint tab and of `jui sprint active`: a Scrum Board, its active
/// Sprints, and the JQL of its backlog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SprintBoard {
    pub board: Board,
    pub sprints: Vec<SprintRef>,
    /// `None` when the Board has no filter.
    pub backlog_jql: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub size: u64,
    pub mime_type: Option<String>,
    pub author: Option<User>,
    pub created: Option<DateTime<FixedOffset>>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub id: String,
    pub author: Option<User>,
    pub created: Option<DateTime<FixedOffset>>,
    pub updated: Option<DateTime<FixedOffset>>,
    pub body: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Worklog {
    pub id: String,
    pub author: Option<User>,
    pub started: Option<DateTime<FixedOffset>>,
    pub updated: Option<DateTime<FixedOffset>>,
    pub time_spent: String,
    pub time_spent_seconds: u64,
    /// The comment in wiki markup.
    pub comment: Option<String>,
}

impl From<api::Worklog> for Worklog {
    fn from(w: api::Worklog) -> Self {
        Worklog {
            started: date(&w.started),
            updated: date(&w.updated),
            id: w.id,
            author: user(w.author),
            time_spent: w.time_spent.unwrap_or_default(),
            time_spent_seconds: w.time_spent_seconds,
            comment: w.comment.filter(|c| !c.trim().is_empty()),
        }
    }
}

/// A Link type. `inward` and `outward` are the relation texts, for example "is blocked by"
/// and "blocks".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkType {
    pub id: String,
    pub name: String,
    pub inward: String,
    pub outward: String,
}

impl From<api::LinkType> for LinkType {
    fn from(t: api::LinkType) -> Self {
        LinkType { id: t.id, name: t.name, inward: t.inward, outward: t.outward }
    }
}

/// The Links of one Ticket (`jui link list`).
#[derive(Debug, Clone, Serialize)]
pub struct TicketLinks {
    pub key: String,
    pub summary: String,
    pub links: Vec<Link>,
}

/// The data that a Worklog Template needs: the summary, the time tracking, and the Worklogs.
#[derive(Debug, Clone, Default)]
pub struct WorklogContext {
    pub key: String,
    pub summary: String,
    pub original_estimate: Option<String>,
    pub remaining_estimate: Option<String>,
    pub time_spent: Option<String>,
    pub worklogs: Vec<Worklog>,
}

impl WorklogContext {
    pub fn find(&self, id: &str) -> Option<&Worklog> {
        self.worklogs.iter().find(|w| w.id == id)
    }
}

/// The Links of an issue from this Ticket's side. A Link with no other Ticket is ignored.
pub fn links_from_api(ls: Vec<api::IssueLink>) -> Vec<Link> {
    ls.into_iter()
        .filter_map(|l| {
            let (relation, other) = match (l.outward_issue, l.inward_issue) {
                (Some(o), _) => (l.kind.outward, o),
                (None, Some(i)) => (l.kind.inward, i),
                (None, None) => return None,
            };
            Some(Link { id: l.id, link_type: l.kind.name, relation, ticket: other.into() })
        })
        .collect()
}

fn user(u: Option<api::User>) -> Option<User> {
    u.map(|u| User { display_name: u.display_name.unwrap_or_else(|| u.name.clone()), name: u.name })
}

fn status(s: Option<api::Status>) -> (String, StatusCategory) {
    match s {
        Some(s) => {
            let cat = s.status_category.map(|c| StatusCategory::from_key(&c.key)).unwrap_or(StatusCategory::Unknown);
            (s.name, cat)
        }
        None => (String::new(), StatusCategory::Unknown),
    }
}

fn date(s: &Option<String>) -> Option<DateTime<FixedOffset>> {
    s.as_deref().and_then(parse_jira_date)
}

impl From<api::Attachment> for Attachment {
    fn from(a: api::Attachment) -> Self {
        Attachment {
            created: date(&a.created),
            id: a.id,
            filename: a.filename,
            size: a.size,
            mime_type: a.mime_type,
            author: user(a.author),
            url: a.content,
        }
    }
}

impl From<api::LinkedIssue> for Related {
    fn from(i: api::LinkedIssue) -> Self {
        let (status, status_category) = status(i.fields.status);
        Related {
            key: i.key,
            summary: i.fields.summary.unwrap_or_default(),
            status,
            status_category,
            kind: i.fields.issuetype.map(|t| t.name).unwrap_or_default(),
        }
    }
}

impl Ticket {
    /// Build a Ticket from the v2 issue and the Agile issue (Sprint and Epic).
    pub fn from_api(i: api::FullIssue, agile: Option<api::AgileIssue>, url: String) -> Self {
        let mut f = i.fields;
        let status_id = f.status.as_mut().and_then(|s| s.id.take());
        let (st, cat) = status(f.status);
        let agile = agile.map(|a| a.fields).unwrap_or_default();
        let epic = match (agile.epic, f.epic_link) {
            (Some(e), _) => Some(EpicRef { key: e.key, name: e.name.or(e.summary) }),
            (None, Some(k)) => Some(EpicRef { key: k, name: None }),
            (None, None) => None,
        };
        let sprint = agile.sprint.or_else(|| agile.closed_sprints.into_iter().max_by_key(|s| s.id));
        let links = links_from_api(f.issuelinks);
        let comments = f.comment.unwrap_or_default();
        let worklogs = f.worklog.unwrap_or_default();
        let watches = f.watches.unwrap_or_default();
        Ticket {
            key: i.key,
            url,
            summary: f.summary.unwrap_or_default(),
            kind: f.issuetype.map(|t| t.name).unwrap_or_default(),
            status: st,
            status_category: cat,
            status_id,
            priority: f.priority.map(|p| p.name),
            assignee: user(f.assignee),
            reporter: user(f.reporter),
            labels: f.labels,
            components: f.components.into_iter().map(|c| c.name).collect(),
            fix_versions: f.fix_versions.into_iter().map(|c| c.name).collect(),
            story_points: f.story_points,
            created: date(&f.created),
            updated: date(&f.updated),
            description: f.description.filter(|d| !d.trim().is_empty()).map(Text::from_wiki),
            parent: f.parent.map(Into::into),
            epic,
            sprint: sprint.map(|s| SprintRef { id: s.id, name: s.name, state: s.state }),
            subtasks: f.subtasks.into_iter().map(Into::into).collect(),
            links,
            attachments: f.attachment.into_iter().map(Attachment::from).collect(),
            comment_total: comments.total.max(comments.comments.len() as u64),
            comments: comments.comments.into_iter().map(Into::into).collect(),
            worklog_total: worklogs.total.max(worklogs.worklogs.len() as u64),
            worklogs: worklogs.worklogs.into_iter().map(Worklog::from).collect(),
            watching: watches.is_watching,
            watch_count: watches.watch_count,
        }
    }
}

/// The data that a comment Template needs: the Ticket summary and its comments.
#[derive(Debug, Clone)]
pub struct CommentContext {
    pub key: String,
    pub summary: String,
    /// The comments, oldest first.
    pub comments: Vec<Comment>,
}

impl CommentContext {
    pub fn find(&self, id: &str) -> Option<&Comment> {
        self.comments.iter().find(|c| c.id == id)
    }
}

impl From<api::Comment> for Comment {
    fn from(c: api::Comment) -> Self {
        Comment {
            created: date(&c.created),
            updated: date(&c.updated),
            id: c.id,
            author: user(c.author),
            body: Text::from_wiki(c.body),
        }
    }
}

impl From<&Ticket> for TicketSummary {
    fn from(t: &Ticket) -> Self {
        TicketSummary {
            key: t.key.clone(),
            summary: t.summary.clone(),
            kind: t.kind.clone(),
            status: t.status.clone(),
            status_category: t.status_category,
            priority: t.priority.clone(),
            assignee: t.assignee.clone(),
            updated: t.updated,
            // A closed Sprint is the last Sprint of a Ticket that is in no open Sprint now.
            sprint: t.sprint.clone().filter(SprintRef::is_open),
        }
    }
}

// ---------------------------------------------------------------- saved filters

/// A saved Jira filter: a name and a JQL query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Filter {
    pub id: String,
    pub name: String,
    /// The username of the owner.
    pub owner: Option<String>,
    pub jql: String,
    pub description: Option<String>,
    pub favourite: bool,
    /// The browser URL of the filter.
    pub url: String,
}

impl Filter {
    /// Make a filter from the API type. `base_url` makes the URL when Jira does not send `viewUrl`.
    pub fn from_api(f: api::Filter, base_url: &str) -> Self {
        let url = f.view_url.clone().unwrap_or_else(|| format!("{base_url}/issues/?filter={}", f.id));
        Filter {
            owner: f.owner.map(|u| u.name),
            description: f.description.filter(|d| !d.trim().is_empty()),
            jql: f.jql,
            favourite: f.favourite,
            url,
            name: f.name,
            id: f.id,
        }
    }
}

// ---------------------------------------------------------------- Boards

/// A Jira Agile Board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Board {
    pub id: u64,
    pub name: String,
    /// `scrum` or `kanban` (other text when Jira sends a different type).
    #[serde(rename = "type")]
    pub kind: String,
}

impl Board {
    pub fn is_scrum(&self) -> bool {
        self.kind.eq_ignore_ascii_case("scrum")
    }

    /// The browser URL of the Board.
    pub fn url(&self, base_url: &str) -> String {
        format!("{base_url}/secure/RapidBoard.jspa?rapidView={}", self.id)
    }

    /// `4166  BCE board (scrum)`.
    pub fn label(&self) -> String {
        format!("{}  {} ({})", self.id, self.name, self.kind)
    }
}

impl From<api::Board> for Board {
    fn from(b: api::Board) -> Self {
        Board { id: b.id, name: b.name, kind: b.kind }
    }
}

/// The configuration of a Board: its filter, its Kanban sub-query, and its columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardConfig {
    pub board: Board,
    pub filter_id: Option<String>,
    pub sub_query: Option<String>,
    /// Each column: the name and the status IDs.
    pub columns: Vec<(String, Vec<String>)>,
}

impl From<api::BoardConfig> for BoardConfig {
    fn from(c: api::BoardConfig) -> Self {
        BoardConfig {
            board: Board { id: c.id, name: c.name, kind: c.kind },
            filter_id: c.filter.map(|f| f.id),
            sub_query: c.sub_query.map(|s| s.query).filter(|q| !q.trim().is_empty()),
            columns: c.column_config.columns.into_iter().map(|col| (col.name, col.statuses.into_iter().map(|s| s.id).collect())).collect(),
        }
    }
}

/// One column of a Board and its Tickets.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardColumn {
    pub name: String,
    /// The status IDs that the column contains.
    pub status_ids: Vec<String>,
    pub tickets: Vec<TicketSummary>,
}

/// A Board with its columns and the Tickets that the Board shows. See ticket 12, part 3:
/// a Scrum Board shows the active Sprints; a Kanban Board shows its filter, its sub-query,
/// and the Done Tickets of the last 14 days.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardView {
    pub board: Board,
    pub url: String,
    /// The ID of the saved filter of the Board.
    pub filter_id: Option<String>,
    /// The Kanban sub-query from the Board configuration.
    pub sub_query: Option<String>,
    /// The active Sprints (Scrum Boards only).
    pub sprints: Vec<SprintRef>,
    /// The JQL that jui added to the Board filter. `None` when jui did not get Tickets.
    pub jql: Option<String>,
    pub columns: Vec<BoardColumn>,
    /// The Tickets that have a status that is in no column. The Board does not show them.
    pub unmapped: u64,
    /// The number of Tickets that the Board query finds.
    pub total: u64,
    /// The number of Tickets that jui got (at most the limit).
    pub loaded: u64,
    /// The Board shows only the Tickets that are assigned to the current user.
    pub only_mine: bool,
}

impl BoardView {
    /// True for a Scrum Board with no active Sprint.
    pub fn no_active_sprint(&self) -> bool {
        self.board.is_scrum() && self.sprints.is_empty()
    }

    /// True when the Board has more Tickets than jui got.
    pub fn truncated(&self) -> bool {
        self.loaded < self.total
    }

    /// The count text for the title: `670`, or `1000 of 1534`.
    pub fn count(&self) -> String {
        if self.truncated() { format!("{} of {}", self.loaded, self.total) } else { self.total.to_string() }
    }
}

// ---------------------------------------------------------------- create and edit

/// A Ticket type of a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueType {
    pub id: String,
    pub name: String,
    pub subtask: bool,
}

/// The fields that a create screen has (createmeta).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateMeta {
    pub project: String,
    pub issue_type: IssueType,
    pub fields: Vec<ScreenField>,
}

/// The data that the edit Template needs: the edit screen fields (editmeta) and the
/// current values. The values are the Jira JSON of each field, because their shape
/// depends on the field type. `core::template` reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct EditData {
    pub key: String,
    pub project: String,
    pub issue_type: String,
    pub summary: String,
    /// The `updated` text from Jira, to find changes during the edit.
    pub updated: Option<String>,
    /// The description in wiki markup.
    pub description: Option<String>,
    pub fields: Vec<ScreenField>,
    pub values: std::collections::BTreeMap<String, serde_json::Value>,
}

impl From<api::EditIssue> for EditData {
    fn from(i: api::EditIssue) -> Self {
        let mut values: std::collections::BTreeMap<String, serde_json::Value> = i.fields.into_iter().collect();
        let text = |v: Option<&serde_json::Value>, k: &str| {
            v.and_then(|v| v.get(k).or(Some(v))).and_then(|v| v.as_str()).map(str::to_string).unwrap_or_default()
        };
        let project = text(values.get("project"), "key");
        let issue_type = text(values.get("issuetype"), "name");
        let summary = text(values.get("summary"), "");
        let updated = values.get("updated").and_then(|v| v.as_str()).map(str::to_string);
        let description = values.remove("description").and_then(|v| v.as_str().map(str::to_string));
        let fields = i.editmeta.fields.into_iter().map(|(id, m)| ScreenField::from_api(id, m)).collect();
        EditData { key: i.key, project, issue_type, summary, updated, description, fields, values }
    }
}

// ---------------------------------------------------------------- Transitions

/// A permitted move of a Ticket to a different Status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Transition {
    pub id: String,
    pub name: String,
    /// The destination Status.
    pub to: String,
    pub to_category: StatusCategory,
    /// The fields on the Transition screen.
    pub fields: Vec<ScreenField>,
}

/// A field on a screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScreenField {
    pub id: String,
    pub name: String,
    pub required: bool,
    pub has_default: bool,
    pub kind: FieldKind,
    /// The permitted values (archived values are removed).
    pub allowed: Vec<Choice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Number,
    User,
    Labels,
    /// One permitted value.
    Choice,
    /// Many permitted values (comma-separated).
    MultiChoice,
    /// A date, `YYYY-MM-DD`.
    Date,
    /// A Ticket key (Epic Link, parent).
    Key,
    /// A type that jui cannot set. The text is the Jira schema type.
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
}

impl ScreenField {
    pub fn from_api(id: String, m: api::FieldMeta) -> Self {
        let allowed: Vec<Choice> = m
            .allowed_values
            .into_iter()
            .filter(|a| !a.archived && !a.disabled)
            .filter_map(|a| a.name.or(a.value).map(|name| Choice { id: a.id, name }))
            .collect();
        let s = &m.schema;
        let custom = s.custom.as_deref().unwrap_or("");
        let kind = match (s.kind.as_str(), s.items.as_deref()) {
            _ if custom.ends_with(":gh-sprint") => FieldKind::Other("sprint".into()),
            _ if custom.ends_with(":gh-epic-link") => FieldKind::Key,
            ("issuelink", _) => FieldKind::Key,
            // Cascading select lists have permitted values, but jui cannot set them.
            ("option-with-child", _) => FieldKind::Other("option-with-child".into()),
            _ if !allowed.is_empty() && s.kind == "array" => FieldKind::MultiChoice,
            _ if !allowed.is_empty() => FieldKind::Choice,
            ("string", _) => FieldKind::Text,
            ("number", _) => FieldKind::Number,
            ("date", _) => FieldKind::Date,
            ("user", _) => FieldKind::User,
            ("array", Some("string")) => FieldKind::Labels,
            (k, Some(i)) => FieldKind::Other(format!("{k} of {i}")),
            (k, None) => FieldKind::Other(k.to_string()),
        };
        ScreenField { id, name: m.name, required: m.required, has_default: m.has_default_value, kind, allowed }
    }
}

impl From<api::ApiTransition> for Transition {
    fn from(t: api::ApiTransition) -> Self {
        let (to, to_category) = status(Some(t.to));
        // Required fields first, so that prompts ask for them first.
        let mut fields: Vec<ScreenField> = t.fields.into_iter().map(|(id, m)| ScreenField::from_api(id, m)).collect();
        fields.sort_by_key(|f| !f.required);
        Transition { id: t.id, name: t.name, to, to_category, fields }
    }
}

/// A JQL query that finds only one Ticket, if the query finds it.
/// The `ORDER BY` part is removed, because it cannot be inside the parentheses.
pub fn jql_with_key(jql: &str, key: &str) -> String {
    let lower = jql.to_lowercase();
    let body = match lower.rfind("order by") {
        Some(i) => &jql[..i],
        None => jql,
    };
    let body = body.trim();
    if body.is_empty() { format!("key = {key}") } else { format!("key = {key} AND ({body})") }
}

/// Make a short key complete: `62886` becomes `BCE-62886`. Other text is upper-cased.
pub fn normalize_key(raw: &str, default_project: &str) -> String {
    let raw = raw.trim();
    if !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()) {
        format!("{default_project}-{raw}")
    } else {
        raw.to_uppercase()
    }
}

/// True when the text looks like a Ticket key (`ABC-123`) or a short key (`123`).
pub fn looks_like_key(s: &str) -> bool {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let Some((proj, num)) = s.split_once('-') else { return false };
    let mut pc = proj.chars();
    matches!(pc.next(), Some(c) if c.is_ascii_alphabetic())
        && pc.all(|c| c.is_ascii_alphanumeric())
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jira_dates() {
        let d = parse_jira_date("2026-10-19T00:30:46.000-0700").unwrap();
        assert_eq!(d.to_rfc3339(), "2026-10-19T00:30:46-07:00");
    }

    #[test]
    fn short_keys() {
        assert_eq!(normalize_key("62886", "BCE"), "BCE-62886");
        assert_eq!(normalize_key("xsup-1", "BCE"), "XSUP-1");
        assert!(looks_like_key("BCE-1"));
        assert!(looks_like_key("bce-1"));
        assert!(looks_like_key("62886"));
        assert!(!looks_like_key("ls"));
        assert!(!looks_like_key("BCE-"));
        assert!(!looks_like_key("-1"));
    }

    #[test]
    fn jql_for_one_key() {
        assert_eq!(
            jql_with_key("assignee = currentUser() ORDER BY updated DESC", "BCE-1"),
            "key = BCE-1 AND (assignee = currentUser())"
        );
        assert_eq!(jql_with_key("order by key", "BCE-1"), "key = BCE-1");
    }

    #[test]
    fn sprint_field_gives_the_open_sprint() {
        let closed = "com.atlassian.greenhopper.service.sprint.Sprint@994371b[activatedDate=2026-09-22T09:00:00.000-07:00,\
autoStartStop=true,completeDate=2026-10-06T09:00:00.000-07:00,endDate=2026-10-06T09:00:00.000-07:00,goal=,id=43197,\
incompleteIssuesDestinationId=43198,name=BCE-26-Sprint-20,rapidViewId=4166,sequence=43197,startDate=2026-09-22T09:00:00.000-07:00,\
state=CLOSED,synced=false]";
        let active = "com.atlassian.greenhopper.service.sprint.Sprint@1f2e[activatedDate=<null>,autoStartStop=false,completeDate=<null>,\
endDate=2026-10-20T09:00:00.000-07:00,goal=Ship it, then rest,id=43198,incompleteIssuesDestinationId=<null>,\
name=BCE-26, Sprint=21,rapidViewId=4166,sequence=43198,startDate=2026-10-06T09:00:00.000-07:00,state=ACTIVE,synced=false]";
        let future = "x.Sprint@3[id=43199,name=BCE-26-Sprint-22,state=FUTURE]";
        let v = serde_json::json!([closed, future, active]);
        let s = sprint_from_field(&v).unwrap();
        assert_eq!(s, SprintRef { id: 43198, name: "BCE-26, Sprint=21".into(), state: "active".into() });

        // Future, when there is no active Sprint. Closed only, or no field: no Sprint.
        assert_eq!(sprint_from_field(&serde_json::json!([closed, future])).unwrap().id, 43199);
        assert_eq!(sprint_from_field(&serde_json::json!([closed])), None);
        assert_eq!(sprint_from_field(&serde_json::Value::Null), None);
        assert_eq!(sprint_from_field(&serde_json::json!(["garbage", "x[name=No ID,state=ACTIVE]"])), None);

        // Objects (Jira Cloud and some Data Center versions).
        let v = serde_json::json!([{ "id": 7, "name": "S7", "state": "active" }]);
        assert_eq!(sprint_from_field(&v).unwrap().name, "S7");
    }

    #[test]
    fn page_has_more() {
        let p = Page { items: vec![1, 2], start_at: 0, total: 3 };
        assert!(p.has_more());
        let p = Page { items: vec![3], start_at: 2, total: 3 };
        assert!(!p.has_more());
    }
}
