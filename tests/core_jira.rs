//! Core tests against a mock HTTP server. They never use the real Jira.

use std::time::Duration;

use jui::core::model::{self, StatusCategory};
use jui::core::{Jira, Settings};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PAGE1: &str = include_str!("fixtures/search_page1.json");
const PAGE2: &str = include_str!("fixtures/search_page2.json");

fn client(server: &MockServer) -> Jira {
    let s = Settings { base_url: server.uri(), timeout_secs: 2, ..Settings::default() };
    Jira::new(&s, Some("test-token".into())).unwrap().with_retry_delays(vec![Duration::ZERO; 3])
}

fn json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

#[tokio::test]
async fn search_sends_bearer_and_maps_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .and(header("Authorization", "Bearer test-token"))
        .and(query_param("jql", "project = BCE"))
        .and(query_param("startAt", "0"))
        .and(query_param("fields", "summary,issuetype,status,priority,assignee,updated,customfield_10020"))
        .respond_with(json(PAGE1))
        .expect(1)
        .mount(&server)
        .await;

    let page = client(&server).search("project = BCE", 0, 2).await.unwrap();
    // The Sprint field: null gives no Sprint; closed and active give the active Sprint.
    assert_eq!(page.items[0].sprint, None);
    let s = page.items[1].sprint.as_ref().unwrap();
    assert_eq!((s.id, s.name.as_str(), s.state.as_str()), (43198, "BCE-26-Sprint-21", "active"));
    assert_eq!(page.total, 3);
    assert!(page.has_more());
    let t = &page.items[0];
    assert_eq!(t.key, "BCE-62886");
    assert_eq!(t.kind, "Task");
    assert_eq!(t.status, "In Progress");
    assert_eq!(t.status_category, StatusCategory::InProgress);
    assert_eq!(t.assignee.as_ref().unwrap().name, "jdoe");
    assert_eq!(t.updated.unwrap().to_rfc3339(), "2026-10-19T00:30:46-07:00");
}

#[tokio::test]
async fn search_all_follows_pages() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search")).and(query_param("startAt", "0")).respond_with(json(PAGE1)).mount(&server).await;
    Mock::given(path("/rest/api/2/search")).and(query_param("startAt", "2")).respond_with(json(PAGE2)).mount(&server).await;

    let page = client(&server).search_all("x", None).await.unwrap();
    assert_eq!(page.items.len(), 3);
    assert!(!page.has_more());
    assert_eq!(page.items[2].key, "XSUP-1182");
    assert!(page.items[2].assignee.is_none());
}

#[tokio::test]
async fn search_all_respects_limit() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("startAt", "0"))
        .and(query_param("maxResults", "1"))
        .respond_with(json(PAGE1))
        .expect(1)
        .mount(&server)
        .await;
    let page = client(&server).search_all("x", Some(1)).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.has_more());
}

#[tokio::test]
async fn http_errors_map_to_kinds() {
    let server = MockServer::start().await;
    let cases: &[(u16, &str, &str)] = &[
        (401, "", "unauthorized"),
        (403, r#"{"errorMessages":["No access."]}"#, "forbidden"),
        (400, r#"{"errorMessages":["Error in the JQL Query."],"errors":{}}"#, "validation"),
        (500, "<html>boom</html>", "server"),
    ];
    for (status, body, kind) in cases {
        server.reset().await;
        Mock::given(path("/rest/api/2/search"))
            .respond_with(ResponseTemplate::new(*status).set_body_raw(*body, "application/json"))
            .mount(&server)
            .await;
        let e = client(&server).search("x", 0, 1).await.unwrap_err();
        assert_eq!(e.kind(), *kind, "status {status}");
        if *status == 500 {
            assert!(!e.to_string().contains("<html>"), "HTML must not be shown: {e}");
        }
    }
}

#[tokio::test]
async fn get_retries_on_503_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/search")).respond_with(json(PAGE1)).mount(&server).await;
    let page = client(&server).search("x", 0, 2).await.unwrap();
    assert_eq!(page.items.len(), 2);
}

#[tokio::test]
async fn get_gives_up_after_three_retries() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .respond_with(ResponseTemplate::new(503))
        .expect(4)
        .mount(&server)
        .await;
    let e = client(&server).search("x", 0, 2).await.unwrap_err();
    assert_eq!(e.kind(), "server");
}

#[tokio::test]
async fn no_retry_on_500() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(client(&server).search("x", 0, 2).await.unwrap_err().kind(), "server");
}

#[tokio::test]
async fn timeout_is_reported() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .respond_with(json(PAGE1).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;
    let s = Settings { base_url: server.uri(), timeout_secs: 1, ..Settings::default() };
    let j = Jira::new(&s, Some("t".into())).unwrap().with_retry_delays(vec![]);
    assert_eq!(j.search("x", 0, 1).await.unwrap_err().kind(), "timeout");
}

#[tokio::test]
async fn bad_json_is_decode_error() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search")).respond_with(json(r#"{"issues": 5}"#)).mount(&server).await;
    assert_eq!(client(&server).search("x", 0, 1).await.unwrap_err().kind(), "decode");
}

const ISSUE: &str = include_str!("fixtures/issue_full.json");
const AGILE: &str = include_str!("fixtures/agile_issue.json");

