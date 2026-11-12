//! API layer: structs that have the same shape as the Jira JSON.
//! Only the conversion functions in `model` use these types.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    #[serde(default)]
    pub start_at: u64,
    #[serde(default)]
    pub max_results: u64,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub issues: Vec<Issue>,
}

#[derive(Debug, Deserialize)]
pub struct Issue {
    pub id: String,
    pub key: String,
    #[serde(default)]
    pub fields: IssueFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct IssueFields {
    pub summary: Option<String>,
    pub issuetype: Option<Named>,
    pub status: Option<Status>,
    pub priority: Option<Named>,
    pub assignee: Option<User>,
    pub updated: Option<String>,
    /// Only when the request asks for it (`PREVIEW_FIELDS`).
    pub description: Option<String>,
    /// The Sprint field: legacy Java strings (`...Sprint@1a2b[id=1,name=S,state=ACTIVE,...]`).
    /// Some servers send objects. `model::sprint_from_field` reads both.
    #[serde(rename = "customfield_10020", default)]
    pub sprint: Option<serde_json::Value>,
    /// Epic Link (`customfield_10016`): the Epic key. Only when the request asks for it
    /// (`MY_EPIC_FIELDS`).
    #[serde(rename = "customfield_10016", default)]
    pub epic_link: Option<String>,
}

/// The fields of the search for the Epics tab: only the Epic Link.
pub const MY_EPIC_FIELDS: &str = "customfield_10016";

#[derive(Debug, Deserialize)]
pub struct Named {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The status ID. Board columns refer to statuses by ID.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub status_category: Option<StatusCategory>,
}

#[derive(Debug, Deserialize)]
pub struct StatusCategory {
    pub key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub name: String,
    pub display_name: Option<String>,
}

// ---------------------------------------------------------------- full issue

#[derive(Debug, Deserialize)]
pub struct FullIssue {
    pub id: String,
    pub key: String,
    #[serde(default)]
    pub fields: FullFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct FullFields {
    pub summary: Option<String>,
    pub issuetype: Option<Named>,
    pub status: Option<Status>,
    pub priority: Option<Named>,
    pub assignee: Option<User>,
    pub reporter: Option<User>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub components: Vec<Named>,
    #[serde(rename = "fixVersions", default)]
    pub fix_versions: Vec<Named>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub description: Option<String>,
    pub parent: Option<LinkedIssue>,
    #[serde(default)]
    pub subtasks: Vec<LinkedIssue>,
    #[serde(default)]
    pub issuelinks: Vec<IssueLink>,
    #[serde(default)]
    pub attachment: Vec<Attachment>,
    pub comment: Option<CommentPage>,
    pub worklog: Option<WorklogPage>,
    /// Epic Link (`customfield_10016`): the Epic key.
    #[serde(rename = "customfield_10016")]
    pub epic_link: Option<String>,
    /// Story Points (`customfield_10021`).
    #[serde(rename = "customfield_10021")]
    pub story_points: Option<f64>,
    pub watches: Option<Watches>,
}

#[derive(Debug, Deserialize)]
pub struct LinkedIssue {
    pub key: String,
    #[serde(default)]
    pub fields: LinkedFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct LinkedFields {
    pub summary: Option<String>,
    pub status: Option<Status>,
    pub issuetype: Option<Named>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueLink {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: LinkType,
    pub inward_issue: Option<LinkedIssue>,
    pub outward_issue: Option<LinkedIssue>,
}

#[derive(Debug, Deserialize)]
pub struct LinkType {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub inward: String,
    pub outward: String,
}

/// `GET /rest/api/2/issueLinkType`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkTypes {
    #[serde(default)]
    pub issue_link_types: Vec<LinkType>,
}

/// The `fields.issuelinks` of an issue, and the summary (for `jui link list`).
#[derive(Debug, Deserialize)]
pub struct LinksIssue {
    pub key: String,
    #[serde(default)]
    pub fields: LinksFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct LinksFields {
    pub summary: Option<String>,
    #[serde(default)]
    pub issuelinks: Vec<IssueLink>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    pub author: Option<User>,
    pub created: Option<String>,
    #[serde(default)]
    pub size: u64,
    pub mime_type: Option<String>,
    pub content: Option<String>,
}

/// `GET /rest/api/2/attachment/meta`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub upload_limit: Option<u64>,
}

/// The summary and the attachments of an issue (for `jui attachment list`).
#[derive(Debug, Deserialize)]
pub struct AttachmentsIssue {
    pub key: String,
    #[serde(default)]
    pub fields: AttachmentsFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct AttachmentsFields {
    pub summary: Option<String>,
    #[serde(default)]
    pub attachment: Vec<Attachment>,
}

/// The `comment` field of an issue, and `GET /rest/api/2/issue/{key}/comment`.
#[derive(Debug, Default, Deserialize)]
pub struct CommentPage {
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub total: u64,
    #[serde(rename = "startAt", default)]
    pub start_at: u64,
}

#[derive(Debug, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: Option<User>,
    #[serde(default)]
    pub body: String,
    pub created: Option<String>,
    pub updated: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorklogPage {
    #[serde(default)]
    pub worklogs: Vec<Worklog>,
    #[serde(default)]
    pub total: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worklog {
    pub id: String,
    pub author: Option<User>,
    pub started: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub time_spent: Option<String>,
    #[serde(default)]
    pub time_spent_seconds: u64,
    pub comment: Option<String>,
}

/// `GET /rest/api/2/issue/{key}?fields=summary,timetracking,worklog`: the data of a Worklog Template.
#[derive(Debug, Deserialize)]
pub struct WorklogIssue {
    pub key: String,
    #[serde(default)]
    pub fields: WorklogFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct WorklogFields {
    pub summary: Option<String>,
    pub timetracking: Option<TimeTracking>,
    pub worklog: Option<WorklogPage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeTracking {
    pub original_estimate: Option<String>,
    pub remaining_estimate: Option<String>,
    pub time_spent: Option<String>,
}

/// `GET /rest/agile/1.0/issue/{key}?fields=sprint,closedSprints,epic`
#[derive(Debug, Default, Deserialize)]
pub struct AgileIssue {
    #[serde(default)]
    pub fields: AgileFields,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgileFields {
    pub sprint: Option<Sprint>,
    #[serde(default)]
    pub closed_sprints: Vec<Sprint>,
    pub epic: Option<Epic>,
}

/// One Sprint: `GET /rest/agile/1.0/sprint/{id}`, the Sprint list of a Board, and the Agile issue.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sprint {
    pub id: u64,
    pub name: String,
    pub state: String,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub complete_date: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub origin_board_id: Option<u64>,
}

/// One Epic: `GET /rest/agile/1.0/epic/{key}` and the `epic` field of the Agile issue.
#[derive(Debug, Deserialize)]
pub struct Epic {
    pub key: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub done: bool,
}

/// The fields that `view` needs.
pub const VIEW_FIELDS: &str = "summary,issuetype,status,priority,assignee,reporter,labels,components,fixVersions,\
created,updated,description,parent,subtasks,issuelinks,attachment,comment,worklog,customfield_10016,customfield_10021,watches";

/// A saved filter: `GET /rest/api/2/filter/{id}` and `GET /rest/api/2/filter/favourite`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub owner: Option<User>,
    #[serde(default)]
    pub jql: String,
    #[serde(default)]
    pub view_url: Option<String>,
    #[serde(default)]
    pub favourite: bool,
}

// ---------------------------------------------------------------- Boards

/// `GET /rest/agile/1.0/board` (a page of Boards) and the Sprint list of a Board.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgilePage<T> {
    #[serde(default)]
    pub start_at: u64,
    #[serde(default)]
    pub max_results: u64,
    /// The Sprint list has no `total`.
    pub total: Option<u64>,
    #[serde(default)]
    pub is_last: bool,
    #[serde(default = "Vec::new")]
    pub values: Vec<T>,
}

/// One Board: `GET /rest/agile/1.0/board/{id}` and the values of the Board list.
#[derive(Debug, Deserialize)]
pub struct Board {
    pub id: u64,
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
}

/// `GET /rest/agile/1.0/board/{id}/configuration`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardConfig {
    pub id: u64,
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    pub filter: Option<IdRef>,
    pub sub_query: Option<SubQuery>,
    #[serde(default)]
    pub column_config: ColumnConfig,
}

#[derive(Debug, Deserialize)]
pub struct IdRef {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct SubQuery {
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct ColumnConfig {
    #[serde(default)]
    pub columns: Vec<Column>,
}

#[derive(Debug, Deserialize)]
pub struct Column {
    pub name: String,
    #[serde(default)]
    pub statuses: Vec<IdRef>,
}

/// `GET /rest/api/1.0/menus/greenhopper_menu` (a private endpoint). jui reads only the
/// "Recent Boards" section.
#[derive(Debug, Default, Deserialize)]
pub struct Menu {
    #[serde(default)]
    pub sections: Vec<MenuSection>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MenuSection {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub items: Vec<MenuItem>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MenuItem {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
}

/// The body of a Jira error response.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    #[serde(default)]
    pub error_messages: Vec<String>,
    #[serde(default)]
    pub errors: std::collections::BTreeMap<String, String>,
}

/// The fields that list views need. Send with `fields=` to keep the response small.
/// `customfield_10020` is the Sprint field: My Tickets groups the rows by Sprint.
pub const LIST_FIELDS: &str = "summary,issuetype,status,priority,assignee,updated,customfield_10020";

/// The list fields and the description. The TUI shows the description in the preview
/// before it has the full Ticket.
pub const PREVIEW_FIELDS: &str = "summary,issuetype,status,priority,assignee,updated,customfield_10020,description";

/// The fields of the Board cards. The status ID puts each card in its column.
pub const BOARD_FIELDS: &str = "summary,issuetype,status,priority,assignee,updated";

/// The fields of an Epic in a search: the summary, the status, the Epic Status
/// (`customfield_10017`), and the Epic Name (`customfield_10018`).
pub const EPIC_FIELDS: &str = "summary,status,customfield_10017,customfield_10018";

/// `GET /rest/api/2/search` with `EPIC_FIELDS`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpicSearch {
    #[serde(default)]
    pub start_at: u64,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub issues: Vec<EpicIssue>,
}

#[derive(Debug, Deserialize)]
pub struct EpicIssue {
    pub key: String,
    #[serde(default)]
    pub fields: EpicFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct EpicFields {
    pub summary: Option<String>,
    /// The workflow status. Some Epics are Done here but not in the Epic Status.
    #[serde(default)]
    pub status: Option<Status>,
    /// Epic Status: a select list with the values "To Do", "In Progress", and "Done".
    #[serde(rename = "customfield_10017")]
    pub epic_status: Option<OptionValue>,
    /// Epic Name.
    #[serde(rename = "customfield_10018")]
    pub epic_name: Option<String>,
}

/// The value of a select list field.
#[derive(Debug, Default, Deserialize)]
pub struct OptionValue {
    pub value: Option<String>,
}

// ---------------------------------------------------------------- Transitions

/// `GET /rest/api/2/issue/{key}/transitions?expand=transitions.fields`
#[derive(Debug, Default, Deserialize)]
pub struct TransitionsResponse {
    #[serde(default)]
    pub transitions: Vec<ApiTransition>,
}

#[derive(Debug, Deserialize)]
pub struct ApiTransition {
    pub id: String,
    pub name: String,
    pub to: Status,
    #[serde(default)]
    pub fields: std::collections::BTreeMap<String, FieldMeta>,
}

/// The metadata of one field on a screen (Transition, createmeta, editmeta).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMeta {
    /// Only in createmeta (a list, not a map).
    pub field_id: Option<String>,
    #[serde(default)]
    pub required: bool,
    pub name: String,
    #[serde(default)]
    pub schema: FieldSchema,
    #[serde(default)]
    pub has_default_value: bool,
    #[serde(default)]
    pub allowed_values: Vec<AllowedValue>,
}

#[derive(Debug, Default, Deserialize)]
pub struct FieldSchema {
    #[serde(rename = "type", default)]
    pub kind: String,
    pub items: Option<String>,
    pub custom: Option<String>,
}

/// A permitted value. Versions, components, and resolutions have `name`. Select lists have `value`.
#[derive(Debug, Deserialize)]
pub struct AllowedValue {
    pub id: String,
    pub name: Option<String>,
    pub value: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub disabled: bool,
}

// ---------------------------------------------------------------- create and edit

/// A page of createmeta values: `GET /rest/api/2/issue/createmeta/{project}/issuetypes[/{id}]`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPage<T> {
    #[serde(default)]
    pub start_at: u64,
    #[serde(default)]
    pub total: u64,
    pub is_last: Option<bool>,
    #[serde(default = "Vec::new")]
    pub values: Vec<T>,
}

#[derive(Debug, Deserialize)]
pub struct IssueTypeMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub subtask: bool,
}

/// `GET /rest/api/2/issue/{key}?expand=editmeta&fields=*navigable`
#[derive(Debug, Deserialize)]
pub struct EditIssue {
    pub key: String,
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub editmeta: EditMeta,
}

#[derive(Debug, Default, Deserialize)]
pub struct EditMeta {
    #[serde(default)]
    pub fields: std::collections::BTreeMap<String, FieldMeta>,
}

/// The response of `POST /rest/api/2/issue`.
#[derive(Debug, Deserialize)]
pub struct Created {
    pub key: String,
}

/// `GET /rest/api/2/issue/{key}?fields=updated`
#[derive(Debug, Deserialize)]
pub struct UpdatedOnly {
    #[serde(default)]
    pub fields: UpdatedFields,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdatedFields {
    pub updated: Option<String>,
}

// ---------------------------------------------------------------- users and watchers

/// One user from `GET /rest/api/2/user/assignable/search` or `/user/search`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEntry {
    pub name: String,
    pub display_name: Option<String>,
    pub email_address: Option<String>,
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

/// The `watches` field of an issue.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watches {
    #[serde(default)]
    pub watch_count: u64,
    #[serde(default)]
    pub is_watching: bool,
}

/// `GET /rest/api/2/issue/{key}/watchers`
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchersResponse {
    #[serde(default)]
    pub is_watching: bool,
    #[serde(default)]
    pub watch_count: u64,
    #[serde(default)]
    pub watchers: Vec<UserEntry>,
}
