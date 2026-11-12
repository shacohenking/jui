//! The `Jira` client: one struct with typed async methods.

use std::time::{Duration, Instant};

use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

use super::error::{Error, Result};
use super::model::{
    Attachment, Board, BoardConfig, BoardView, Comment, CommentContext, CreateMeta, EditData, Epic, Filter, IssueType, LinkType, ListRow, Page,
    Related, ScreenField, Sprint, SprintBoard, SprintRef, Ticket, TicketLinks, TicketSummary, Transition, User, Watchers, Worklog,
    WorklogContext, links_from_api,
};
use super::{agile, api, attachment, board, link, worklog};
use super::user::Assignee;
use super::settings::Settings;

/// The delays between retries of a GET request.
const RETRY_DELAYS: [Duration; 3] = [Duration::from_millis(500), Duration::from_secs(1), Duration::from_secs(2)];
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct Jira {
    http: reqwest::Client,
    base_url: String,
    token: String,
    timeout_secs: u64,
    retry_delays: Vec<Duration>,
}

impl std::fmt::Debug for Jira {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never show the token.
        f.debug_struct("Jira").field("base_url", &self.base_url).finish_non_exhaustive()
    }
}

impl Jira {
    /// Make a client. The token comes only from `JIRA_TOKEN` (see the caller).
    pub fn new(settings: &Settings, token: Option<String>) -> Result<Self> {
        let token = token.filter(|t| !t.trim().is_empty()).ok_or(Error::NoToken)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(settings.timeout_secs))
            .user_agent(concat!("jui/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Network { host: settings.base_url.clone(), cause: e.to_string() })?;
        Ok(Jira {
            http,
            base_url: settings.base_url.trim_end_matches('/').to_string(),
            token: token.trim().to_string(),
            timeout_secs: settings.timeout_secs,
            retry_delays: RETRY_DELAYS.to_vec(),
        })
    }

    /// Change the retry delays (tests use zero delays).
    pub fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = delays;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The browser URL of a Ticket.
    pub fn browse_url(&self, key: &str) -> String {
        format!("{}/browse/{key}", self.base_url)
    }

    // ------------------------------------------------------------ operations

    /// Search with JQL. Returns one page.
    pub async fn search(&self, jql: &str, start_at: u64, max_results: u64) -> Result<Page<TicketSummary>> {
        let r = self.search_page(jql, start_at, max_results, api::LIST_FIELDS).await?;
        Ok(Page { start_at: r.start_at, total: r.total, items: r.issues.into_iter().map(Into::into).collect() })
    }

    /// Search with JQL for a TUI list. Each row also has the description for the preview.
    pub async fn search_rows(&self, jql: &str, start_at: u64, max_results: u64) -> Result<Page<ListRow>> {
        let r = self.search_page(jql, start_at, max_results, api::PREVIEW_FIELDS).await?;
        Ok(Page { start_at: r.start_at, total: r.total, items: r.issues.into_iter().map(Into::into).collect() })
    }

    async fn search_page(&self, jql: &str, start_at: u64, max_results: u64, fields: &str) -> Result<api::SearchResponse> {
        let start = start_at.to_string();
        let max = max_results.to_string();
        let query = [("jql", jql), ("startAt", start.as_str()), ("maxResults", max.as_str()), ("fields", fields)];
        self.get_json("/rest/api/2/search", &query).await
    }

    /// Search with JQL and get pages until `limit` results (or all, when `limit` is `None`).
    pub async fn search_all(&self, jql: &str, limit: Option<u64>) -> Result<Page<TicketSummary>> {
        // The server caps a page (4000 on this server). 1000 keeps each response small.
        const MAX_PAGE: u64 = 1000;
        let mut items = Vec::new();
        let mut total;
        loop {
            let want = match limit {
                Some(l) => (l - items.len() as u64).min(MAX_PAGE),
                None => MAX_PAGE,
            };
            let page = self.search(jql, items.len() as u64, want).await?;
            total = page.total;
            let got = page.items.len();
            items.extend(page.items);
            let done = got == 0
                || items.len() as u64 >= total
                || limit.is_some_and(|l| items.len() as u64 >= l);
            if done {
                break;
            }
        }
        if let Some(l) = limit {
            items.truncate(l as usize);
        }
        Ok(Page { items, start_at: 0, total })
    }

    /// The Epics of the user: each Epic that has at least one Ticket that is assigned to the
    /// user and is not Done, with the number of those Tickets. The most Tickets first.
    pub async fn my_epic_counts(&self) -> Result<Vec<(String, usize)>> {
        const PAGE: u64 = 1000;
        let mut links: Vec<String> = Vec::new();
        let mut start = 0u64;
        loop {
            let r = self.search_page(super::agile::MY_EPIC_TICKETS_JQL, start, PAGE, api::MY_EPIC_FIELDS).await?;
            let got = r.issues.len() as u64;
            links.extend(r.issues.into_iter().filter_map(|i| i.fields.epic_link));
            start += got;
            if got == 0 || start >= r.total {
                break;
            }
        }
        Ok(super::agile::count_by_epic(links.iter().map(String::as_str)))
    }

    /// Read one Ticket with all fields that `view` shows. Also reads the Sprint and
    /// the Epic from the Agile API; if that request fails, the Ticket has no Sprint.
    pub async fn ticket(&self, key: &str) -> Result<Ticket> {
        let path = format!("/rest/api/2/issue/{key}");
        let agile_path = format!("/rest/agile/1.0/issue/{key}");
        let (issue, agile) = tokio::join!(
            self.get_json::<api::FullIssue>(&path, &[("fields", api::VIEW_FIELDS)]),
            self.get_json::<api::AgileIssue>(&agile_path, &[("fields", "sprint,closedSprints,epic")]),
        );
        let issue = issue?;
        let agile = match agile {
            Ok(a) => Some(a),
            Err(e) => {
                tracing::debug!(target: "jui::http", "no Agile data for {key}: {e}");
                None
            }
        };
        let url = self.browse_url(&issue.key);
        let mut t = Ticket::from_api(issue, agile, url);
        // The issue response has at most 20 Worklogs. Get all when there are more.
        if t.worklog_total > t.worklogs.len() as u64 {
            let page: api::WorklogPage = self.get_json(&format!("{path}/worklog"), &[]).await?;
            let all = api::FullIssue {
                id: String::new(),
                key: t.key.clone(),
                fields: api::FullFields { worklog: Some(page), ..Default::default() },
            };
            t.worklogs = Ticket::from_api(all, None, String::new()).worklogs;
        }
        Ok(t)
    }

    /// The Transitions of a Ticket, with the screen fields. jui gets them each time,
    /// because the IDs change with the Ticket type.
    pub async fn transitions(&self, key: &str) -> Result<Vec<Transition>> {
        let path = format!("/rest/api/2/issue/{key}/transitions");
        let r: api::TransitionsResponse = self.get_json(&path, &[("expand", "transitions.fields")]).await?;
        Ok(r.transitions.into_iter().map(Into::into).collect())
    }

    /// Do a Transition. `fields` has the screen field values (JSON). `comment` is Markdown;
    /// jui converts it to wiki markup. A validation error names the fields.
    pub async fn transition(
        &self,
        key: &str,
        t: &Transition,
        fields: &serde_json::Map<String, serde_json::Value>,
        comment: Option<&str>,
    ) -> Result<()> {
        let wiki = comment.map(|c| super::markup::markdown_to_wiki(c).text);
        let body = super::transition::request_body(&t.id, fields, wiki.as_deref());
        let path = format!("/rest/api/2/issue/{key}/transitions");
        self.send_retry(Method::POST, &path, |rb| rb.json(&body))
            .await
            .map(|_| ())
            .map_err(|e| super::transition::name_fields(e, &t.fields))
    }

    // ------------------------------------------------------------ create and edit

    /// The username of the current user (`me` in Templates).
    pub async fn myself(&self) -> Result<String> {
        let u: api::User = self.get_json("/rest/api/2/myself", &[]).await?;
        Ok(u.name)
    }

    /// Check that a project exists and that the user can see it. Returns the key as Jira
    /// writes it. A project that does not exist gives `Error::NotFound`.
    pub async fn project_key(&self, key: &str) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct P {
            key: String,
        }
        let key = key.trim().to_uppercase();
        let p: P = self.get_json(&format!("/rest/api/2/project/{key}"), &[]).await?;
        Ok(p.key)
    }