#[tokio::test]
async fn ticket_maps_all_view_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/rest/api/2/issue/BCE-62886")).respond_with(json(ISSUE)).mount(&server).await;
    Mock::given(method("GET")).and(path("/rest/agile/1.0/issue/BCE-62886")).respond_with(json(AGILE)).mount(&server).await;

    let t = client(&server).ticket("BCE-62886").await.unwrap();
    assert_eq!(t.url, format!("{}/browse/BCE-62886", server.uri()));
    assert_eq!(t.reporter.as_ref().unwrap().name, "rroe");
    assert_eq!(t.components, vec!["Scanner"]);
    assert_eq!(t.story_points, Some(3.0));
    assert_eq!(t.epic.as_ref().unwrap().name.as_deref(), Some("Scanner reliability"));
    assert_eq!(t.sprint.as_ref().unwrap().name, "BCE-26-Sprint-21");
    assert_eq!(t.subtasks[0].key, "BCE-62890");
    assert_eq!(t.links[0].relation, "blocks");
    assert_eq!(t.links[1].relation, "relates to");
    assert_eq!(t.links[1].ticket.key, "BCE-61200");
    assert_eq!(t.comments.len(), 2);
    assert_eq!(t.worklogs[0].time_spent_seconds, 5400);
    let d = t.description.unwrap();
    assert!(d.wiki.starts_with("h3. Goal"));
    assert!(d.markdown.starts_with("### Goal"));
    assert!(d.markdown.contains("@rroe"));
    assert!(d.markdown.contains("`jira:{color:#de350b}Important{color}`"));
}

#[tokio::test]
async fn ticket_without_agile_data_still_works() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/issue/BCE-62886")).respond_with(json(ISSUE)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/issue/BCE-62886")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let t = client(&server).ticket("BCE-62886").await.unwrap();
    assert!(t.sprint.is_none());
    assert_eq!(t.epic.as_ref().unwrap().key, "BCE-47921", "Epic Link field is the fallback");
}

#[tokio::test]
async fn ticket_not_found() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/issue/BCE-1"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(r#"{"errorMessages":["Issue Does Not Exist"],"errors":{}}"#, "application/json"))
        .mount(&server)
        .await;
    let e = client(&server).ticket("BCE-1").await.unwrap_err();
    assert_eq!(e.kind(), "not_found");
    assert_eq!(e.to_string(), "Ticket BCE-1 not found.");
}

#[tokio::test]
async fn ticket_gets_all_worklogs_when_more_than_embedded() {
    let server = MockServer::start().await;
    let issue = ISSUE.replace(r#""maxResults": 20, "total": 1"#, r#""maxResults": 20, "total": 2"#);
    Mock::given(path("/rest/api/2/issue/BCE-62886")).respond_with(json(&issue)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/issue/BCE-62886")).respond_with(json(AGILE)).mount(&server).await;
    Mock::given(path("/rest/api/2/issue/BCE-62886/worklog"))
        .respond_with(json(r#"{"startAt":0,"maxResults":2,"total":2,"worklogs":[
            {"id":"1","timeSpent":"1h","timeSpentSeconds":3600},
            {"id":"2","timeSpent":"2h","timeSpentSeconds":7200}]}"#))
        .expect(1)
        .mount(&server)
        .await;
    let t = client(&server).ticket("BCE-62886").await.unwrap();
    assert_eq!(t.worklogs.len(), 2);
}

#[tokio::test]
async fn search_rows_asks_for_the_description() {
    let server = MockServer::start().await;
    let body = PAGE1.replace(r#""summary": "Add retry to the upload client","#, r#""summary": "Add retry to the upload client", "description": "h3. Goal","#);
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("fields", "summary,issuetype,status,priority,assignee,updated,customfield_10020,description"))
        .and(query_param("maxResults", "100"))
        .respond_with(json(&body))
        .expect(1)
        .mount(&server)
        .await;
    let p = client(&server).search_rows("x", 0, 100).await.unwrap();
    assert_eq!(p.total, 3);
    assert_eq!(p.items[0].ticket.key, "BCE-62886");
    assert_eq!(p.items[0].description.as_deref(), Some("h3. Goal"));
    assert_eq!(p.items[1].description, None);
}

const TR_TASK: &str = include_str!("fixtures/transitions_task.json");
const TR_BUG: &str = include_str!("fixtures/transitions_bug.json");

#[tokio::test]
async fn transitions_map_screen_fields_and_differ_by_type() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/BCE-1/transitions"))
        .and(query_param("expand", "transitions.fields"))
        .respond_with(json(TR_TASK))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/rest/api/2/issue/BCE-2/transitions")).respond_with(json(TR_BUG)).mount(&server).await;
    let j = client(&server);
    let task = j.transitions("BCE-1").await.unwrap();
    let bug = j.transitions("BCE-2").await.unwrap();

    let done = task.iter().find(|t| t.to == "Done").unwrap();
    assert_eq!(done.id, "31");
    assert_eq!(done.fields[0].id, "resolution", "required fields come first");
    assert!(done.fields[0].required);
    assert_eq!(done.fields[0].kind, jui::core::model::FieldKind::Choice);
    assert_eq!(done.fields[0].allowed.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Won't Do", "Duplicate", "Done"]);
    let points = done.fields.iter().find(|f| f.name == "Story Points").unwrap();
    assert_eq!(points.kind, jui::core::model::FieldKind::Number);

    // Transition 161 goes to a different Status for a Task and a Bug.
    assert_eq!(task.iter().find(|t| t.id == "161").unwrap().to, "In Product Design");
    assert_eq!(bug.iter().find(|t| t.id == "161").unwrap().to, "New");
}

#[tokio::test]
async fn transition_posts_fields_and_comment() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/rest/api/2/issue/BCE-1/transitions")).respond_with(json(TR_TASK)).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/BCE-1/transitions"))
        .and(wiremock::matchers::body_json(serde_json::json!({
            "transition": {"id": "31"},
            "fields": {"resolution": {"id": "10401"}},
            "update": {"comment": [{"add": {"body": "*Fixed* in [~rroe] PR"}}]}
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    let ts = j.transitions("BCE-1").await.unwrap();
    let done = ts.iter().find(|t| t.to == "Done").unwrap();
    let mut fields = serde_json::Map::new();
    fields.insert("resolution".into(), serde_json::json!({"id": "10401"}));
    j.transition("BCE-1", done, &fields, Some("**Fixed** in @rroe PR")).await.unwrap();
}

#[tokio::test]
async fn transition_400_names_the_fields_and_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/rest/api/2/issue/BCE-1/transitions")).respond_with(json(TR_TASK)).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/BCE-1/transitions"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"errorMessages":[],"errors":{"resolution":"Resolution is required."}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/BCE-9/transitions"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    let ts = j.transitions("BCE-1").await.unwrap();
    let done = ts.iter().find(|t| t.to == "Done").unwrap();
    let e = j.transition("BCE-1", done, &serde_json::Map::new(), None).await.unwrap_err();
    let jui::core::Error::Validation { fields, .. } = e else { panic!("not a validation error") };
    assert_eq!(fields.get("Resolution").map(String::as_str), Some("Resolution is required."));
    // A POST is not retried on HTTP 503 (expect(1) checks this).
    let e = j.transition("BCE-9", done, &serde_json::Map::new(), None).await.unwrap_err();
    assert_eq!(e.kind(), "server");
}

// ------------------------------------------------------------------ create and edit

#[tokio::test]
async fn create_meta_matches_the_type_and_follows_pages() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/issue/createmeta/BCE/issuetypes"))
        .respond_with(json(include_str!("fixtures/createmeta_bce_types.json")))
        .mount(&server)
        .await;
    // Two pages: the first has `isLast: false`.
    let page1 = r#"{"startAt":0,"maxResults":1,"total":2,"isLast":false,"values":[
        {"fieldId":"summary","name":"Summary","required":true,"schema":{"type":"string","system":"summary"}}]}"#;
    let page2 = r#"{"startAt":1,"maxResults":1,"total":2,"isLast":true,"values":[
        {"fieldId":"customfield_11908","name":"Severity","required":true,"schema":{"type":"option","custom":"x:select"},
         "allowedValues":[{"id":"1","value":"Major"},{"id":"2","value":"Old","disabled":true}]}]}"#;
    Mock::given(path("/rest/api/2/issue/createmeta/BCE/issuetypes/10102"))
        .and(query_param("startAt", "0"))
        .respond_with(json(page1))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/issue/createmeta/BCE/issuetypes/10102"))
        .and(query_param("startAt", "1"))
        .respond_with(json(page2))
        .mount(&server)
        .await;
    let m = client(&server).create_meta("bce", "bu").await.unwrap();
    assert_eq!(m.issue_type.name, "Bug");
    assert_eq!(m.fields.len(), 2);
    assert_eq!(m.fields[1].allowed.len(), 1, "disabled values are removed");

    let e = client(&server).create_meta("BCE", "S").await.unwrap_err();
    assert_eq!(e.kind(), "input", "`S` matches Story and Sub-task");
    let sub = client(&server).issue_types("BCE").await.unwrap();
    assert!(jui::core::jira::pick_type(&sub, "subtask").unwrap().subtask);
}