    /// The Ticket types of a project.
    pub async fn issue_types(&self, project: &str) -> Result<Vec<IssueType>> {
        let path = format!("/rest/api/2/issue/createmeta/{project}/issuetypes");
        let r: api::MetaPage<api::IssueTypeMeta> = self.get_json(&path, &[("maxResults", "100")]).await?;
        Ok(r.values.into_iter().map(|t| IssueType { id: t.id, name: t.name, subtask: t.subtask }).collect())
    }

    /// The create screen of a project and a Ticket type. The type matches by name (ignores
    /// case, a unique prefix is enough), or `subtask` selects the first Sub-task type.
    /// jui gets it each time (see the caching fog in the map).
    pub async fn create_meta(&self, project: &str, type_name: &str) -> Result<CreateMeta> {
        let project = project.trim().to_uppercase();
        let types = self.issue_types(&project).await?;
        let t = pick_type(&types, type_name).map_err(|hint| {
            Error::input_hint(format!("Project {project} has no Ticket type `{type_name}`."), hint)
        })?;
        let path = format!("/rest/api/2/issue/createmeta/{project}/issuetypes/{}", t.id);
        let mut fields: Vec<ScreenField> = Vec::new();
        let mut start = 0u64;
        loop {
            let s = start.to_string();
            let r: api::MetaPage<api::FieldMeta> = self.get_json(&path, &[("startAt", s.as_str()), ("maxResults", "200")]).await?;
            let got = r.values.len() as u64;
            for m in r.values {
                let id = m.field_id.clone().unwrap_or_default();
                fields.push(ScreenField::from_api(id, m));
            }
            start += got;
            if got == 0 || r.is_last.unwrap_or(start >= r.total) || start >= r.total {
                break;
            }
        }
        Ok(CreateMeta { project, issue_type: t.clone(), fields })
    }

    /// The edit screen of a Ticket and its current values. `fields=*navigable` is necessary,
    /// because Jira returns editmeta only for the fields in the request.
    pub async fn edit_data(&self, key: &str) -> Result<EditData> {
        let path = format!("/rest/api/2/issue/{key}");
        let r: api::EditIssue = self.get_json(&path, &[("expand", "editmeta"), ("fields", "*navigable")]).await?;
        Ok(r.into())
    }

    /// Create a Ticket. `body` comes from `core::template`. Returns the new key.
    /// A POST is not retried (only HTTP 429).
    pub async fn create(&self, body: &serde_json::Value) -> Result<String> {
        let path = "/rest/api/2/issue";
        let resp = self.send_retry(Method::POST, path, |rb| rb.json(body)).await?;
        let c: api::Created = decode(path, resp).await?;
        Ok(c.key)
    }

    /// Change the fields of a Ticket. `body` comes from `core::template`.
    pub async fn edit(&self, key: &str, body: &serde_json::Value) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}");
        self.send_retry(Method::PUT, &path, |rb| rb.json(body)).await.map(|_| ())
    }

    // ------------------------------------------------------------ comments

    /// All comments of a Ticket, oldest first. jui reads all pages.
    pub async fn comments(&self, key: &str) -> Result<Vec<Comment>> {
        let path = format!("/rest/api/2/issue/{key}/comment");
        let mut out: Vec<Comment> = Vec::new();
        loop {
            let start = out.len().to_string();
            let page: api::CommentPage = self.get_json(&path, &[("startAt", start.as_str()), ("maxResults", "1000")]).await?;
            let got = page.comments.len();
            out.extend(page.comments.into_iter().map(Comment::from));
            if got == 0 || out.len() as u64 >= page.total {
                break;
            }
        }
        Ok(out)
    }

    /// The summary and the comments of a Ticket, for a comment Template. With `id`, the
    /// comment must be in the result: jui gets it alone when the issue response does not have it.
    pub async fn comment_context(&self, key: &str, id: Option<&str>) -> Result<CommentContext> {
        let path = format!("/rest/api/2/issue/{key}");
        let issue: api::FullIssue = self.get_json(&path, &[("fields", "summary,comment")]).await?;
        let page = issue.fields.comment.unwrap_or_default();
        let mut ctx = CommentContext {
            key: issue.key,
            summary: issue.fields.summary.unwrap_or_default(),
            comments: page.comments.into_iter().map(Comment::from).collect(),
        };
        if let Some(id) = id
            && ctx.find(id).is_none()
        {
            let c = self.comment(key, id).await?;
            ctx.comments.push(c);
        }
        Ok(ctx)
    }

    /// One comment.
    pub async fn comment(&self, key: &str, id: &str) -> Result<Comment> {
        let c: api::Comment = self.get_json(&format!("/rest/api/2/issue/{key}/comment/{id}"), &[]).await?;
        Ok(c.into())
    }

    /// Add a comment. `wiki` is the body in wiki markup (the caller converts the Markdown).
    /// A POST is not retried (only HTTP 429), so a network error cannot make two comments.
    pub async fn add_comment(&self, key: &str, wiki: &str) -> Result<Comment> {
        let path = format!("/rest/api/2/issue/{key}/comment");
        let body = serde_json::json!({ "body": wiki });
        let resp = self.send_retry(Method::POST, &path, |rb| rb.json(&body)).await?;
        let c: api::Comment = decode(&path, resp).await?;
        Ok(c.into())
    }

    /// Replace the body of a comment. `wiki` is the new body in wiki markup.
    pub async fn edit_comment(&self, key: &str, id: &str, wiki: &str) -> Result<Comment> {
        let path = format!("/rest/api/2/issue/{key}/comment/{id}");
        let body = serde_json::json!({ "body": wiki });
        let resp = self.send_retry(Method::PUT, &path, |rb| rb.json(&body)).await?;
        let c: api::Comment = decode(&path, resp).await?;
        Ok(c.into())
    }

    /// Delete a comment. This cannot be reversed.
    pub async fn delete_comment(&self, key: &str, id: &str) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}/comment/{id}");
        self.send_retry(Method::DELETE, &path, |rb| rb).await.map(|_| ())
    }

    // ------------------------------------------------------------ users, assign, watch

    /// The users that can be the assignee of a Ticket and that match the text. The server
    /// finds the text at the start of the username, the display name, or the email.
    pub async fn assignable_users(&self, scope: &AssignScope, text: &str, max: u32) -> Result<Vec<User>> {
        let max = max.to_string();
        let (k, v) = match scope {
            AssignScope::Ticket(key) => ("issueKey", key.as_str()),
            AssignScope::Project(p) => ("project", p.as_str()),
        };
        let query = [(k, v), ("username", text), ("maxResults", max.as_str())];
        let r: Vec<api::UserEntry> = self.get_json("/rest/api/2/user/assignable/search", &query).await?;
        Ok(r.into_iter().filter(|u| u.active).map(User::from).collect())
    }

    /// All active users that match the text (for the watchers, who need not be assignable).
    pub async fn search_users(&self, text: &str, max: u32) -> Result<Vec<User>> {
        let max = max.to_string();
        let query = [("username", text), ("maxResults", max.as_str())];
        let r: Vec<api::UserEntry> = self.get_json("/rest/api/2/user/search", &query).await?;
        Ok(r.into_iter().filter(|u| u.active).map(User::from).collect())
    }

    /// Set or remove the assignee. `Me` needs the username, so jui gets it first.
    /// A PUT is not retried (only HTTP 429).
    pub async fn assign(&self, key: &str, who: &Assignee) -> Result<()> {
        let name = match who {
            Assignee::Me => serde_json::Value::String(self.myself().await?),
            Assignee::Nobody => serde_json::Value::Null,
            Assignee::User(u) => serde_json::Value::String(u.name.clone()),
        };
        let body = serde_json::json!({ "name": name });
        let path = format!("/rest/api/2/issue/{key}/assignee");
        self.send_retry(Method::PUT, &path, |rb| rb.json(&body)).await.map(|_| ())
    }

    /// The watchers of a Ticket.
    pub async fn watchers(&self, key: &str) -> Result<Watchers> {
        let r: api::WatchersResponse = self.get_json(&format!("/rest/api/2/issue/{key}/watchers"), &[]).await?;
        Ok(Watchers {
            key: key.to_string(),
            watching: r.is_watching,
            count: r.watch_count.max(r.watchers.len() as u64),
            users: r.watchers.into_iter().map(User::from).collect(),
        })
    }

    /// Add a watcher. With `None`, Jira adds the current user (an empty body).
    /// The body of a named user is a JSON string, for example `"jdoe"`.
    pub async fn watch(&self, key: &str, user: Option<&str>) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}/watchers");
        let body = match user {
            Some(u) => serde_json::to_string(u).unwrap_or_default(),
            None => String::new(),
        };
        self.send_retry(Method::POST, &path, |rb| rb.header("Content-Type", "application/json").body(body.clone()))
            .await
            .map(|_| ())
    }

    /// Remove a watcher. With `None`, jui removes the current user.
    pub async fn unwatch(&self, key: &str, user: Option<&str>) -> Result<()> {
        let name = match user {
            Some(u) => u.to_string(),
            None => self.myself().await?,
        };
        let path = format!("/rest/api/2/issue/{key}/watchers");
        self.send_retry(Method::DELETE, &path, |rb| rb.query(&[("username", name.as_str())])).await.map(|_| ())
    }

    // ------------------------------------------------------------ saved filters

    /// The favourite filters of the current user. Jira Data Center has no endpoint for
    /// "all filters" or "my filters" (`/filter/search` and `/filter/my` give HTTP 404).
    pub async fn favourite_filters(&self) -> Result<Vec<Filter>> {
        let r: Vec<api::Filter> = self.get_json("/rest/api/2/filter/favourite", &[]).await?;
        Ok(r.into_iter().map(|f| Filter::from_api(f, &self.base_url)).collect())
    }

    /// One filter by its ID. Any filter that the user can see works, also when it is not a favourite.
    pub async fn filter(&self, id: &str) -> Result<Filter> {
        let f: api::Filter = self.get_json(&format!("/rest/api/2/filter/{id}"), &[]).await?;
        Ok(Filter::from_api(f, &self.base_url))
    }

    // ------------------------------------------------------------ Boards

    /// One page of the Boards that the user can see. All filters are optional.
    pub async fn boards(&self, q: &BoardQuery, start_at: u64, max: u64) -> Result<Page<Board>> {
        let start = start_at.to_string();
        let max = max.to_string();
        let mut query: Vec<(&str, &str)> = vec![("startAt", start.as_str()), ("maxResults", max.as_str())];
        if let Some(n) = q.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            query.push(("name", n));
        }
        if let Some(p) = q.project.as_deref().filter(|p| !p.is_empty()) {
            query.push(("projectKeyOrId", p));
        }
        if let Some(t) = q.kind.as_deref() {
            query.push(("type", t));
        }
        let r: api::AgilePage<api::Board> = self.get_json("/rest/agile/1.0/board", &query).await?;
        let items: Vec<Board> = r.values.into_iter().map(Board::from).collect();
        let total = r.total.unwrap_or(r.start_at + items.len() as u64 + u64::from(!r.is_last));
        Ok(Page { start_at: r.start_at, total, items })
    }

    /// All Boards that match the query, at most `limit` (all pages when `None`).
    pub async fn boards_all(&self, q: &BoardQuery, limit: Option<u64>) -> Result<Page<Board>> {
        let mut items: Vec<Board> = Vec::new();
        loop {
            let want = limit.map_or(board::LIST_PAGE, |l| (l - items.len() as u64).min(board::LIST_PAGE));
            let page = self.boards(q, items.len() as u64, want).await?;
            let total = page.total;
            let got = page.items.len();
            items.extend(page.items);
            let done = got == 0 || items.len() as u64 >= total || limit.is_some_and(|l| items.len() as u64 >= l);
            if done {
                return Ok(Page { items, start_at: 0, total });
            }
        }
    }

    /// One Board by its ID. HTTP 404 means: the Board does not exist, or the user cannot see it.
    pub async fn board(&self, id: u64) -> Result<Board> {
        let b: api::Board = self.get_json(&format!("/rest/agile/1.0/board/{id}"), &[]).await?;
        Ok(b.into())
    }

    /// The Recent Boards of the user, from a private endpoint. This is "best effort": an
    /// error or an unknown response shape gives an empty list, not an error.
    pub async fn recent_boards(&self) -> Vec<Board> {
        match self.get_json::<api::Menu>("/rest/api/1.0/menus/greenhopper_menu", &[("inAdminMode", "false")]).await {
            Ok(m) => board::recent_from_menu(&m),
            Err(e) => {
                tracing::debug!(target: "jui::http", "no Recent Boards: {e}");
                Vec::new()
            }
        }
    }

    /// The suggestions of the Board picker: the Recent Boards, then the Boards of the project.
    pub async fn board_suggestions(&self, project: &str) -> Result<Vec<board::Suggestion>> {
        let q = BoardQuery { project: Some(project.to_string()), ..Default::default() };
        let (recent, project) = tokio::join!(self.recent_boards(), self.boards_all(&q, Some(200)));
        Ok(board::suggestions(recent, project?.items))
    }

    /// The active Sprints of a Scrum Board. jui gets all pages (a Board has few active Sprints).
    pub async fn active_sprints(&self, board_id: u64) -> Result<Vec<SprintRef>> {
        let path = format!("/rest/agile/1.0/board/{board_id}/sprint");
        let mut out = Vec::new();
        loop {
            let start = out.len().to_string();
            let r: api::AgilePage<api::Sprint> =
                self.get_json(&path, &[("state", "active"), ("startAt", start.as_str()), ("maxResults", "50")]).await?;
            let got = r.values.len();
            out.extend(r.values.into_iter().map(|s| SprintRef { id: s.id, name: s.name, state: s.state }));
            if r.is_last || got == 0 {
                return Ok(out);
            }
        }
    }

    /// A Board with its columns and its Tickets. See ticket 12, part 3: a Scrum Board shows
    /// the Tickets of the active Sprints; a Kanban Board shows its filter, its sub-query, and
    /// the Done Tickets of the last 14 days. jui gets at most `limit` Tickets, in pages of 200.
    /// `only_mine` keeps only the Tickets that are assigned to the current user.
    pub async fn board_view(&self, id: u64, limit: u64, only_mine: bool) -> Result<BoardView> {
        // The Sprint request is slow on this server (2 to 5 s), so it runs at the same time as
        // the configuration request. For a Kanban Board, Jira gives HTTP 400: no Sprints.
        let (c, sprints) = tokio::join!(self.board_config(id), self.active_sprints(id));
        let BoardConfig { board: b, filter_id, sub_query, columns } = c?;
        let sprints = if b.is_scrum() { sprints? } else { Vec::new() };
        let status_ids: Vec<String> = columns.iter().flat_map(|(_, ids)| ids.iter().cloned()).collect();
        let sprint_ids: Vec<u64> = sprints.iter().map(|s| s.id).collect();
        let jql = filter_id.as_deref().and_then(|f| {
            board::board_jql(&board::BoardQueryParts {
                filter_id: f,
                is_scrum: b.is_scrum(),
                sprint_ids: &sprint_ids,
                sub_query: sub_query.as_deref(),
                status_ids: &status_ids,
                only_mine,
            })
        });
        let (mut total, mut rows) = (0, Vec::new());
        if let Some(q) = &jql {
            let first = self.search_page(q, 0, board::BOARD_PAGE.min(limit), api::BOARD_FIELDS).await?;
            total = first.total;
            rows.extend(first.issues);
            // The other pages in parallel. The server can send fewer rows than asked.
            let want = total.min(limit);
            let starts: Vec<u64> = (rows.len() as u64..want).step_by(board::BOARD_PAGE as usize).collect();
            if !rows.is_empty() {
                let pages = futures::future::try_join_all(
                    starts.iter().map(|&s| self.search_page(q, s, board::BOARD_PAGE.min(want - s), api::BOARD_FIELDS)),
                )
                .await?;
                rows.extend(pages.into_iter().flat_map(|p| p.issues));
            }
            rows.truncate(limit as usize);
        }
        let loaded = rows.len() as u64;
        let tickets = rows
            .into_iter()
            .map(|mut i| (i.fields.status.as_mut().and_then(|s| s.id.take()), TicketSummary::from(i)))
            .collect();
        let (columns, unmapped) = board::sort_columns(&columns, tickets);
        Ok(BoardView {
            url: b.url(&self.base_url),
            board: b,
            filter_id,
            sub_query,
            sprints,
            jql,
            columns,
            unmapped,
            total: total.max(loaded),
            loaded,
            only_mine,
        })
    }

    /// The configuration of a Board: the filter, the sub-query, and the columns.
    pub async fn board_config(&self, id: u64) -> Result<BoardConfig> {
        let c: api::BoardConfig = self.get_json(&format!("/rest/agile/1.0/board/{id}/configuration"), &[]).await?;
        Ok(c.into())
    }

    // ------------------------------------------------------------ Sprints and Epics

    /// The Sprints of a Board in some states (for example `active,future`). jui gets all
    /// pages. The list has no `total`, so jui stops at `isLast`. A Kanban Board gives an
    /// input error: it has no Sprints.
    pub async fn sprints(&self, board_id: u64, states: &str) -> Result<Vec<Sprint>> {
        let path = format!("/rest/agile/1.0/board/{board_id}/sprint");
        let mut out: Vec<Sprint> = Vec::new();
        loop {
            let start = out.len().to_string();
            let r = self
                .get_json::<api::AgilePage<api::Sprint>>(&path, &[("state", states), ("startAt", start.as_str()), ("maxResults", "50")])
                .await
                .map_err(|e| no_sprints(e, board_id))?;
            let got = r.values.len();
            out.extend(r.values.into_iter().map(Sprint::from));
            if r.is_last || got == 0 {
                return Ok(out);
            }
        }
    }

    /// One Sprint by its ID. Jira gives HTTP 404 when it does not exist.
    pub async fn sprint(&self, id: u64) -> Result<Sprint> {
        let s: api::Sprint = self.get_json(&format!("/rest/agile/1.0/sprint/{id}"), &[]).await?;
        Ok(s.into())
    }

    /// The Sprint tab data of a Board: the configuration, the active Sprints, and the
    /// backlog JQL. `sprints` are the active Sprints when the caller has them (the Boards
    /// tab has them), because the Sprint request is slow on big Boards (2 to 5 s).
    pub async fn sprint_board(&self, id: u64, sprints: Option<Vec<SprintRef>>) -> Result<SprintBoard> {
        if let Some(s) = sprints {
            return Ok(agile::sprint_board(&self.board_config(id).await?, s));
        }
        // A Kanban Board gives HTTP 400 for the Sprints. jui ignores it.
        let (c, s) = tokio::join!(self.board_config(id), self.active_sprints(id));
        let c = c?;
        let s = if c.board.is_scrum() { s? } else { Vec::new() };
        Ok(agile::sprint_board(&c, s))
    }

    /// Move Tickets to a Sprint. Jira accepts at most 50 Tickets in one request, so jui sends
    /// more requests for more Tickets. A POST is not retried (only HTTP 429).
    pub async fn move_to_sprint(&self, sprint_id: u64, keys: &[String]) -> Result<()> {
        let path = format!("/rest/agile/1.0/sprint/{sprint_id}/issue");
        for part in agile::chunks(keys) {
            let body = serde_json::json!({ "issues": part });
            self.send_retry(Method::POST, &path, |rb| rb.json(&body)).await?;
        }
        Ok(())
    }

    /// Move Tickets to the backlog (out of all open Sprints).
    pub async fn move_to_backlog(&self, keys: &[String]) -> Result<()> {
        let path = "/rest/agile/1.0/backlog/issue";
        for part in agile::chunks(keys) {
            let body = serde_json::json!({ "issues": part });
            self.send_retry(Method::POST, path, |rb| rb.json(&body)).await?;
        }
        Ok(())
    }

    /// One Epic by its key. Jira gives HTTP 404 when the key is not an Epic.
    pub async fn epic(&self, key: &str) -> Result<Epic> {
        let e: api::Epic = self
            .get_json(&format!("/rest/agile/1.0/epic/{key}"), &[])
            .await
            .map_err(|e| match e {
                Error::NotFound(_) => Error::input_hint(
                    format!("{key} is not an Epic, or you cannot see it."),
                    "Use `jui epic list <board>` to find the Epic key.",
                ),
                e => e,
            })?;
        Ok(e.into())
    }

    /// Search Epics with JQL. Returns one page.
    pub async fn search_epics(&self, jql: &str, start_at: u64, max: u64) -> Result<Page<Epic>> {
        let start = start_at.to_string();
        let max = max.to_string();
        let query = [("jql", jql), ("startAt", start.as_str()), ("maxResults", max.as_str()), ("fields", api::EPIC_FIELDS)];
        let r: api::EpicSearch = self.get_json("/rest/api/2/search", &query).await?;
        Ok(Page { start_at: r.start_at, total: r.total, items: r.issues.into_iter().map(Epic::from).collect() })
    }

    /// All Epics that a JQL finds, at most `limit` (all when `None`). jui gets the first page,
    /// then the other pages at the same time (a Board can have more than 2000 open Epics).
    pub async fn search_epics_all(&self, jql: &str, limit: Option<u64>) -> Result<Page<Epic>> {
        const PAGE: u64 = 500;
        let first = self.search_epics(jql, 0, limit.unwrap_or(PAGE).min(PAGE)).await?;
        let total = first.total;
        let want = limit.map_or(total, |l| l.min(total));
        let mut items = first.items;
        if !items.is_empty() && (items.len() as u64) < want {
            let starts: Vec<u64> = (items.len() as u64..want).step_by(PAGE as usize).collect();
            let pages =
                futures::future::try_join_all(starts.iter().map(|&s| self.search_epics(jql, s, PAGE.min(want - s)))).await?;
            items.extend(pages.into_iter().flat_map(|p| p.items));
        }
        items.truncate(want as usize);
        Ok(Page { items, start_at: 0, total })
    }

    /// The Epics of a Board (its filter), open Epics only unless `done`.
    pub async fn board_epics(&self, board_id: u64, done: bool, limit: Option<u64>) -> Result<(Board, Page<Epic>)> {
        let c = self.board_config(board_id).await?;
        let Some(f) = c.filter_id.as_deref() else {
            return Ok((c.board, Page { items: Vec::new(), start_at: 0, total: 0 }));
        };
        let page = self.search_epics_all(&agile::board_epics_jql(f, done), limit).await?;
        Ok((c.board, page))
    }

    /// Set the Epic of Tickets. Jira accepts at most 50 Tickets in one request.
    pub async fn set_epic(&self, epic: &str, keys: &[String]) -> Result<()> {
        let path = format!("/rest/agile/1.0/epic/{epic}/issue");
        for part in agile::chunks(keys) {
            let body = serde_json::json!({ "issues": part });
            self.send_retry(Method::POST, &path, |rb| rb.json(&body)).await?;
        }
        Ok(())
    }

    /// Remove the Epic of Tickets.
    pub async fn unset_epic(&self, keys: &[String]) -> Result<()> {
        self.set_epic("none", keys).await
    }

    // ------------------------------------------------------------ Sub-tasks, Links, Worklogs

    /// The summary and the Sub-tasks of a Ticket, in the order of Jira (the order of creation).
    pub async fn subtasks(&self, key: &str) -> Result<(String, Vec<Related>)> {
        let path = format!("/rest/api/2/issue/{key}");
        let i: api::FullIssue = self.get_json(&path, &[("fields", "summary,subtasks")]).await?;
        Ok((i.fields.summary.unwrap_or_default(), i.fields.subtasks.into_iter().map(Related::from).collect()))
    }

    /// All Link types of the server.
    pub async fn link_types(&self) -> Result<Vec<LinkType>> {
        let r: api::LinkTypes = self.get_json("/rest/api/2/issueLinkType", &[]).await?;
        Ok(r.issue_link_types.into_iter().map(LinkType::from).collect())
    }

    /// The Links of a Ticket, from its side.
    pub async fn links(&self, key: &str) -> Result<TicketLinks> {
        let path = format!("/rest/api/2/issue/{key}");
        let i: api::LinksIssue = self.get_json(&path, &[("fields", "summary,issuelinks")]).await?;
        Ok(TicketLinks { key: i.key, summary: i.fields.summary.unwrap_or_default(), links: links_from_api(i.fields.issuelinks) })
    }

    /// Add a Link. `plan` tells which Ticket is the inward side (see `core::link`).
    /// Jira gives HTTP 201 with no body. A POST is not retried (only HTTP 429).
    pub async fn add_link(&self, plan: &link::LinkPlan) -> Result<()> {
        let body = plan.body();
        self.send_retry(Method::POST, "/rest/api/2/issueLink", |rb| rb.json(&body)).await.map(|_| ())
    }

    /// Delete a Link by its ID. This cannot be reversed.
    pub async fn delete_link(&self, id: &str) -> Result<()> {
        let path = format!("/rest/api/2/issueLink/{id}");
        self.send_retry(Method::DELETE, &path, |rb| rb).await.map(|_| ())
    }

    /// All Worklogs of a Ticket, oldest first. The endpoint gives all Worklogs in one response.
    pub async fn worklogs(&self, key: &str) -> Result<Vec<Worklog>> {
        let page: api::WorklogPage = self.get_json(&format!("/rest/api/2/issue/{key}/worklog"), &[]).await?;
        Ok(page.worklogs.into_iter().map(Worklog::from).collect())
    }

    /// One Worklog.
    pub async fn worklog(&self, key: &str, id: &str) -> Result<Worklog> {
        let w: api::Worklog = self.get_json(&format!("/rest/api/2/issue/{key}/worklog/{id}"), &[]).await?;
        Ok(w.into())
    }

    /// The summary, the time tracking, and the Worklogs of a Ticket, for a Worklog Template.
    pub async fn worklog_context(&self, key: &str) -> Result<WorklogContext> {
        let path = format!("/rest/api/2/issue/{key}");
        let i: api::WorklogIssue = self.get_json(&path, &[("fields", "summary,timetracking,worklog")]).await?;
        let page = i.fields.worklog.unwrap_or_default();
        let tt = i.fields.timetracking.unwrap_or_default();
        let mut ctx = WorklogContext {
            key: i.key,
            summary: i.fields.summary.unwrap_or_default(),
            original_estimate: tt.original_estimate,
            remaining_estimate: tt.remaining_estimate,
            time_spent: tt.time_spent,
            worklogs: page.worklogs.into_iter().map(Worklog::from).collect(),
        };
        // The issue response has at most 20 Worklogs.
        if page.total > ctx.worklogs.len() as u64 {
            ctx.worklogs = self.worklogs(key).await?;
        }
        Ok(ctx)
    }

    /// Add a Worklog. A POST is not retried (only HTTP 429), so a network error cannot make two Worklogs.
    pub async fn add_worklog(&self, key: &str, plan: &worklog::WorklogPlan) -> Result<Worklog> {
        let path = format!("/rest/api/2/issue/{key}/worklog");
        let query = plan.adjust.query();
        let resp = self.send_retry(Method::POST, &path, |rb| rb.query(&query).json(&plan.body)).await?;
        let w: api::Worklog = decode(&path, resp).await?;
        Ok(w.into())
    }

    /// Change a Worklog.
    pub async fn edit_worklog(&self, key: &str, id: &str, plan: &worklog::WorklogPlan) -> Result<Worklog> {
        let path = format!("/rest/api/2/issue/{key}/worklog/{id}");
        let query = plan.adjust.query();
        let resp = self.send_retry(Method::PUT, &path, |rb| rb.query(&query).json(&plan.body)).await?;
        let w: api::Worklog = decode(&path, resp).await?;
        Ok(w.into())
    }

    /// Delete a Worklog. This cannot be reversed.
    pub async fn delete_worklog(&self, key: &str, id: &str, adjust: &worklog::Adjust) -> Result<()> {
        let path = format!("/rest/api/2/issue/{key}/worklog/{id}");
        let query = adjust.delete_query();
        self.send_retry(Method::DELETE, &path, |rb| rb.query(&query)).await.map(|_| ())
    }

    // ------------------------------------------------------------ attachments

    /// The summary and the attachments of a Ticket, in the order of Jira.
    pub async fn attachments(&self, key: &str) -> Result<(String, Vec<Attachment>)> {
        let path = format!("/rest/api/2/issue/{key}");
        let i: api::AttachmentsIssue = self.get_json(&path, &[("fields", "summary,attachment")]).await?;
        Ok((i.fields.summary.unwrap_or_default(), i.fields.attachment.into_iter().map(Attachment::from).collect()))
    }

    /// The attachment settings of the server. When attachments are off, this is an input error.
    pub async fn attachment_meta(&self) -> Result<attachment::Meta> {
        let m: api::AttachmentMeta = self.get_json("/rest/api/2/attachment/meta", &[]).await?;
        if !m.enabled {
            return Err(Error::input("Attachments are off on this Jira server."));
        }
        Ok(attachment::Meta { enabled: m.enabled, limit: m.upload_limit.unwrap_or(attachment::DEFAULT_LIMIT) })
    }

    /// Download an attachment into a file. jui writes a temporary file in the same directory
    /// and renames it at the end, so an error leaves no part of a file. Returns the size.
    /// The token goes only to the base URL (see `attachment::download_path`).
    pub async fn download_to(&self, a: &Attachment, target: &std::path::Path) -> Result<u64> {
        let dir = match target.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => std::path::PathBuf::from("."),
        };
        let tmp = tempfile::Builder::new()
            .prefix(".jui-download-")
            .tempfile_in(&dir)
            .map_err(|e| Error::input(format!("Cannot write in {}: {e}", dir.display())))?;
        let mut file = tokio::fs::File::from_std(tmp.reopen().map_err(|e| Error::input(format!("Cannot write the file: {e}")))?);
        let n = self.download(a, &mut file).await?;
        use tokio::io::AsyncWriteExt;
        file.flush().await.map_err(|e| Error::input(format!("Cannot write the file: {e}")))?;
        drop(file);
        // A temporary file has mode 0600. A download gets the mode of a normal new file.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o644));
        }
        tmp.persist(target).map_err(|e| Error::input(format!("Cannot write {}: {}", target.display(), e.error)))?;
        Ok(n)
    }

    /// Download an attachment into a writer (for example stdout). Returns the size.
    pub async fn download<W: tokio::io::AsyncWrite + Unpin>(&self, a: &Attachment, out: &mut W) -> Result<u64> {
        use tokio::io::AsyncWriteExt;
        let path = attachment::download_path(a, &self.base_url);
        let timeout = self.transfer_timeout();
        let mut resp = self
            .send_retry(Method::GET, &path, |rb| rb.timeout(timeout))
            .await
            .map_err(|e| match e {
                Error::NotFound(_) => Error::NotFound(format!("Attachment {} ({})", a.id, a.filename)),
                e => e,
            })?;
        let mut n = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(|e| self.net_err(e))? {
            out.write_all(&chunk).await.map_err(|e| Error::input(format!("Cannot write the file: {e}")))?;
            n += chunk.len() as u64;
        }
        Ok(n)
    }

    /// Upload one file to a Ticket. Jira needs `X-Atlassian-Token: no-check`. A POST is not
    /// retried (only HTTP 429), so a network error cannot make two attachments.
    pub async fn upload(&self, key: &str, file: &attachment::Upload) -> Result<Vec<Attachment>> {
        let path = format!("/rest/api/2/issue/{key}/attachments");
        let timeout = self.transfer_timeout();
        let resp = self
            .send_retry(Method::POST, &path, |rb| {
                let part = reqwest::multipart::Part::bytes(file.bytes.clone()).file_name(file.name.clone());
                let part = part.mime_str(file.mime).unwrap_or_else(|_| reqwest::multipart::Part::bytes(file.bytes.clone()).file_name(file.name.clone()));
                // The raw UTF-8 name, as browsers send it. Jira does not read `filename*`.
                let form = reqwest::multipart::Form::new().percent_encode_noop().part("file", part);
                rb.header("X-Atlassian-Token", "no-check").timeout(timeout).multipart(form)
            })
            .await?;
        let r: Vec<api::Attachment> = decode(&path, resp).await?;
        Ok(r.into_iter().map(Attachment::from).collect())
    }

    /// Delete an attachment. This cannot be reversed.
    pub async fn delete_attachment(&self, id: &str) -> Result<()> {
        let path = format!("/rest/api/2/attachment/{id}");
        self.send_retry(Method::DELETE, &path, |rb| rb).await.map(|_| ())
    }

    /// The timeout of a download or an upload: at least 5 minutes (50 MB on a slow VPN).
    fn transfer_timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.max(300))
    }

    /// The raw Jira JSON of a Ticket (for `--raw`).
    pub async fn ticket_raw(&self, key: &str) -> Result<serde_json::Value> {
        self.get_json(&format!("/rest/api/2/issue/{key}"), &[("fields", api::VIEW_FIELDS)]).await
    }

    // ------------------------------------------------------------ transport

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http.request(method, self.url(path)).bearer_auth(&self.token).header("Accept", "application/json")
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let resp = self.send_retry(Method::GET, path, |rb| rb.query(query)).await?;
        decode(path, resp).await
    }

    /// Send a request. GET requests are retried on network errors, timeouts, and HTTP 502-504.
    /// All requests are retried on HTTP 429.
    async fn send_retry(
        &self,
        method: Method,
        path: &str,
        build: impl Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<Response> {
        let mut attempt = 0;
        loop {
            let result = self.send_once(method.clone(), path, &build).await;
            let err = match result {
                Ok(r) => return Ok(r),
                Err(e) => e,
            };
            let can_retry = match &err {
                Error::RateLimited { .. } => true,
                e => method == Method::GET && e.is_retryable(),
            };
            let Some(delay) = self.retry_delays.get(attempt).copied().filter(|_| can_retry) else {
                return Err(err);
            };
            let delay = match &err {
                Error::RateLimited { retry_after_secs: Some(s) } => Duration::from_secs(*s).min(MAX_RETRY_AFTER),
                _ => delay,
            };
            tracing::info!(target: "jui::http", "retry {} after {:?}: {}", attempt + 1, delay, err);
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn send_once(
        &self,
        method: Method,
        path: &str,
        build: &impl Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<Response> {
        let mut req = build(self.request(method.clone(), path)).build().map_err(|e| self.net_err(e))?;
        // A download is a file, not JSON.
        if path.starts_with("/secure/attachment/") {
            req.headers_mut().insert(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("*/*"));
        }
        let shown = redact_url(req.url());
        let started = Instant::now();
        let resp = self.http.execute(req).await;
        let ms = started.elapsed().as_millis();
        match resp {
            Ok(r) => {
                tracing::debug!(target: "jui::http", "{method} {shown} -> {} ({ms} ms)", r.status().as_u16());
                if r.status().is_success() {
                    Ok(r)
                } else {
                    Err(status_error(r, path).await)
                }
            }
            Err(e) => {
                tracing::debug!(target: "jui::http", "{method} {shown} -> error ({ms} ms): {e}");
                Err(self.net_err(e))
            }
        }
    }

    fn net_err(&self, e: reqwest::Error) -> Error {
        if e.is_timeout() {
            return Error::Timeout(self.timeout_secs);
        }
        let host = url::Url::parse(&self.base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| self.base_url.clone());
        let mut cause = e.to_string();
        let mut src = std::error::Error::source(&e);
        while let Some(s) = src {
            cause = s.to_string();
            src = s.source();
        }
        Error::Network { host, cause }
    }
}

/// The filters of a Board search. All are optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardQuery {
    /// A part of the Board name.
    pub name: Option<String>,
    /// A project key.
    pub project: Option<String>,
    /// `scrum` or `kanban`.
    pub kind: Option<String>,
}

/// Where the assignee must be valid: an existing Ticket, or a project (for a new Ticket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignScope {
    Ticket(String),
    Project(String),
}

impl AssignScope {
    /// "assignable user for BCE-1", for the "no match" error.
    pub fn what(&self) -> String {
        match self {
            AssignScope::Ticket(k) => format!("assignable user for {k}"),
            AssignScope::Project(p) => format!("assignable user in project {p}"),
        }
    }
}

/// Find a Ticket type by name: exact, then a unique prefix (ignores case).
/// `subtask` or `sub-task` selects the first Sub-task type. The error is the hint.
pub fn pick_type<'a>(types: &'a [IssueType], name: &str) -> std::result::Result<&'a IssueType, String> {
    let n = name.trim().to_lowercase();
    let names = || format!("Use one of: {}.", types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "));
    if let Some(t) = types.iter().find(|t| t.name.to_lowercase() == n || t.id == n) {
        return Ok(t);
    }
    if matches!(n.as_str(), "subtask" | "sub-task" | "sub task")
        && let Some(t) = types.iter().find(|t| t.subtask)
    {
        return Ok(t);
    }
    let found: Vec<&IssueType> = types.iter().filter(|t| t.name.to_lowercase().starts_with(&n)).collect();
    match found.as_slice() {
        [one] => Ok(one),
        _ => Err(names()),
    }
}