#[tokio::test]
async fn edit_data_reads_editmeta_and_put_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/BCE-63337"))
        .and(query_param("expand", "editmeta"))
        .and(query_param("fields", "*navigable"))
        .respond_with(json(include_str!("fixtures/edit_bce_bug.json")))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/BCE-63337"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    let d = j.edit_data("BCE-63337").await.unwrap();
    assert_eq!(d.project, "BCE");
    assert_eq!(d.issue_type, "Bug");
    assert_eq!(d.updated.as_deref(), Some("2026-10-19T10:00:00.000-0700"));
    assert!(d.description.unwrap().starts_with("h3. Steps"));
    assert!(d.fields.iter().any(|f| f.id == "customfield_11908" && f.required));
    let e = j.edit("BCE-63337", &serde_json::json!({"fields": {}})).await.unwrap_err();
    assert_eq!(e.kind(), "server");
}

#[tokio::test]
async fn comment_writes_are_not_retried_and_map_the_answer() {
    let server = MockServer::start().await;
    let c = r#"{"id":"9","author":{"name":"shacohen","displayName":"Sha"},"body":"*b* [~jdoe]","created":"2026-10-19T09:00:00.000-0700","updated":"2026-10-19T09:30:00.000-0700"}"#;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/BCE-1/comment"))
        .respond_with(ResponseTemplate::new(201).set_body_raw(c, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT")).and(path("/rest/api/2/issue/BCE-1/comment/9")).respond_with(ResponseTemplate::new(503)).expect(1).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/rest/api/2/issue/BCE-1/comment/9")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    let j = client(&server);
    let added = j.add_comment("BCE-1", "*b* [~jdoe]").await.unwrap();
    assert_eq!(added.id, "9");
    assert_eq!(added.body.markdown, "**b** @jdoe");
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&reqs[0].body).unwrap(), serde_json::json!({"body": "*b* [~jdoe]"}));
    let e = j.edit_comment("BCE-1", "9", "x").await.unwrap_err();
    assert_eq!(e.kind(), "server", "a PUT is not retried on HTTP 503");
    j.delete_comment("BCE-1", "9").await.unwrap();
}

#[tokio::test]
async fn comment_context_gets_a_comment_that_the_issue_does_not_have() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/issue/BCE-62886"))
        .and(query_param("fields", "summary,comment"))
        .respond_with(json(include_str!("fixtures/issue_full.json")))
        .mount(&server)
        .await;
    let old = r#"{"id":"5","author":{"name":"old"},"body":"Old one.","created":"2026-01-01T09:00:00.000-0700"}"#;
    Mock::given(path("/rest/api/2/issue/BCE-62886/comment/5")).respond_with(json(old)).expect(1).mount(&server).await;
    let j = client(&server);
    let ctx = j.comment_context("BCE-62886", Some("7001")).await.unwrap();
    assert_eq!(ctx.summary, "Add retry to the upload client");
    assert_eq!(ctx.comments.len(), 2);
    let ctx = j.comment_context("BCE-62886", Some("5")).await.unwrap();
    assert_eq!(ctx.find("5").unwrap().body.wiki, "Old one.");
}

// ---------------------------------------------------------------- users, assign, watch

const USERS: &str = r#"[
  {"name":"jdoe","key":"JIRAUSER1","displayName":"Jane Doe","emailAddress":"jdoe@example.com","active":true},
  {"name":"old","key":"JIRAUSER9","displayName":"Old User","active":false}
]"#;

#[tokio::test]
async fn assignable_users_send_the_scope_and_skip_inactive_users() {
    use jui::core::jira::AssignScope;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/user/assignable/search"))
        .and(query_param("issueKey", "BCE-1"))
        .and(query_param("username", "jan"))
        .and(query_param("maxResults", "20"))
        .respond_with(json(USERS))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/user/assignable/search"))
        .and(query_param("project", "BCE"))
        .respond_with(json("[]"))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    let users = j.assignable_users(&AssignScope::Ticket("BCE-1".into()), "jan", 20).await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name, "jdoe");
    assert_eq!(users[0].display_name, "Jane Doe");
    assert!(j.assignable_users(&AssignScope::Project("BCE".into()), "x", 20).await.unwrap().is_empty());
}

#[tokio::test]
async fn assign_sends_the_name_null_or_me_and_is_not_retried() {
    use jui::core::model::User;
    use jui::core::user::Assignee;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/myself"))
        .respond_with(json(r#"{"name":"shacohen","displayName":"Sha Cohen"}"#))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/BCE-1/assignee"))
        .respond_with(ResponseTemplate::new(204))
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/rest/api/2/issue/BCE-2/assignee"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    let jane = User { name: "jdoe".into(), display_name: "Jane Doe".into() };
    j.assign("BCE-1", &Assignee::User(jane.clone())).await.unwrap();
    j.assign("BCE-1", &Assignee::Nobody).await.unwrap();
    j.assign("BCE-1", &Assignee::Me).await.unwrap();
    let e = j.assign("BCE-2", &Assignee::User(jane)).await.unwrap_err();
    assert_eq!(e.kind(), "server");

    let reqs = server.received_requests().await.unwrap();
    let bodies: Vec<serde_json::Value> = reqs
        .iter()
        .filter(|r| r.method.as_str() == "PUT" && r.url.path().ends_with("BCE-1/assignee"))
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(bodies[0], serde_json::json!({"name": "jdoe"}));
    assert_eq!(bodies[1], serde_json::json!({"name": null}));
    assert_eq!(bodies[2], serde_json::json!({"name": "shacohen"}));
}

#[tokio::test]
async fn watchers_watch_and_unwatch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/BCE-1/watchers"))
        .respond_with(json(r#"{"isWatching":true,"watchCount":2,"watchers":[
            {"name":"shacohen","displayName":"Sha Cohen","active":true},
            {"name":"jdoe","displayName":"Jane Doe","active":true}]}"#))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/myself"))
        .respond_with(json(r#"{"name":"shacohen"}"#))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/BCE-1/watchers"))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/rest/api/2/issue/BCE-1/watchers"))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    let j = client(&server);
    let w = j.watchers("BCE-1").await.unwrap();
    assert!(w.watching);
    assert_eq!(w.count, 2);
    assert_eq!(w.users[1].name, "jdoe");

    j.watch("BCE-1", None).await.unwrap();
    j.watch("BCE-1", Some("jdoe")).await.unwrap();
    j.unwatch("BCE-1", None).await.unwrap();
    j.unwatch("BCE-1", Some("jdoe")).await.unwrap();

    let reqs = server.received_requests().await.unwrap();
    let posts: Vec<&wiremock::Request> = reqs.iter().filter(|r| r.method.as_str() == "POST").collect();
    // An empty body adds the current user. A named user is a JSON string.
    assert!(posts[0].body.is_empty());
    assert_eq!(posts[1].body, b"\"jdoe\"");
    assert_eq!(posts[1].headers.get("content-type").unwrap(), "application/json");
    let deletes: Vec<String> = reqs
        .iter()
        .filter(|r| r.method.as_str() == "DELETE")
        .map(|r| r.url.query().unwrap_or("").to_string())
        .collect();
    assert_eq!(deletes, ["username=shacohen", "username=jdoe"]);
}

#[tokio::test]
async fn ticket_reads_the_watch_state() {
    let server = MockServer::start().await;
    let mut issue: serde_json::Value = serde_json::from_str(include_str!("fixtures/issue_full.json")).unwrap();
    issue["fields"]["watches"] = serde_json::json!({"watchCount": 3, "isWatching": true});
    Mock::given(path("/rest/api/2/issue/BCE-62886"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&issue))
        .mount(&server)
        .await;
    let t = client(&server).ticket("BCE-62886").await.unwrap();
    assert!(t.watching);
    assert_eq!(t.watch_count, 3);
    let reqs = server.received_requests().await.unwrap();
    let fields = reqs[0].url.query_pairs().find(|(k, _)| k == "fields").unwrap().1.to_string();
    assert!(fields.ends_with(",watches"), "{fields}");
}

// ------------------------------------------------------------------ saved filters

#[tokio::test]
async fn favourite_filters_and_one_filter_map_to_domain_types() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/filter/favourite"))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(json(include_str!("fixtures/filter_favourites.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/filter/117549"))
        .respond_with(json(include_str!("fixtures/filter_one.json")))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    let favs = j.favourite_filters().await.unwrap();
    assert_eq!(favs.len(), 3);
    assert_eq!(favs[0].name, "BCE bugs");
    assert_eq!(favs[0].owner.as_deref(), Some("shacohen"));
    assert_eq!(favs[0].description.as_deref(), Some("Open bugs in BCE"));
    // With no `viewUrl`, jui makes the URL from the base URL.
    assert_eq!(favs[1].url, format!("{}/issues/?filter=202", server.uri()));
    let f = j.filter("117549").await.unwrap();
    assert_eq!(f.jql, "project = BCE ORDER BY Rank ASC");
    assert!(!f.favourite);
    assert_eq!(f.description, None, "an empty description is None");
    assert_eq!(f.url, "https://jira.example/issues/?filter=117549");
}