/// A Kanban Board gives HTTP 400 "The board doesn't support sprints." for the Sprint list.
fn no_sprints(e: Error, board_id: u64) -> Error {
    match e {
        Error::Validation { ref messages, .. } if messages.iter().any(|m| m.contains("support sprints")) => Error::input_hint(
            format!("Board {board_id} is a Kanban Board. It has no Sprints."),
            "Use a Scrum Board. `jui board list --type scrum` shows them.",
        ),
        e => e,
    }
}

/// The path and query of a URL for logs. The URL never contains the token.
fn redact_url(u: &url::Url) -> String {
    match u.query() {
        Some(q) => format!("{}?{}", u.path(), q),
        None => u.path().to_string(),
    }
}

async fn decode<T: DeserializeOwned>(path: &str, resp: Response) -> Result<T> {
    let bytes = resp.bytes().await.map_err(|e| Error::Decode { endpoint: path.into(), message: e.to_string() })?;
    tracing::trace!(target: "jui::http", "body: {}", truncate(&String::from_utf8_lossy(&bytes), 2000));
    serde_json::from_slice(&bytes).map_err(|e| Error::Decode { endpoint: path.into(), message: e.to_string() })
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// Make an `Error` from a response that is not a success.
async fn status_error(resp: Response, path: &str) -> Error {
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let text = resp.text().await.unwrap_or_default();
    let body: api::ErrorBody = serde_json::from_str(&text).unwrap_or_default();
    let joined = || {
        let mut all = body.error_messages.clone();
        all.extend(body.errors.values().cloned());
        if all.is_empty() {
            status.canonical_reason().unwrap_or("").to_string()
        } else {
            all.join(" ")
        }
    };
    match status {
        StatusCode::UNAUTHORIZED => Error::Unauthorized,
        StatusCode::FORBIDDEN => Error::Forbidden(joined()),
        StatusCode::NOT_FOUND => Error::NotFound(describe(path)),
        StatusCode::BAD_REQUEST => Error::Validation { fields: body.errors, messages: body.error_messages },
        StatusCode::CONFLICT => Error::Conflict(joined()),
        StatusCode::TOO_MANY_REQUESTS => Error::RateLimited { retry_after_secs: retry_after },
        s => {
            // Do not show HTML error pages.
            let shown = if text.trim_start().starts_with('<') || text.trim().is_empty() {
                s.canonical_reason().unwrap_or("").to_string()
            } else if body.error_messages.is_empty() && body.errors.is_empty() {
                truncate(text.trim(), 200)
            } else {
                joined()
            };
            Error::Server { status: s.as_u16(), body: shown }
        }
    }
}

/// A short description of what a path asks for, for `NotFound` messages.
fn describe(path: &str) -> String {
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match parts.as_slice() {
        ["rest", "api", "2", "issue", key, "comment", id] => format!("Comment {id} on {key}"),
        ["rest", "api", "2", "issue", key, "worklog", id] => format!("Worklog {id} on {key}"),
        ["rest", "api", "2", "issueLink", id] => format!("Link {id}"),
        ["rest", "api", "2", "attachment", id] => format!("Attachment {id}"),
        ["rest", "api", "2", "issue", key, ..] => format!("Ticket {key}"),
        ["rest", "agile", "1.0", "board", id, ..] => format!("Board {id}"),
        ["rest", "agile", "1.0", "sprint", id, ..] => format!("Sprint {id}"),
        ["rest", "agile", "1.0", "epic", key, ..] => format!("Epic {key}"),
        ["rest", "api", "2", "filter", id, ..] => format!("Filter {id}"),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_paths() {
        assert_eq!(describe("/rest/api/2/issue/BCE-1"), "Ticket BCE-1");
        assert_eq!(describe("/rest/api/2/issue/BCE-1/comment/7"), "Comment 7 on BCE-1");
        assert_eq!(describe("/rest/api/2/issue/BCE-1/comment"), "Ticket BCE-1");
        assert_eq!(describe("/rest/agile/1.0/board/4166/configuration"), "Board 4166");
        assert_eq!(describe("/rest/api/2/search"), "/rest/api/2/search");
    }

    #[test]
    fn debug_hides_token() {
        let s = Settings::default();
        let j = Jira::new(&s, Some("secret-token".into())).unwrap();
        assert!(!format!("{j:?}").contains("secret"));
    }

    #[test]
    fn no_token_is_an_error() {
        let s = Settings::default();
        assert_eq!(Jira::new(&s, None).unwrap_err().kind(), "no_token");
        assert_eq!(Jira::new(&s, Some("  ".into())).unwrap_err().kind(), "no_token");
    }
}