#[tokio::test]
async fn an_unknown_filter_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/filter/9"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(r#"{"errorMessages":["The filter does not exist."]}"#, "application/json"))
        .mount(&server)
        .await;
    let e = client(&server).filter("9").await.unwrap_err();
    assert_eq!(e.to_string(), "Filter 9 not found.");
    assert_eq!(e.exit_code(), 4);
}

// ------------------------------------------------------------------ Boards

const BOARD_CONFIG_SCRUM: &str = include_str!("fixtures/board_config_scrum.json");
const BOARD_CONFIG_KANBAN: &str = include_str!("fixtures/board_config_kanban.json");
const BOARD_SPRINTS: &str = include_str!("fixtures/board_sprints_active.json");
const BOARD_SEARCH: &str = include_str!("fixtures/board_search.json");
const BOARDS_BCE: &str = include_str!("fixtures/boards_bce.json");
const MENU: &str = include_str!("fixtures/greenhopper_menu.json");

#[tokio::test]
async fn scrum_board_view_uses_the_active_sprints_and_the_columns() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4166/configuration")).respond_with(json(BOARD_CONFIG_SCRUM)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint"))
        .and(query_param("state", "active"))
        .respond_with(json(BOARD_SPRINTS))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .and(query_param(
            "jql",
            "filter = 117549 AND sprint in (43198) AND issuetype != Epic AND status in (10000, 3, 10001) ORDER BY Rank ASC",
        ))
        .and(query_param("fields", "summary,issuetype,status,priority,assignee,updated"))
        .respond_with(json(BOARD_SEARCH))
        .expect(1)
        .mount(&server)
        .await;

    let v = client(&server).board_view(4166, 1000, false).await.unwrap();
    assert_eq!(v.board.name, "BCE board");
    assert_eq!(v.sprints[0].name, "BCE-26-Sprint-21");
    assert_eq!(v.filter_id.as_deref(), Some("117549"));
    assert_eq!(v.url, format!("{}/secure/RapidBoard.jspa?rapidView=4166", server.uri()));
    let cols: Vec<(&str, Vec<&str>)> =
        v.columns.iter().map(|c| (c.name.as_str(), c.tickets.iter().map(|t| t.key.as_str()).collect())).collect();
    assert_eq!(cols, vec![("To Do", vec!["BCE-1"]), ("In Progress", vec!["BCE-2"]), ("Done", vec!["BCE-3"])]);
    assert_eq!((v.total, v.loaded, v.unmapped), (3, 3, 0));
    assert!(!v.truncated());
}

#[tokio::test]
async fn kanban_board_view_uses_the_sub_query_and_14_days_and_the_limit() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4533/configuration")).respond_with(json(BOARD_CONFIG_KANBAN)).mount(&server).await;
    // A Kanban Board has no Sprints: Jira gives HTTP 400, and jui ignores it.
    Mock::given(path("/rest/agile/1.0/board/4533/sprint")).respond_with(ResponseTemplate::new(400)).mount(&server).await;
    let mut big: serde_json::Value = serde_json::from_str(BOARD_SEARCH).unwrap();
    big["total"] = 1534.into();
    Mock::given(path("/rest/api/2/search"))
        .and(query_param(
            "jql",
            "filter = 96374 AND (fixVersion is EMPTY) AND (statusCategory != Done OR resolved >= -14d) AND status in (10000, 10001) ORDER BY Rank ASC",
        ))
        .and(query_param("maxResults", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&big))
        .expect(1)
        .mount(&server)
        .await;

    let v = client(&server).board_view(4533, 2, false).await.unwrap();
    assert!(v.sprints.is_empty());
    assert!(!v.no_active_sprint());
    // The limit is 2: jui keeps 2 Tickets and tells the total.
    assert_eq!((v.loaded, v.total), (2, 1534));
    assert_eq!(v.count(), "2 of 1534");
    // BCE-2 has status 3, which is in no column of this Board.
    assert_eq!(v.unmapped, 1);
    assert_eq!(v.columns[0].tickets.len(), 0);
    assert_eq!(v.columns[1].tickets[0].key, "BCE-1");
}

#[tokio::test]
async fn scrum_board_with_no_active_sprint_gets_no_tickets() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4166/configuration")).respond_with(json(BOARD_CONFIG_SCRUM)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint"))
        .respond_with(json(r#"{"maxResults":50,"startAt":0,"isLast":true,"values":[]}"#))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/search")).respond_with(json(BOARD_SEARCH)).expect(0).mount(&server).await;
    let v = client(&server).board_view(4166, 1000, false).await.unwrap();
    assert!(v.no_active_sprint());
    assert_eq!(v.jql, None);
}

#[tokio::test]
async fn board_list_sends_the_filters_and_a_missing_board_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board"))
        .and(query_param("projectKeyOrId", "BCE"))
        .and(query_param("name", "bce"))
        .and(query_param("type", "scrum"))
        .respond_with(json(BOARDS_BCE))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/rest/agile/1.0/board/9"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(r#"{"errorMessages":[],"errors":{"rapidViewId":"x"}}"#, "application/json"))
        .mount(&server)
        .await;
    let j = client(&server);
    let q = jui::core::jira::BoardQuery { name: Some("bce".into()), project: Some("BCE".into()), kind: Some("scrum".into()) };
    let p = j.boards_all(&q, None).await.unwrap();
    assert_eq!(p.items.len(), 3);
    assert_eq!(p.items[1].kind, "scrum");
    let e = j.board(9).await.unwrap_err();
    assert_eq!(e.to_string(), "Board 9 not found.");
    assert_eq!(e.exit_code(), 4);
}

#[tokio::test]
async fn suggestions_put_recent_first_and_a_broken_menu_is_ignored() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board")).and(query_param("projectKeyOrId", "BCE")).respond_with(json(BOARDS_BCE)).mount(&server).await;
    Mock::given(path("/rest/api/1.0/menus/greenhopper_menu")).respond_with(json(MENU)).up_to_n_times(1).mount(&server).await;
    let j = client(&server);
    let s = j.board_suggestions("BCE").await.unwrap();
    let ids: Vec<u64> = s.iter().map(|s| s.board.id).collect();
    // Recent first (4537, 4166), then the project Boards: Scrum first, then by name. No duplicates.
    assert_eq!(ids, vec![4537, 4166, 6589, 4533]);

    // The private endpoint fails (or changes): no Recent Boards and no error.
    Mock::given(path("/rest/api/1.0/menus/greenhopper_menu")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
    let s = j.board_suggestions("BCE").await.unwrap();
    assert_eq!(s.iter().map(|s| s.board.id).collect::<Vec<_>>(), vec![6589, 4166, 4533]);
}

// ------------------------------------------------------------------ Sprints and Epics

const SPRINTS_OPEN: &str = include_str!("fixtures/board_sprints_open.json");
const SPRINT_ONE: &str = include_str!("fixtures/sprint_one.json");
const EPIC_ONE: &str = include_str!("fixtures/epic_one.json");
const EPIC_SEARCH: &str = include_str!("fixtures/epic_search.json");
const CONFIG_SCRUM: &str = include_str!("fixtures/board_config_scrum.json");
const CONFIG_KANBAN: &str = include_str!("fixtures/board_config_kanban.json");

#[tokio::test]
async fn sprints_follow_pages_and_map_dates() {
    let server = MockServer::start().await;
    let page1 = r#"{"maxResults":1,"startAt":0,"isLast":false,"values":[{"id":1,"name":"A","state":"ACTIVE"}]}"#;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint"))
        .and(query_param("state", "active,future"))
        .and(query_param("startAt", "0"))
        .respond_with(json(page1))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint"))
        .and(query_param("startAt", "1"))
        .respond_with(json(SPRINTS_OPEN))
        .expect(1)
        .mount(&server)
        .await;
    let all = client(&server).sprints(4166, "active,future").await.unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].state, "active", "the state is lower case");
    let s = &all[2];
    assert_eq!(s.id, 43198);
    assert_eq!(s.start.unwrap().to_rfc3339(), "2026-10-12T01:07:00-07:00");
    assert_eq!(s.goal, None, "an empty goal is no goal");
    assert_eq!(s.board_id, Some(4166));
    let open: Vec<u64> = jui::core::agile::open_sprints(all).iter().map(|s| s.id).collect();
    assert_eq!(open, vec![1, 43198, 20744, 43199]);
}

#[tokio::test]
async fn sprints_of_a_kanban_board_give_an_input_error() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4533/sprint"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(r#"{"errorMessages":["The board doesn't support sprints."],"errors":{}}"#, "application/json"))
        .mount(&server)
        .await;
    let e = client(&server).sprints(4533, "active,future").await.unwrap_err();
    assert_eq!(e.kind(), "input");
    assert_eq!(e.to_string(), "Board 4533 is a Kanban Board. It has no Sprints.");
}

#[tokio::test]
async fn sprint_board_uses_given_sprints_or_gets_them() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4166/configuration")).respond_with(json(CONFIG_SCRUM)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint"))
        .and(query_param("state", "active"))
        .respond_with(json(include_str!("fixtures/board_sprints_active.json")))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    let given = vec![model::SprintRef { id: 7, name: "Given".into(), state: "active".into() }];
    let b = j.sprint_board(4166, Some(given)).await.unwrap();
    assert_eq!(b.sprints[0].id, 7, "no Sprint request when the caller has the Sprints");
    let b = j.sprint_board(4166, None).await.unwrap();
    assert_eq!(b.sprints[0].id, 43198);
    assert_eq!(
        b.backlog_jql.as_deref(),
        Some(
            "filter = 117549 AND (sprint is EMPTY OR sprint not in (openSprints(), futureSprints())) \
             AND issuetype != Epic AND status in (10000, 3) ORDER BY Rank ASC"
        )
    );

    // A Kanban Board: no Sprints, no backlog, and the HTTP 400 of the Sprint list is ignored.
    Mock::given(path("/rest/agile/1.0/board/4533/configuration")).respond_with(json(CONFIG_KANBAN)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/board/4533/sprint")).respond_with(ResponseTemplate::new(400)).mount(&server).await;
    let k = j.sprint_board(4533, None).await.unwrap();
    assert!(k.sprints.is_empty() && k.backlog_jql.is_none());
}

#[tokio::test]
async fn one_sprint_and_a_missing_sprint() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/sprint/43198")).respond_with(json(SPRINT_ONE)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/sprint/9")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let j = client(&server);
    let s = j.sprint(43198).await.unwrap();
    assert_eq!(s.goal.as_deref(), Some("Finish the scanner"));
    let e = j.sprint(9).await.unwrap_err();
    assert_eq!(e.to_string(), "Sprint 9 not found.");
}

#[tokio::test]
async fn moves_send_chunks_of_50_and_are_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/43198/issue"))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/backlog/issue"))
        .and(wiremock::matchers::body_json(serde_json::json!({ "issues": ["BCE-1"] })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/sprint/1/issue"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    let keys: Vec<String> = (1..=60).map(|n| format!("BCE-{n}")).collect();
    j.move_to_sprint(43198, &keys).await.unwrap();
    j.move_to_backlog(&keys[..1]).await.unwrap();
    assert!(j.move_to_sprint(1, &keys[..1]).await.is_err());
    let reqs = server.received_requests().await.unwrap();
    let first: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(first["issues"].as_array().unwrap().len(), 50);
    let second: serde_json::Value = serde_json::from_slice(&reqs[1].body).unwrap();
    assert_eq!(second["issues"][0], "BCE-51");
}

#[tokio::test]
async fn epics_map_the_name_and_the_done_state() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("fields", "summary,status,customfield_10017,customfield_10018"))
        .and(query_param("jql", "issuetype = Epic AND filter = 117549 AND (cf[10017] is EMPTY OR cf[10017] != Done) ORDER BY Rank ASC"))
        .respond_with(json(EPIC_SEARCH))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/rest/agile/1.0/board/4166/configuration")).respond_with(json(CONFIG_SCRUM)).mount(&server).await;
    let (b, page) = client(&server).board_epics(4166, false, Some(50)).await.unwrap();
    assert_eq!(b.name, "BCE board");
    assert_eq!(page.total, 3);
    assert_eq!(page.items[1].name, "Proxy - Cider Integration MVP");
    assert!(!page.items[1].done);
    assert_eq!(page.items[2].name, "Old epic with no name", "no Epic Name: the summary");
    assert!(page.items[2].done);
}

#[tokio::test]
async fn one_epic_and_a_key_that_is_not_an_epic() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/epic/BCE-13070")).respond_with(json(EPIC_ONE)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/epic/BCE-1"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(r#"{"errorMessages":["The requested epic cannot be viewed"],"errors":{}}"#, "application/json"))
        .mount(&server)
        .await;
    let j = client(&server);
    let e = j.epic("BCE-13070").await.unwrap();
    assert_eq!((e.key.as_str(), e.name.as_str(), e.done), ("BCE-13070", "Proxy - Cider Integration MVP", false));
    let err = j.epic("BCE-1").await.unwrap_err();
    assert_eq!(err.kind(), "input");
    assert_eq!(err.to_string(), "BCE-1 is not an Epic, or you cannot see it.");
}

#[tokio::test]
async fn set_and_unset_epic_post_the_keys() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/epic/BCE-13070/issue"))
        .and(wiremock::matchers::body_json(serde_json::json!({ "issues": ["BCE-1", "BCE-2"] })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/agile/1.0/epic/none/issue"))
        .and(wiremock::matchers::body_json(serde_json::json!({ "issues": ["BCE-3"] })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let j = client(&server);
    j.set_epic("BCE-13070", &["BCE-1".into(), "BCE-2".into()]).await.unwrap();
    j.unset_epic(&["BCE-3".into()]).await.unwrap();
}

// ---------------------------------------------------------------- Sub-tasks, Links, Worklogs

#[tokio::test]
async fn worklog_context_has_the_estimates_and_gets_all_worklogs() {
    let server = MockServer::start().await;
    let issue = r#"{"key":"BCE-1","fields":{"summary":"S","timetracking":{"originalEstimate":"1d","remainingEstimate":"5h","timeSpent":"3h"},
        "worklog":{"startAt":0,"maxResults":20,"total":2,"worklogs":[{"id":"1","timeSpent":"1h","timeSpentSeconds":3600}]}}}"#;
    Mock::given(path("/rest/api/2/issue/BCE-1")).and(query_param("fields", "summary,timetracking,worklog")).respond_with(json(issue)).expect(1).mount(&server).await;
    Mock::given(path("/rest/api/2/issue/BCE-1/worklog"))
        .respond_with(json(r#"{"startAt":0,"maxResults":2,"total":2,"worklogs":[
            {"id":"1","timeSpent":"1h","timeSpentSeconds":3600},{"id":"2","timeSpent":"2h","timeSpentSeconds":7200}]}"#))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server).worklog_context("BCE-1").await.unwrap();
    assert_eq!((c.original_estimate.as_deref(), c.remaining_estimate.as_deref(), c.time_spent.as_deref()), (Some("1d"), Some("5h"), Some("3h")));
    assert_eq!(c.worklogs.len(), 2);
    assert!(c.find("2").is_some());
}

#[tokio::test]
async fn delete_worklog_sends_the_estimate_query_and_a_missing_one_is_not_found() {
    use jui::core::worklog::Adjust;
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/rest/api/2/issue/BCE-1/worklog/9"))
        .and(query_param("adjustEstimate", "new"))
        .and(query_param("newEstimate", "2h"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/BCE-1/worklog/8"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(r#"{"errorMessages":["Cannot find worklog with id: 8"],"errors":{}}"#, "application/json"))
        .mount(&server)
        .await;
    let j = client(&server);
    j.delete_worklog("BCE-1", "9", &Adjust::New("2h".into())).await.unwrap();
    let e = j.worklog("BCE-1", "8").await.unwrap_err();
    assert_eq!(e.kind(), "not_found");
}

#[tokio::test]
async fn delete_link_and_link_types() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE")).and(path("/rest/api/2/issueLink/501")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/rest/api/2/issueLinkType")).respond_with(json(include_str!("fixtures/link_types.json"))).mount(&server).await;
    let j = client(&server);
    j.delete_link("501").await.unwrap();
    let t = j.link_types().await.unwrap();
    assert_eq!(t[0], model::LinkType { id: "10000".into(), name: "Blocks".into(), inward: "is blocked by".into(), outward: "blocks".into() });
}

// ------------------------------------------------------------------ attachments

fn attachment_json(server: &str, id: &str, name: &str, size: u64) -> serde_json::Value {
    serde_json::json!({
        "id": id, "filename": name, "size": size, "mimeType": "text/plain",
        "author": {"name": "jdoe", "displayName": "Jane Doe"}, "created": "2026-10-11T09:00:00.000-0700",
        "content": format!("{server}/secure/attachment/{id}/{name}")
    })
}

#[tokio::test]
async fn attachments_list_and_meta() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"key": "BCE-1", "fields": {"summary": "S", "attachment": [attachment_json(&server.uri(), "900", "trace.log", 2048)]}});
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/BCE-1"))
        .and(query_param("fields", "summary,attachment"))
        .respond_with(json(&body.to_string()))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/rest/api/2/attachment/meta")).respond_with(json(r#"{"enabled":true,"uploadLimit":1000}"#)).mount(&server).await;
    let j = client(&server);
    let (summary, atts) = j.attachments("BCE-1").await.unwrap();
    assert_eq!((summary.as_str(), atts.len(), atts[0].id.as_str(), atts[0].size), ("S", 1, "900", 2048));
    assert_eq!(atts[0].author.as_ref().unwrap().name, "jdoe");
    assert_eq!(j.attachment_meta().await.unwrap().limit, 1000);
}

#[tokio::test]
async fn attachments_off_is_an_input_error() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/attachment/meta")).respond_with(json(r#"{"enabled":false,"uploadLimit":1000}"#)).mount(&server).await;
    assert_eq!(client(&server).attachment_meta().await.unwrap_err().kind(), "input");
}

#[tokio::test]
async fn download_sends_bearer_to_the_base_url_and_writes_the_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/secure/attachment/900/trace.log"))
        .and(header("Authorization", "Bearer test-token"))
        .and(header("Accept", "*/*"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"line 1\nline 2\n".to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    let a: model::Attachment = serde_json::from_value::<jui::core::api::Attachment>(attachment_json(&server.uri(), "900", "trace.log", 14)).unwrap().into();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.log");
    let n = client(&server).download_to(&a, &target).await.unwrap();
    assert_eq!(n, 14);
    assert_eq!(std::fs::read(&target).unwrap(), b"line 1\nline 2\n");
    // No temporary file stays.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o644);
    }
}

#[tokio::test]
async fn download_of_a_missing_attachment_leaves_no_file() {
    let server = MockServer::start().await;
    Mock::given(path("/secure/attachment/901/gone.txt")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let a: model::Attachment = serde_json::from_value::<jui::core::api::Attachment>(attachment_json(&server.uri(), "901", "gone.txt", 1)).unwrap().into();
    let dir = tempfile::tempdir().unwrap();
    let e = client(&server).download_to(&a, &dir.path().join("gone.txt")).await.unwrap_err();
    assert_eq!(e.to_string(), "Attachment 901 (gone.txt) not found.");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn upload_sends_multipart_with_the_no_check_header_once() {
    let server = MockServer::start().await;
    let resp = serde_json::json!([attachment_json(&server.uri(), "950", "a b.txt", 5)]);
    Mock::given(method("POST"))
        .and(path("/rest/api/2/issue/BCE-1/attachments"))
        .and(header("X-Atlassian-Token", "no-check"))
        .respond_with(json(&resp.to_string()))
        .expect(1)
        .mount(&server)
        .await;
    let file = jui::core::attachment::Upload { name: "a b.txt".into(), mime: "text/plain", bytes: b"hello".to_vec() };
    let added = client(&server).upload("BCE-1", &file).await.unwrap();
    assert_eq!(added[0].id, "950");
    let reqs = server.received_requests().await.unwrap();
    let ct = reqs[0].headers.get("content-type").unwrap().to_str().unwrap().to_string();
    assert!(ct.starts_with("multipart/form-data; boundary="), "{ct}");
    let body = String::from_utf8_lossy(&reqs[0].body);
    assert!(body.contains(r#"Content-Disposition: form-data; name="file"; filename="a b.txt""#), "{body}");
    assert!(body.contains("Content-Type: text/plain"), "{body}");
    assert!(body.contains("\r\n\r\nhello\r\n"), "{body}");
}

#[tokio::test]
async fn upload_is_not_retried_after_a_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/rest/api/2/issue/BCE-1/attachments")).respond_with(ResponseTemplate::new(503)).expect(1).mount(&server).await;
    let file = jui::core::attachment::Upload { name: "x.txt".into(), mime: "text/plain", bytes: b"x".to_vec() };
    assert_eq!(client(&server).upload("BCE-1", &file).await.unwrap_err().kind(), "server");
}

#[tokio::test]
async fn delete_attachment_and_a_missing_one() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE")).and(path("/rest/api/2/attachment/900")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    Mock::given(method("DELETE")).and(path("/rest/api/2/attachment/1")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let j = client(&server);
    j.delete_attachment("900").await.unwrap();
    assert_eq!(j.delete_attachment("1").await.unwrap_err().to_string(), "Attachment 1 not found.");
}

#[tokio::test]
async fn project_key_checks_the_project_with_a_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/project/XSUP"))
        .respond_with(json(r#"{"id":"10100","key":"XSUP","name":"Support"}"#))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/project/NOPE"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_raw(r#"{"errorMessages":["No project could be found with key 'NOPE'."],"errors":{}}"#, "application/json"),
        )
        .mount(&server)
        .await;

    let j = client(&server);
    assert_eq!(j.project_key(" xsup ").await.unwrap(), "XSUP");
    let e = j.project_key("nope").await.unwrap_err();
    assert!(matches!(e, jui::core::Error::NotFound(_)), "{e:?}");
    let reqs = server.received_requests().await.unwrap();
    assert!(reqs.iter().all(|r| r.method == wiremock::http::Method::GET));
}
