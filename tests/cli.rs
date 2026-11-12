//! CLI tests: run the binary against a mock HTTP server, with a temporary config file.
//! They never read the user's config file or use the real Jira.

use assert_cmd::Command;
use wiremock::matchers::{header, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PAGE1: &str = include_str!("fixtures/search_page1.json");
const PAGE2: &str = include_str!("fixtures/search_page2.json");

struct Ctx {
    dir: tempfile::TempDir,
    base_url: String,
}

impl Ctx {
    fn new(base_url: &str) -> Self {
        Ctx { dir: tempfile::tempdir().unwrap(), base_url: base_url.to_string() }
    }
    fn config(&self) -> std::path::PathBuf {
        self.dir.path().join("config.toml")
    }
    fn jui(&self) -> Command {
        let mut c = Command::cargo_bin("jui").unwrap();
        c.env_clear()
            .env("JUI_CONFIG", self.config())
            .env("JUI_BASE_URL", &self.base_url)
            .env("JIRA_TOKEN", "test-token")
            .env("NO_COLOR", "1")
            .env("HOME", self.dir.path());
        c
    }
}

fn json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

async fn server_with_tickets() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .and(header("Authorization", "Bearer test-token"))
        .and(query_param("startAt", "0"))
        .respond_with(json(PAGE1))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("startAt", "2"))
        .respond_with(json(PAGE2))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn ls_uses_my_tickets_jql_and_shows_table() {
    let server = server_with_tickets().await;
    let ctx = Ctx::new(&server.uri());
    std::fs::write(ctx.config(), "[tickets]\nmy_tickets_jql = \"assignee = jdoe\"\n").unwrap();

    let out = ctx.jui().args(["ls", "--all"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("BCE-62886"));
    assert!(stdout.contains("In Pull Request"));
    assert!(stdout.contains("XSUP-1182"));
    // One table for each Sprint group: active, future, then No Sprint. The Updated column
    // changes with the date, so the snapshot has the keys and the headings only.
    let shape: Vec<String> = stdout
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            if t.is_empty() || t.starts_with("Key") || t.starts_with("BCE-26") || t.starts_with("No Sprint") {
                Some(t.split("  ").next().unwrap().to_string())
            } else {
                t.split_whitespace().next().filter(|k| k.contains('-')).map(|k| format!("  {k}"))
            }
        })
        .collect();
    insta::assert_snapshot!("ls_grouped_by_sprint", shape.join("\n"));

    let reqs = server.received_requests().await.unwrap();
    assert!(reqs.iter().all(|r| r.url.query_pairs().any(|(k, v)| k == "jql" && v == "assignee = jdoe")));
    assert!(reqs.iter().all(|r| r.url.query_pairs().any(|(k, v)| k == "fields" && v.contains("customfield_10020"))));
}

#[tokio::test]
async fn ls_with_group_by_sprint_off_is_one_table_in_query_order() {
    let server = server_with_tickets().await;
    let ctx = Ctx::new(&server.uri());
    std::fs::write(ctx.config(), "[tickets]\ngroup_by_sprint = false\n").unwrap();
    let out = ctx.jui().args(["ls", "--all"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    insta::assert_snapshot!("ls_table_header", stdout.lines().next().unwrap().trim_end());
    assert!(!stdout.contains("No Sprint"));
    let keys: Vec<&str> = stdout.lines().skip(1).filter_map(|l| l.split_whitespace().next()).collect();
    assert_eq!(keys, ["BCE-62886", "BCE-63337", "XSUP-1182"]);

    // `jui search` is never grouped.
    let out = Ctx::new(&server.uri()).jui().args(["search", "project = BCE", "--all"]).output().unwrap();
    assert!(!String::from_utf8(out.stdout).unwrap().contains("No Sprint"));
}

#[tokio::test]
async fn ls_json_is_domain_types() {
    let server = server_with_tickets().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["ls", "--json", "--all"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    insta::assert_json_snapshot!("ls_json", v);
}

#[tokio::test]
async fn ls_limit_shows_more_message_on_stderr() {
    let server = server_with_tickets().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["ls", "--limit", "2", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "Showing 2 of 3. Use --limit or --all.");
}

#[tokio::test]
async fn search_passes_jql() {
    let server = server_with_tickets().await;
    let ctx = Ctx::new(&server.uri());
    ctx.jui().args(["search", "project = XSUP", "--all"]).assert().success();
    let reqs = server.received_requests().await.unwrap();
    assert!(reqs[0].url.query_pairs().any(|(k, v)| k == "jql" && v == "project = XSUP"));
}

#[test]
fn no_token_exits_3_with_hint() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    let out = ctx.jui().env_remove("JIRA_TOKEN").arg("ls").output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    insta::assert_snapshot!("no_token", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn no_token_json_error() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    let out = ctx.jui().env_remove("JIRA_TOKEN").args(["ls", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["kind"], "no_token");
}

#[tokio::test]
async fn bad_jql_exits_5() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"errorMessages":["Error in the JQL Query: Expecting operator."],"errors":{}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["search", "project ="]).output().unwrap();
    assert_eq!(out.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Error in the JQL Query"));
}

#[test]
fn network_error_exits_6() {
    // Port 9 (discard) is closed on test machines.
    let ctx = Ctx::new("http://127.0.0.1:9");
    let out = ctx.jui().arg("ls").output().unwrap();
    assert_eq!(out.status.code(), Some(6), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("hint: Check the VPN."));
}

#[test]
fn config_syntax_error_exits_7() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    std::fs::write(ctx.config(), "[jira\n").unwrap();
    let out = ctx.jui().args(["config", "show"]).output().unwrap();
    assert_eq!(out.status.code(), Some(7));
    // `config path` still works.
    ctx.jui().args(["config", "path"]).assert().success();
}

#[test]
fn config_set_get_unset_round_trip() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    ctx.jui().args(["config", "set", "tickets.default_project", "xsup"]).assert().success();
    let out = ctx.jui().args(["config", "get", "tickets.default_project"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "XSUP");
    let text = std::fs::read_to_string(ctx.config()).unwrap();
    assert!(text.starts_with("# jui Settings."), "the template is written first");

    ctx.jui().args(["config", "unset", "tickets.default_project"]).assert().success();
    let out = ctx.jui().args(["config", "get", "tickets.default_project"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "BCE");
}

#[test]
fn config_set_rejects_bad_value() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    let out = ctx.jui().args(["config", "set", "display.color", "pink"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Permitted: auto, always, never."));
}

#[test]
fn config_show_marks_env_source() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    let out = ctx.jui().args(["config", "show", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["base_url"], "http://127.0.0.1:9");
    let out = ctx.jui().args(["config", "show"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("jira.base_url")).unwrap();
    assert!(line.trim_end().ends_with("env"), "{line}");
}

#[test]
fn unknown_key_in_file_warns() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    std::fs::write(ctx.config(), "[jira]\nbase_ulr = \"x\"\n").unwrap();
    let out = ctx.jui().args(["config", "get", "jira.timeout_secs"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning: Unknown Setting `jira.base_ulr`"));
}

#[test]
fn config_init_does_not_overwrite() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    ctx.jui().args(["config", "init"]).assert().success();
    ctx.jui().args(["config", "init"]).assert().code(2);
    ctx.jui().args(["config", "init", "--force"]).assert().success();
}

#[tokio::test]
async fn verbose_log_never_contains_token() {
    let server = server_with_tickets().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["-vv", "ls", "--all"]).output().unwrap();
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("GET /rest/api/2/search"), "{err}");
    assert!(!err.contains("test-token"));
}

const ISSUE: &str = include_str!("fixtures/issue_full.json");
const AGILE: &str = include_str!("fixtures/agile_issue.json");

async fn server_with_issue() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/issue/BCE-62886")).respond_with(json(ISSUE)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/issue/BCE-62886")).respond_with(json(AGILE)).mount(&server).await;
    server
}

/// Remove the parts of `view` output that change with the test date.
fn without_relative_dates(s: &str) -> String {
    s.lines()
        .map(|l| match l.find(" (") {
            Some(p) if l.trim_end().ends_with(')') && (l.starts_with("Created") || l.starts_with("Updated") || l.trim_start().starts_with('@')) => {
                l[..p].to_string()
            }
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn view_shows_ticket() {
    let server = server_with_issue().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["view", "BCE-62886"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap().replace(&server.uri(), "http://jira.test");
    insta::assert_snapshot!("view", without_relative_dates(&text));
}

#[tokio::test]
async fn bare_and_short_keys_use_default_project() {
    let server = server_with_issue().await;
    let ctx = Ctx::new(&server.uri());
    ctx.jui().arg("BCE-62886").assert().success();
    ctx.jui().arg("62886").assert().success();
    ctx.jui().args(["view", "bce-62886"]).assert().success();
}

#[tokio::test]
async fn view_wiki_and_comment_count() {
    let server = server_with_issue().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["view", "BCE-62886", "--wiki", "--comments", "1"]).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("{color:#de350b}Important{color}"));
    assert!(text.contains("Comments (1 of 2, --comments all for all)"));
    assert!(text.contains("Done. See {{retry.rs}}."));
    assert!(!text.contains("cover *429*"));
}

#[tokio::test]
async fn view_json_has_wiki_and_markdown() {
    let server = server_with_issue().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["view", "BCE-62886", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["key"], "BCE-62886");
    assert!(v["description"]["wiki"].as_str().unwrap().starts_with("h3. Goal"));
    assert!(v["description"]["markdown"].as_str().unwrap().starts_with("### Goal"));
    assert_eq!(v["comments"][1]["body"]["markdown"], "Done. See `retry.rs`.");
}

#[tokio::test]
async fn view_raw_is_jira_json() {
    let server = server_with_issue().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["view", "BCE-62886", "--raw"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["fields"]["customfield_10016"], "BCE-47921");
}

#[tokio::test]
async fn view_not_found_exits_4() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/issue/BCE-1")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["view", "BCE-1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Ticket BCE-1 not found."));
}

#[test]
fn view_bad_key_exits_2() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    ctx.jui().args(["view", "nonsense"]).assert().code(2);
}

#[test]
fn open_print_uses_base_url() {
    let ctx = Ctx::new("https://jira.example");
    let out = ctx.jui().args(["open", "62886", "--print"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "https://jira.example/browse/BCE-62886");
}

#[test]
fn tui_without_a_terminal_exits_2() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    for args in [&[][..], &["tui"][..]] {
        let out = ctx.jui().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).contains("The TUI needs a terminal."));
    }
}

const TR_TASK: &str = include_str!("fixtures/transitions_task.json");

async fn server_with_transitions() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/issue/BCE-62886/transitions"))
        .and(wiremock::matchers::method("GET"))
        .respond_with(json(TR_TASK))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn mv_with_prefix_posts_the_transition() {
    let server = server_with_transitions().await;
    Mock::given(path("/rest/api/2/issue/BCE-62886/transitions"))
        .and(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::body_json(serde_json::json!({"transition": {"id": "41"}})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["mv", "62886", "pull"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "BCE-62886 moved to In Pull Request.");
    assert!(out.stdout.is_empty());
}

#[tokio::test]
async fn mv_done_needs_resolution_in_a_script() {
    let server = server_with_transitions().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["mv", "BCE-62886", "done"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("The Transition to Done needs Resolution."), "{err}");
    assert!(err.contains("--resolution <name>"), "{err}");
    let posts = server.received_requests().await.unwrap().iter().filter(|r| r.method.as_str() == "POST").count();
    assert_eq!(posts, 0, "no write when a field is missing");
}

#[tokio::test]
async fn mv_done_with_resolution_set_and_comment() {
    let server = server_with_transitions().await;
    Mock::given(path("/rest/api/2/issue/BCE-62886/transitions"))
        .and(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::body_json(serde_json::json!({
            "transition": {"id": "31"},
            "fields": {"resolution": {"id": "10001"}, "customfield_10021": 2.0},
            "update": {"comment": [{"add": {"body": "Not needed."}}]}
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    ctx.jui()
        .args(["ticket", "move", "BCE-62886", "Done", "-r", "won't", "--set", "Story Points=2", "-m", "Not needed."])
        .assert()
        .success();
}

#[tokio::test]
async fn mv_ambiguous_or_unknown_target_lists_the_targets() {
    let server = server_with_transitions().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["mv", "BCE-62886", "in"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("`in` matches more than one Transition of BCE-62886."), "{err}");
    assert!(err.contains("In Progress, In Pull Request, In Product Design"), "{err}");

    let out = ctx.jui().args(["mv", "BCE-62886"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Give the target Status for BCE-62886."));
}

#[tokio::test]
async fn mv_validation_error_exits_5_with_field_name() {
    let server = server_with_transitions().await;
    Mock::given(path("/rest/api/2/issue/BCE-62886/transitions"))
        .and(wiremock::matchers::method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"errorMessages":[],"errors":{"customfield_10021":"Number value expected."}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["mv", "BCE-62886", "done", "-r", "done"]).output().unwrap();
    assert_eq!(out.status.code(), Some(5));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Story Points: Number value expected."), "{err}");
}

// ------------------------------------------------------------------ create and edit

const TYPES: &str = include_str!("fixtures/createmeta_bce_types.json");
const META_BUG: &str = include_str!("fixtures/createmeta_bce_bug.json");
const META_TASK: &str = include_str!("fixtures/createmeta_bce_task.json");
const META_SUBTASK: &str = include_str!("fixtures/createmeta_bce_subtask.json");
const EDIT_BUG: &str = include_str!("fixtures/edit_bce_bug.json");

/// A mock server with createmeta, editmeta, and `myself`. The write requests have
/// separate mocks in each test.
async fn server_with_meta() -> MockServer {
    let server = MockServer::start().await;
    let get = |p: &str| Mock::given(wiremock::matchers::method("GET")).and(path(p.to_string()));
    get("/rest/api/2/myself").respond_with(json(r#"{"name":"shacohen","displayName":"Sha"}"#)).mount(&server).await;
    get("/rest/api/2/issue/createmeta/BCE/issuetypes").respond_with(json(TYPES)).mount(&server).await;
    get("/rest/api/2/issue/createmeta/BCE/issuetypes/10102").respond_with(json(META_BUG)).mount(&server).await;
    get("/rest/api/2/issue/createmeta/BCE/issuetypes/10100").respond_with(json(META_TASK)).mount(&server).await;
    get("/rest/api/2/issue/createmeta/BCE/issuetypes/10101").respond_with(json(META_SUBTASK)).mount(&server).await;
    get("/rest/api/2/issue/BCE-63337")
        .and(query_param("expand", "editmeta"))
        .respond_with(json(EDIT_BUG))
        .mount(&server)
        .await;
    server
}

fn writes(reqs: &[wiremock::Request]) -> Vec<&wiremock::Request> {
    reqs.iter().filter(|r| r.method.as_str() != "GET").collect()
}

fn drafts(ctx: &Ctx) -> Vec<std::path::PathBuf> {
    let dir = ctx.dir.path().join("state/jui/drafts");
    match std::fs::read_dir(dir) {
        Ok(d) => d.flatten().map(|e| e.path()).collect(),
        Err(_) => vec![],
    }
}

impl Ctx {
    /// `jui` with a known Draft directory.
    fn jui_w(&self) -> Command {
        let mut c = self.jui();
        c.env("XDG_STATE_HOME", self.dir.path().join("state"));
        c
    }
}

#[tokio::test]
async fn new_bug_with_flags_posts_names_as_ids() {
    let server = server_with_meta().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue"))
        .respond_with(ResponseTemplate::new(201).set_body_raw(r#"{"id":"1","key":"BCE-70001"}"#, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx
        .jui_w()
        .args(["new", "-t", "bug", "-s", "Scanner stops", "-c", "DCO-Flow", "-d", "**Bold** text", "-a", "me"])
        .args(["--set", "severity=crit", "--set", "detected_in_production=no", "--set", "Regression=Yes"])
        .args(["--set", "affects_versions=unified-pc-february-HF3"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "BCE-70001");
    let reqs = server.received_requests().await.unwrap();
    let w = writes(&reqs);
    assert_eq!(w.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&w[0].body).unwrap();
    let f = &body["fields"];
    assert_eq!(f["project"], serde_json::json!({"key": "BCE"}));
    assert_eq!(f["issuetype"], serde_json::json!({"id": "10102"}));
    assert_eq!(f["customfield_11908"], serde_json::json!({"id": "44419"}));
    assert_eq!(f["reporter"], serde_json::json!({"name": "shacohen"}));
    assert_eq!(f["assignee"], serde_json::json!({"name": "shacohen"}));
    assert_eq!(f["description"], serde_json::json!("*Bold* text"));
    assert_eq!(f["versions"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn new_in_a_script_with_missing_fields_stops_with_a_draft() {
    let server = server_with_meta().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["new", "-t", "Bug", "-s", "Only a summary"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("severity: a value is required."), "{err}");
    assert!(err.contains("components: a value is required."), "{err}");
    assert!(err.contains("jui new --from"), "{err}");
    let d = drafts(&ctx);
    assert_eq!(d.len(), 1, "one Draft");
    assert!(std::fs::read_to_string(&d[0]).unwrap().contains("summary = \"Only a summary\""));
    assert!(writes(&server.received_requests().await.unwrap()).is_empty(), "no write request");
}

#[tokio::test]
async fn new_without_summary_in_a_script_needs_flags_or_from() {
    let server = server_with_meta().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["new"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("jui needs a terminal to open the Template."));
}

#[tokio::test]
async fn new_from_a_file_and_unknown_type() {
    let server = server_with_meta().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue"))
        .respond_with(ResponseTemplate::new(201).set_body_raw(r#"{"key":"BCE-70002"}"#, "application/json"))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let file = ctx.dir.path().join("t.md");
    std::fs::write(&file, "+++\nproject = \"BCE\"\ntype = \"Task\"\nsummary = \"From a file\"\ncomponents = [\"DCO-Flow\"]\n+++\n\nText\n").unwrap();
    let out = ctx.jui_w().args(["new", "--from"]).arg(&file).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let out = ctx.jui_w().args(["new", "-t", "Spike", "-s", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Project BCE has no Ticket type `Spike`."), "{err}");
    assert!(err.contains("Epic, Story, Task, Sub-task, Bug, Initiative"), "{err}");
}

#[tokio::test]
async fn new_with_parent_creates_a_subtask() {
    let server = server_with_meta().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue"))
        .respond_with(ResponseTemplate::new(201).set_body_raw(r#"{"key":"BCE-70003"}"#, "application/json"))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["new", "--parent", "62886", "-s", "Child", "-c", "DCO-Flow"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body["fields"]["parent"], serde_json::json!({"key": "BCE-62886"}));
    assert_eq!(body["fields"]["issuetype"], serde_json::json!({"id": "10101"}));
}

#[tokio::test]
async fn new_rejected_by_jira_in_a_script_writes_a_draft() {
    let server = server_with_meta().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            r#"{"errorMessages":[],"errors":{"customfield_10021":"Number value expected."}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["new", "-s", "x", "-c", "DCO-Flow"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("story_points: Number value expected."), "{err}");
    assert_eq!(drafts(&ctx).len(), 1);
}

#[tokio::test]
async fn edit_with_flags_puts_only_the_changed_fields() {
    let server = server_with_meta().await;
    Mock::given(wiremock::matchers::method("PUT"))
        .and(path("/rest/api/2/issue/BCE-63337"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["edit", "63337", "--set", "severity=minor", "-l", "a", "-l", "b"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Saved BCE-63337. Changed: labels, severity."));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body, serde_json::json!({"fields": {"customfield_11908": {"id": "44422"}, "labels": ["a", "b"]}}));
}

#[tokio::test]
async fn edit_with_the_same_values_sends_nothing() {
    let server = server_with_meta().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["edit", "BCE-63337", "-s", "Scanner stops on an empty file"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("No changes."));
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());
}

#[tokio::test]
async fn edit_of_the_description_in_a_script_needs_yes() {
    let server = server_with_meta().await;
    Mock::given(wiremock::matchers::method("PUT"))
        .and(path("/rest/api/2/issue/BCE-63337"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["edit", "BCE-63337", "-d", "New text"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("-h3. Steps"), "the diff shows the old wiki text: {err}");
    assert!(err.contains("+New text"), "{err}");
    assert!(err.contains("Use --yes"), "{err}");
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());

    let out = ctx.jui_w().args(["edit", "BCE-63337", "-d", "New text", "--yes"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body, serde_json::json!({"fields": {"description": "New text"}}));
}

#[tokio::test]
async fn edit_stops_when_the_ticket_changed_in_jira() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/myself")).respond_with(json(r#"{"name":"shacohen"}"#)).mount(&server).await;
    // The first read gives the fixture. The second read (before the PUT) gives a newer Ticket.
    Mock::given(path("/rest/api/2/issue/BCE-63337"))
        .and(wiremock::matchers::method("GET"))
        .respond_with(json(EDIT_BUG))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let newer = EDIT_BUG
        .replace("2026-10-19T10:00:00.000-0700", "2026-10-19T11:00:00.000-0700")
        .replace("Scanner stops on an empty file", "Changed by a colleague");
    Mock::given(path("/rest/api/2/issue/BCE-63337"))
        .and(wiremock::matchers::method("GET"))
        .respond_with(json(&newer))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["edit", "BCE-63337", "--set", "severity=minor"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("BCE-63337 changed in Jira while you edited it. Changed: Summary."), "{err}");
    assert_eq!(drafts(&ctx).len(), 1);
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());
}

#[tokio::test]
async fn edit_wiki_from_a_file_is_not_converted() {
    let server = server_with_meta().await;
    Mock::given(wiremock::matchers::method("PUT"))
        .and(path("/rest/api/2/issue/BCE-63337"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let file = ctx.dir.path().join("w.md");
    std::fs::write(&file, "+++\nsummary = \"Scanner stops on an empty file\"\n+++\n\nh2. *New*\n").unwrap();
    let out = ctx.jui_w().args(["edit", "BCE-63337", "--wiki", "--yes", "--from"]).arg(&file).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body, serde_json::json!({"fields": {"description": "h2. *New*"}}));
}

#[test]
fn config_edit_without_a_terminal_exits_2() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    let out = ctx.jui().args(["config", "edit"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("needs a terminal"));
}

// ------------------------------------------------------------------ comments

const COMMENT_7002: &str = r#"{"id":"7002","author":{"name":"jdoe","displayName":"Jane Doe"},"body":"Done. See {{retry.rs}}.","created":"2026-10-13T09:00:00.000-0700","updated":"2026-10-13T09:00:00.000-0700"}"#;

/// A mock server with the Ticket (summary and comments) and comment 7002.
async fn server_with_comments() -> MockServer {
    let server = MockServer::start().await;
    let get = |p: &str| Mock::given(wiremock::matchers::method("GET")).and(path(p.to_string()));
    get("/rest/api/2/issue/BCE-62886").respond_with(json(ISSUE)).mount(&server).await;
    get("/rest/api/2/issue/BCE-62886/comment/7002").respond_with(json(COMMENT_7002)).mount(&server).await;
    get("/rest/api/2/issue/BCE-62886/comment/1")
        .respond_with(ResponseTemplate::new(404).set_body_raw(r#"{"errorMessages":["Can not find a comment for the id: 1."],"errors":{}}"#, "application/json"))
        .mount(&server)
        .await;
    server
}

fn created_comment(body: &str) -> ResponseTemplate {
    let v = serde_json::json!({"id": "7003", "author": {"name": "shacohen"}, "body": body,
        "created": "2026-10-19T09:00:00.000-0700", "updated": "2026-10-19T09:00:00.000-0700"});
    ResponseTemplate::new(201).set_body_raw(v.to_string(), "application/json")
}

#[tokio::test]
async fn comment_list_reads_all_pages_and_shows_ids() {
    let server = MockServer::start().await;
    let page = |start: u64, ids: &[&str]| {
        let comments: Vec<_> = ids.iter().map(|id| serde_json::json!({"id": id, "author": {"name": "jdoe"}, "body": format!("Text {id}"), "created": "2026-10-13T09:00:00.000-0700"})).collect();
        json(&serde_json::json!({"startAt": start, "maxResults": 2, "total": 3, "comments": comments}).to_string())
    };
    Mock::given(path("/rest/api/2/issue/BCE-1/comment")).and(query_param("startAt", "0")).respond_with(page(0, &["1", "2"])).mount(&server).await;
    Mock::given(path("/rest/api/2/issue/BCE-1/comment")).and(query_param("startAt", "2")).respond_with(page(2, &["3"])).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["comment", "list", "1"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("Comments (3)"), "{text}");
    assert!(text.contains("@jdoe  #3"), "{text}");
    assert!(text.contains("    Text 3"), "{text}");

    let out = ctx.jui().args(["comment", "ls", "BCE-1", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3);
    assert_eq!(v[0]["body"]["markdown"], "Text 1");
}

#[tokio::test]
async fn comment_add_with_text_converts_markdown_and_mentions() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue/BCE-62886/comment"))
        .respond_with(created_comment("x"))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "add", "62886", "Hi @jdoe, see **this** and `@code`."]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "7003");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Added comment 7003 to BCE-62886."));
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1, "a text needs no other request");
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body, serde_json::json!({"body": "Hi [~jdoe], see *this* and {{@code}}."}));
}

#[tokio::test]
async fn comment_add_from_stdin_and_wiki() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue/BCE-62886/comment"))
        .respond_with(created_comment("x"))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "add", "BCE-62886", "-", "--wiki"]).write_stdin("h2. **raw**\n\n").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["body"], "h2. **raw**");
}

#[tokio::test]
async fn comment_add_empty_or_without_a_terminal() {
    let server = MockServer::start().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "add", "BCE-62886", "  "]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Cancelled. No comment added."));
    let out = ctx.jui_w().args(["comment", "add", "BCE-62886"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("jui needs a terminal to open the comment Template."));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn comment_add_from_a_file_ignores_the_reference_part() {
    let server = server_with_comments().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue/BCE-62886/comment"))
        .respond_with(created_comment("x"))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let file = ctx.dir.path().join("c.md");
    std::fs::write(&file, "# ERROR: old error\nFrom a *file*.\n\n# ------------------------ >8 ------------------------\n# @rroe\n#   Old text\n").unwrap();
    let out = ctx.jui_w().args(["comment", "add", "BCE-62886", "--from"]).arg(&file).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body["body"], "From a _file_.");
}

#[tokio::test]
async fn comment_add_rejected_in_a_script_writes_a_draft() {
    let server = server_with_comments().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue/BCE-62886/comment"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(r#"{"errorMessages":[],"errors":{"comment":"Comment body can not be empty!"}}"#, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let file = ctx.dir.path().join("c.md");
    std::fs::write(&file, "Text\n").unwrap();
    let out = ctx.jui_w().args(["comment", "add", "BCE-62886", "--from"]).arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("comment: Comment body can not be empty!"), "{err}");
    assert!(err.contains("jui comment add BCE-62886 --from"), "{err}");
    let d = drafts(&ctx);
    assert_eq!(d.len(), 1);
    assert!(d[0].file_name().unwrap().to_string_lossy().contains("BCE-62886-comment"));
}

#[tokio::test]
async fn comment_edit_unchanged_sends_nothing() {
    let server = server_with_comments().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "edit", "BCE-62886", "7002", "Done. See `retry.rs`."]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("No changes."));
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());
}

#[tokio::test]
async fn comment_edit_shows_the_diff_and_needs_yes_in_a_script() {
    let server = server_with_comments().await;
    Mock::given(wiremock::matchers::method("PUT"))
        .and(path("/rest/api/2/issue/BCE-62886/comment/7002"))
        .respond_with(json(COMMENT_7002))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "edit", "BCE-62886", "7002", "Done. See `retry.rs` and @jdoe."]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("-Done. See {{retry.rs}}."), "{err}");
    assert!(err.contains("+Done. See {{retry.rs}} and [~jdoe]."), "{err}");
    assert!(err.contains("Use --yes"), "{err}");
    assert_eq!(drafts(&ctx).len(), 1, "the text of the user goes to a Draft");
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());

    let out = ctx.jui_w().args(["comment", "edit", "BCE-62886", "7002", "--yes", "Done. See `retry.rs` and @jdoe."]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Saved comment 7002 on BCE-62886."));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body, serde_json::json!({"body": "Done. See {{retry.rs}} and [~jdoe]."}));
}

#[tokio::test]
async fn comment_edit_wiki_is_not_converted() {
    let server = server_with_comments().await;
    Mock::given(wiremock::matchers::method("PUT"))
        .and(path("/rest/api/2/issue/BCE-62886/comment/7002"))
        .respond_with(json(COMMENT_7002))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "edit", "BCE-62886", "7002", "--wiki", "-y", "Done. See {{retry.rs}}. **Not bold**"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body["body"], "Done. See {{retry.rs}}. **Not bold**");
}

#[tokio::test]
async fn comment_edit_stops_when_the_comment_changed_in_jira() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/issue/BCE-62886")).respond_with(json(ISSUE)).mount(&server).await;
    let newer = COMMENT_7002.replace("Done. See {{retry.rs}}.", "Changed by jdoe.");
    Mock::given(path("/rest/api/2/issue/BCE-62886/comment/7002")).respond_with(json(&newer)).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "edit", "BCE-62886", "7002", "-y", "My text"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Comment 7002 on BCE-62886 changed in Jira while you edited it."), "{err}");
    assert_eq!(drafts(&ctx).len(), 1);
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());
}

#[tokio::test]
async fn comment_edit_of_an_unknown_comment_exits_4() {
    let server = server_with_comments().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "edit", "BCE-62886", "1", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Comment 1 on BCE-62886 not found."));
}

#[tokio::test]
async fn comment_delete_needs_yes_in_a_script() {
    let server = server_with_comments().await;
    Mock::given(wiremock::matchers::method("DELETE"))
        .and(path("/rest/api/2/issue/BCE-62886/comment/7002"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["comment", "delete", "BCE-62886", "7002"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("jui must ask before it deletes comment 7002 on BCE-62886."), "{err}");
    assert!(err.contains("Use --yes"), "{err}");
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());

    let out = ctx.jui_w().args(["comment", "rm", "BCE-62886", "7002", "--yes"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Deleted comment 7002 on BCE-62886."));
}

// ---------------------------------------------------------------- assign and watch

async fn server_with_users() -> MockServer {
    let server = MockServer::start().await;
    let users = |body: &'static str| ResponseTemplate::new(200).set_body_raw(body, "application/json");
    Mock::given(path("/rest/api/2/user/assignable/search"))
        .and(query_param("username", "jane"))
        .respond_with(users(r#"[{"name":"jdoe","displayName":"Jane Doe","active":true},{"name":"jroe","displayName":"Jane Roe","active":true}]"#))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/user/assignable/search"))
        .and(query_param("username", "jdoe"))
        .respond_with(users(r#"[{"name":"jdoe","displayName":"Jane Doe","active":true},{"name":"jdoe2","displayName":"J Doe","active":true}]"#))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/user/assignable/search"))
        .and(query_param("username", "zz"))
        .respond_with(users("[]"))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/user/search"))
        .and(query_param("username", "roe"))
        .respond_with(users(r#"[{"name":"jroe","displayName":"Jane Roe","active":true}]"#))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/myself"))
        .respond_with(users(r#"{"name":"shacohen","displayName":"Sha Cohen"}"#))
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::method("PUT"))
        .and(path("/rest/api/2/issue/BCE-1/assignee"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    server
}

fn put_bodies(reqs: &[wiremock::Request]) -> Vec<serde_json::Value> {
    writes(reqs).iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect()
}

#[tokio::test]
async fn assign_me_none_and_an_exact_username() {
    let server = server_with_users().await;
    let ctx = Ctx::new(&server.uri());
    for (arg, msg) in [("me", "Assigned BCE-1 to you."), ("none", "BCE-1 is now unassigned."), ("jdoe", "Assigned BCE-1 to Jane Doe (jdoe).")] {
        let out = ctx.jui().args(["assign", "1", arg]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stderr).contains(msg), "{}", String::from_utf8_lossy(&out.stderr));
    }
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        put_bodies(&reqs),
        [serde_json::json!({"name": "shacohen"}), serde_json::json!({"name": null}), serde_json::json!({"name": "jdoe"})]
    );
    // `me` and `none` send no user search.
    let searches = reqs.iter().filter(|r| r.url.path().contains("/user/")).count();
    assert_eq!(searches, 1);
}

#[tokio::test]
async fn assign_with_more_matches_stops_in_a_script_and_no_match_is_an_error() {
    let server = server_with_users().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["ticket", "assign", "BCE-1", "jane"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error: `jane` matches 2 users."), "{err}");
    assert!(err.contains("hint: Use the username. Matches: Jane Doe (jdoe), Jane Roe (jroe)."), "{err}");

    let out = ctx.jui().args(["assign", "BCE-1", "zz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("No assignable user for BCE-1 matches `zz`."));

    let out = ctx.jui().args(["assign", "BCE-1", "jane", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["kind"], "input");
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());
}

#[tokio::test]
async fn watch_unwatch_and_watchers() {
    let server = server_with_users().await;
    Mock::given(path("/rest/api/2/issue/BCE-1/watchers"))
        .and(wiremock::matchers::method("GET"))
        .respond_with(json(r#"{"isWatching":false,"watchCount":1,"watchers":[{"name":"jroe","displayName":"Jane Roe","active":true}]}"#))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/issue/BCE-1/watchers"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());

    let out = ctx.jui().args(["ticket", "watch", "BCE-1"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("You now watch BCE-1."));
    let out = ctx.jui().args(["ticket", "watch", "BCE-1", "roe"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("Jane Roe (jroe) now watches BCE-1."));
    let out = ctx.jui().args(["ticket", "unwatch", "BCE-1"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("You no longer watch BCE-1."));
    let out = ctx.jui().args(["ticket", "watch", "BCE-1", "none"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));

    let out = ctx.jui().args(["ticket", "watchers", "BCE-1"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "jroe\tJane Roe\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 watcher on BCE-1."));
    let out = ctx.jui().args(["ticket", "watchers", "BCE-1", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v, serde_json::json!({"key": "BCE-1", "watching": false, "count": 1, "users": [{"name": "jroe", "display_name": "Jane Roe"}]}));

    let reqs = server.received_requests().await.unwrap();
    let w = writes(&reqs);
    assert_eq!(w.iter().map(|r| r.method.as_str()).collect::<Vec<_>>(), ["POST", "POST", "DELETE"]);
    assert!(w[0].body.is_empty());
    assert_eq!(w[1].body, b"\"jroe\"");
    assert_eq!(w[2].url.query(), Some("username=shacohen"));
}

#[tokio::test]
async fn edit_a_with_a_search_text_uses_the_one_matching_user() {
    let server = server_with_meta().await;
    Mock::given(path("/rest/api/2/user/assignable/search"))
        .and(query_param("issueKey", "BCE-63337"))
        .and(query_param("username", "roe"))
        .respond_with(json(r#"[{"name":"jroe","displayName":"Jane Roe","active":true}]"#))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/user/assignable/search"))
        .and(query_param("project", "BCE"))
        .and(query_param("username", "zz"))
        .respond_with(json("[]"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::method("PUT"))
        .and(path("/rest/api/2/issue/BCE-63337"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["edit", "BCE-63337", "-a", "roe"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body["fields"]["assignee"], serde_json::json!({"name": "jroe"}));

    // `jui new` searches in the project. No match stops before the create screen.
    let out = ctx.jui_w().args(["new", "-s", "S", "-a", "zz"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("No assignable user in project BCE matches `zz`."));
}

// ---------------------------------------------------------------- saved filters

async fn server_with_filters(favourites: &'static str) -> MockServer {
    let server = server_with_tickets().await;
    Mock::given(path("/rest/api/2/filter/favourite"))
        .respond_with(json(favourites))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/filter/117549"))
        .respond_with(json(include_str!("fixtures/filter_one.json")))
        .mount(&server)
        .await;
    server
}

const FAVS: &str = include_str!("fixtures/filter_favourites.json");

fn jqls(reqs: &[wiremock::Request]) -> Vec<String> {
    reqs.iter()
        .filter(|r| r.url.path() == "/rest/api/2/search")
        .filter_map(|r| r.url.query_pairs().find(|(k, _)| k == "jql").map(|(_, v)| v.to_string()))
        .collect()
}

#[tokio::test]
async fn filter_list_shows_the_favourites_and_json() {
    let server = server_with_filters(FAVS).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["filter", "list"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.lines().next().unwrap().trim_start().starts_with("ID"), "{text}");
    assert!(text.contains("201") && text.contains("BCE bugs") && text.contains("My open work"), "{text}");

    let out = ctx.jui().args(["filter", "ls", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3);
    assert_eq!(v[2]["jql"], "assignee = currentUser() AND statusCategory != Done");
    assert_eq!(v[0]["owner"], "shacohen");
}

#[tokio::test]
async fn filter_list_with_no_favourites_tells_what_to_do() {
    let server = server_with_filters("[]").await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["filter", "list"]).output().unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("You have no favourite filters."), "{err}");
    assert!(err.contains("hint: Give the filter ID"), "{err}");
    let out = ctx.jui().args(["filter", "list", "--json"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "[]");
}

#[tokio::test]
async fn filter_view_by_id_or_name() {
    let server = server_with_filters(FAVS).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["filter", "view", "117549"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("117549  Filter for BCE board\nOwner      jdoe\nFavourite  no\n"), "{text}");
    assert!(text.trim_end().ends_with("project = BCE ORDER BY Rank ASC"), "{text}");

    // A name searches only the favourite filters: no request for the ID.
    let out = ctx.jui().args(["filter", "view", "my open", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["id"], "203");
}

#[tokio::test]
async fn filter_name_with_more_matches_or_no_match_stops_in_a_script() {
    let server = server_with_filters(FAVS).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["filter", "run", "bce"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error: `bce` matches 2 favourite filters."), "{err}");
    assert!(err.contains("hint: Use the filter ID. Matches: BCE bugs (201), BCE bugs (all) (202)."), "{err}");

    let out = ctx.jui().args(["filter", "view", "zz", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["kind"], "input");
    assert!(jqls(&server.received_requests().await.unwrap()).is_empty(), "no search runs");
}

#[tokio::test]
async fn filter_run_uses_the_filter_jql() {
    let server = server_with_filters(FAVS).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["filter", "run", "117549", "--all"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("BCE-62886"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Filter 117549: Filter for BCE board"));

    // The exact name wins over the other name that starts with it.
    let out = ctx.jui().args(["filter", "run", "BCE bugs", "--json", "--limit", "2"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("Filter 201"), "--json writes no filter line");

    assert_eq!(
        jqls(&server.received_requests().await.unwrap()),
        ["project = BCE ORDER BY Rank ASC", "project = BCE ORDER BY Rank ASC", "project = BCE AND type = Bug AND resolution = Unresolved"]
    );
}

#[tokio::test]
async fn filter_view_of_an_unknown_id_exits_4() {
    let server = server_with_filters(FAVS).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["filter", "view", "9"]).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
}

// ------------------------------------------------------------------ Boards

const BOARDS_BCE: &str = include_str!("fixtures/boards_bce.json");
const BOARD_CONFIG_SCRUM: &str = include_str!("fixtures/board_config_scrum.json");
const BOARD_SPRINTS: &str = include_str!("fixtures/board_sprints_active.json");
const BOARD_SEARCH: &str = include_str!("fixtures/board_search.json");

fn my_boards(ctx: &Ctx) -> Vec<(u64, Option<String>)> {
    let path = ctx.config();
    let s = jui::core::Settings::parse(&std::fs::read_to_string(&path).unwrap_or_default(), &path).unwrap();
    s.my_boards.into_iter().map(|b| (b.id, b.name)).collect()
}

async fn board_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board")).respond_with(json(BOARDS_BCE)).mount(&server).await;
    for (id, name, kind) in [(4166, "BCE board", "scrum"), (4533, "BCE Kanban Board", "kanban")] {
        let body = format!(r#"{{"id":{id},"name":"{name}","type":"{kind}"}}"#);
        Mock::given(path(format!("/rest/agile/1.0/board/{id}"))).respond_with(json(&body)).mount(&server).await;
    }
    Mock::given(path("/rest/agile/1.0/board/4166/configuration")).respond_with(json(BOARD_CONFIG_SCRUM)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint")).respond_with(json(BOARD_SPRINTS)).mount(&server).await;
    Mock::given(path("/rest/api/2/search")).respond_with(json(BOARD_SEARCH)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/board/9")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    server
}

#[tokio::test]
async fn board_list_marks_my_boards_and_sends_the_filters() {
    let server = board_server().await;
    let ctx = Ctx::new(&server.uri());
    std::fs::write(ctx.config(), "[[my_boards]]\nid = 4166\nname = \"BCE board\"\n").unwrap();
    let out = ctx.jui().args(["board", "list", "-p", "bce", "--type", "scrum", "--name", "bce"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let line = stdout.lines().find(|l| l.contains("4166")).unwrap();
    assert!(line.trim_start().starts_with('*'), "{stdout}");
    assert!(!stdout.lines().find(|l| l.contains("4533")).unwrap().trim_start().starts_with('*'));
    let reqs = server.received_requests().await.unwrap();
    let q: Vec<(String, String)> = reqs[0].url.query_pairs().map(|(k, v)| (k.into(), v.into())).collect();
    assert!(q.contains(&("projectKeyOrId".into(), "BCE".into())), "{q:?}");
    assert!(q.contains(&("type".into(), "scrum".into())));
    assert!(q.contains(&("name".into(), "bce".into())));

    let out = ctx.jui().args(["board", "ls", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[1]["id"], 4166);
    assert_eq!(v[1]["mine"], true);
    assert_eq!(v[0]["mine"], false);
}

#[tokio::test]
async fn board_add_by_id_and_by_name_keeps_the_order_and_the_comments() {
    let server = board_server().await;
    let ctx = Ctx::new(&server.uri());
    std::fs::write(ctx.config(), "# mine\n[display]\ncolor = \"never\"\n").unwrap();
    let out = ctx.jui().args(["board", "add", "4533", "bce board"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Added 4533  BCE Kanban Board to My Boards."), "{err}");
    assert!(err.contains("Added 4166  BCE board to My Boards."));
    assert_eq!(my_boards(&ctx), vec![(4533, Some("BCE Kanban Board".into())), (4166, Some("BCE board".into()))]);
    assert!(std::fs::read_to_string(ctx.config()).unwrap().starts_with("# mine\n"));

    // A second add of the same Board changes nothing.
    let out = ctx.jui().args(["board", "add", "4166"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("4166  BCE board is already in My Boards."));
    assert_eq!(my_boards(&ctx).len(), 2);
    assert!(writes(&server.received_requests().await.unwrap()).is_empty(), "no write request to Jira");
}

#[tokio::test]
async fn board_add_with_more_matches_stops_in_a_script() {
    let server = board_server().await;
    let ctx = Ctx::new(&server.uri());
    // The text "board" is in all 3 names. No name is the same as it, and no name starts with it.
    let out = ctx.jui().args(["board", "add", "board"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("`board` matches 3 Boards."), "{err}");
    assert!(err.contains("hint: Use the Board ID."));
    assert!(my_boards(&ctx).is_empty());
    assert!(!ctx.config().exists(), "no file is written");
}

#[tokio::test]
async fn board_remove_needs_no_token_and_unknown_is_an_error() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    std::fs::write(ctx.config(), "[[my_boards]]\nid = 1\nname = \"One\"\n\n[[my_boards]]\nid = 2\nname = \"Two\"\n").unwrap();
    let out = ctx.jui().env_remove("JIRA_TOKEN").args(["board", "rm", "two"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Removed 2  Two from My Boards."));
    assert_eq!(my_boards(&ctx), vec![(1, Some("One".into()))]);
    let out = ctx.jui().args(["board", "remove", "77"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("`77` is not in My Boards."));
}

#[tokio::test]
async fn board_mine_empty_stops_in_a_script_and_json_is_empty() {
    let server = board_server().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["board", "mine"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error: My Boards is empty."), "{err}");
    assert!(err.contains("hint: Use `jui board add <id>`, or run `jui board mine` on a terminal."));
    let out = ctx.jui().args(["board", "mine", "--json"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "[]");
}

#[tokio::test]
async fn board_mine_shows_not_found_and_updates_the_names() {
    let server = board_server().await;
    let ctx = Ctx::new(&server.uri());
    std::fs::write(ctx.config(), "[[my_boards]]\nid = 4166\nname = \"Old name\"   # keep\n\n[[my_boards]]\nid = 9\nname = \"Gone\"\n").unwrap();
    let out = ctx.jui().args(["board", "mine"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "4166\tBCE board\tscrum\n9\tGone\tnot found\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 Board is not found."));
    // The name changed in Jira: jui updates it and keeps the comment. The missing Board stays.
    let text = std::fs::read_to_string(ctx.config()).unwrap();
    assert!(text.contains("name = \"BCE board\"   # keep"), "{text}");
    assert_eq!(my_boards(&ctx), vec![(4166, Some("BCE board".into())), (9, Some("Gone".into()))]);
    let out = ctx.jui().args(["board", "mine", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[1]["found"], false);
}

#[tokio::test]
async fn board_view_shows_the_columns_and_json() {
    let server = board_server().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["board", "view", "4166"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap().replace(&server.uri(), "http://jira");
    insta::assert_snapshot!("board_view", text);
    let out = ctx.jui().args(["board", "view", "4166", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["board"]["type"], "scrum");
    assert_eq!(v["columns"][1]["tickets"][0]["key"], "BCE-2");
    assert_eq!(v["sprints"][0]["id"], 43198);
    let out = ctx.jui().args(["board", "view", "9", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let e: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["kind"], "not_found");
}

#[tokio::test]
async fn board_view_mine_adds_the_assignee_to_the_jql() {
    let server = board_server().await;
    let ctx = Ctx::new(&server.uri());
    // The TUI Setting does not change the CLI: only the flag does.
    std::fs::write(ctx.config(), "[boards]\nonly_mine = true\n").unwrap();
    ctx.jui().args(["board", "view", "4166", "--json"]).assert().success();
    let mine = |reqs: &[wiremock::Request]| {
        reqs.iter()
            .filter(|r| r.url.path() == "/rest/api/2/search")
            .map(|r| r.url.query_pairs().any(|(k, v)| k == "jql" && v.contains("AND assignee = currentUser() ORDER BY")))
            .collect::<Vec<_>>()
    };
    assert_eq!(mine(&server.received_requests().await.unwrap()), [false]);
    server.reset().await;
    let server2 = board_server().await;
    let ctx = Ctx::new(&server2.uri());
    let out = ctx.jui().args(["board", "view", "4166", "-m"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains(" (only mine)\n"));
    assert_eq!(mine(&server2.received_requests().await.unwrap()), [true]);
    let out = ctx.jui().args(["board", "view", "4166", "--mine", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["only_mine"], true);
}

// ------------------------------------------------------------------ Sprints and Epics

const SPRINTS_OPEN: &str = include_str!("fixtures/board_sprints_open.json");
const SPRINT_ONE: &str = include_str!("fixtures/sprint_one.json");
const EPIC_ONE: &str = include_str!("fixtures/epic_one.json");
const EPIC_SEARCH: &str = include_str!("fixtures/epic_search.json");

#[tokio::test]
async fn sprint_list_shows_open_sprints_active_first_and_json() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint"))
        .and(query_param("state", "active,future"))
        .respond_with(json(SPRINTS_OPEN))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["sprint", "list", "4166"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("ID"), "{text}");
    assert!(lines[1].starts_with("43198  active  2026-10-12  2026-10-25  BCE-26-Sprint-21"), "{text}");
    assert!(lines[3].contains("BCE-26-Sprint-22"), "{text}");
    let out = ctx.jui().args(["sprint", "ls", "4166", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[0]["id"], 43198);
    assert_eq!(v[0]["start"], "2026-10-12T01:07:00-07:00");
}

#[tokio::test]
async fn sprint_view_shows_the_head_and_the_tickets() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/sprint/43198")).respond_with(json(SPRINT_ONE)).mount(&server).await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("jql", "sprint = 43198 ORDER BY Rank ASC"))
        .respond_with(json(PAGE1))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["sprint", "view", "43198", "--limit", "2"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("43198  BCE-26-Sprint-21\nState      active\n"), "{text}");
    assert!(text.contains("Goal       Finish the scanner"));
    assert!(text.contains("BCE-62886"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Showing 2 of 3."));
    let out = ctx.jui().args(["sprint", "view", "43198", "--json", "--limit", "2"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["sprint"]["name"], "BCE-26-Sprint-21");
    assert_eq!(v["total"], 3);
    assert_eq!(v["tickets"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn sprint_active_lists_each_active_sprint() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4166/configuration"))
        .respond_with(json(include_str!("fixtures/board_config_scrum.json")))
        .mount(&server)
        .await;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint"))
        .and(query_param("state", "active"))
        .respond_with(json(include_str!("fixtures/board_sprints_active.json")))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("jql", "sprint = 43198 ORDER BY Rank ASC"))
        .respond_with(json(PAGE1))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["sprint", "active", "4166", "--limit", "2"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Sprint BCE-26-Sprint-21 (43198): 2 of 3 Tickets"), "{err}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("BCE-62886"));
}

#[tokio::test]
async fn sprint_active_and_view_mine_add_the_assignee() {
    const MINE: &str = "(sprint = 43198) AND assignee = currentUser() ORDER BY Rank ASC";
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4166/configuration"))
        .respond_with(json(include_str!("fixtures/board_config_scrum.json")))
        .mount(&server)
        .await;
    Mock::given(path("/rest/agile/1.0/board/4166/sprint"))
        .and(query_param("state", "active"))
        .respond_with(json(include_str!("fixtures/board_sprints_active.json")))
        .mount(&server)
        .await;
    Mock::given(path("/rest/agile/1.0/sprint/43198")).respond_with(json(SPRINT_ONE)).mount(&server).await;
    // Only the query with the assignee gets an answer: the other query gives HTTP 404.
    Mock::given(path("/rest/api/2/search")).and(query_param("jql", MINE)).respond_with(json(PAGE2)).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    // The TUI Setting does not change the CLI.
    std::fs::write(ctx.config(), "[sprint]\nonly_mine = true\n").unwrap();

    let out = ctx.jui().args(["sprint", "active", "4166", "--mine"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    // The mock gives the same page for each `startAt`, so jui gets all 3 Tickets.
    assert!(err.contains("Sprint BCE-26-Sprint-21 (43198): 3 Tickets (only mine)"), "{err}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("XSUP-1182"));

    let out = ctx.jui().args(["sprint", "view", "43198", "-m", "--json"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["tickets"][0]["key"], "XSUP-1182");

    // With no flag, jui sends the query with no assignee, also when the Setting is on.
    let out = ctx.jui().args(["sprint", "view", "43198"]).output().unwrap();
    assert!(!out.status.success());
    let reqs = server.received_requests().await.unwrap();
    let last = reqs.iter().rev().find(|r| r.url.path() == "/rest/api/2/search").unwrap();
    assert!(last.url.query_pairs().any(|(k, v)| k == "jql" && v == "sprint = 43198 ORDER BY Rank ASC"));
}

#[tokio::test]
async fn sprint_add_checks_the_sprint_and_backlog_moves() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/sprint/43198")).respond_with(json(SPRINT_ONE)).mount(&server).await;
    let closed = SPRINT_ONE.replace("\"active\"", "\"closed\"").replace("43198", "43197");
    Mock::given(path("/rest/agile/1.0/sprint/43197")).respond_with(json(&closed)).mount(&server).await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/agile/1.0/sprint/43198/issue"))
        .and(wiremock::matchers::body_json(serde_json::json!({ "issues": ["BCE-1", "BCE-2"] })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/agile/1.0/backlog/issue"))
        .and(wiremock::matchers::body_json(serde_json::json!({ "issues": ["XSUP-5"] })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    // A short key gets the default project. A duplicate is removed.
    let out = ctx.jui().args(["sprint", "add", "43198", "1", "BCE-2", "bce-1"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Moved BCE-1, BCE-2 to Sprint BCE-26-Sprint-21."));
    let out = ctx.jui().args(["sprint", "add", "43197", "BCE-1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is closed"));
    let out = ctx.jui().args(["sprint", "backlog", "xsup-5"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Moved XSUP-5 to the backlog."));
    let posts = server.received_requests().await.unwrap().iter().filter(|r| r.method.as_str() == "POST").count();
    assert_eq!(posts, 2, "the closed Sprint gets no POST");
}

#[tokio::test]
async fn epic_list_tickets_set_and_unset() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4166/configuration"))
        .respond_with(json(include_str!("fixtures/board_config_scrum.json")))
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("fields", "summary,status,customfield_10017,customfield_10018"))
        .respond_with(json(EPIC_SEARCH))
        .mount(&server)
        .await;
    Mock::given(path("/rest/agile/1.0/epic/BCE-13070")).respond_with(json(EPIC_ONE)).mount(&server).await;
    Mock::given(path("/rest/agile/1.0/epic/BCE-1")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("jql", "\"Epic Link\" = BCE-13070 ORDER BY Rank ASC"))
        .respond_with(json(PAGE1))
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/agile/1.0/epic/BCE-13070/issue"))
        .and(wiremock::matchers::body_json(serde_json::json!({ "issues": ["BCE-5"] })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/agile/1.0/epic/none/issue"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());

    let out = ctx.jui().args(["epic", "list", "4166", "--done"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.lines().nth(1).unwrap(), "BCE-13070  Proxy - Cider Integration MVP");
    assert!(text.contains("Old epic with no name  (done)"));

    let out = ctx.jui().args(["epic", "tickets", "13070", "--limit", "2"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Epic BCE-13070: Proxy - Cider Integration MVP"));

    let out = ctx.jui().args(["epic", "set", "BCE-1", "BCE-5"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "BCE-1 is not an Epic");
    let out = ctx.jui().args(["epic", "set", "BCE-13070", "BCE-13070"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "an Epic cannot be in itself");
    let out = ctx.jui().args(["epic", "set", "13070", "5"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Set the Epic of BCE-5 to BCE-13070"));
    let out = ctx.jui().args(["epic", "unset", "BCE-5"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Removed the Epic of BCE-5."));
}

#[tokio::test]
async fn epic_mine_counts_my_open_tickets_in_each_epic() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("jql", "assignee = currentUser() AND statusCategory != Done AND \"Epic Link\" is not EMPTY"))
        .and(query_param("fields", "customfield_10016"))
        .respond_with(json(
            r#"{"startAt":0,"total":4,"issues":[
              {"id":"1","key":"BCE-1","fields":{"customfield_10016":"CRTX-9"}},
              {"id":"1","key":"BCE-2","fields":{"customfield_10016":"BCE-7"}},
              {"id":"1","key":"BCE-3","fields":{"customfield_10016":"BCE-7"}},
              {"id":"1","key":"BCE-4","fields":{"customfield_10016":"GONE-1"}}]}"#,
        ))
        .expect(2) // One request for each of the two runs.
        .mount(&server)
        .await;
    Mock::given(path("/rest/api/2/search"))
        .and(query_param("jql", "key in (BCE-7, CRTX-9, GONE-1) ORDER BY updated DESC"))
        .respond_with(json(
            r#"{"startAt":0,"total":2,"issues":[
              {"key":"CRTX-9","fields":{"summary":"Old","status":{"name":"Done","statusCategory":{"key":"done"}},
                "customfield_10017":{"value":"To Do"},"customfield_10018":"Old work"}},
              {"key":"BCE-7","fields":{"summary":"New","status":{"name":"Open","statusCategory":{"key":"new"}},
                "customfield_10017":null,"customfield_10018":"New work"}}]}"#,
        ))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());

    let out = ctx.jui().args(["epic", "mine"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text, "BCE-7     2  New work\nCRTX-9    1  Old work  (done)\n", "the most Tickets first; Done by the status category");

    let out = ctx.jui().args(["epic", "mine", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[0]["key"], "BCE-7");
    assert_eq!(v[0]["my_open_tickets"], 2);
    assert_eq!(v.as_array().unwrap().len(), 2, "an Epic that the search does not give is not shown");
}

#[tokio::test]
async fn sprint_list_of_a_kanban_board_is_an_input_error_and_json_error() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/agile/1.0/board/4533/sprint"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(r#"{"errorMessages":["The board doesn't support sprints."],"errors":{}}"#, "application/json"))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["sprint", "list", "4533", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["kind"], "input");
}

// ---------------------------------------------------------------- Sub-tasks, Links, Worklogs

const LINK_TYPES: &str = include_str!("fixtures/link_types.json");

fn worklog_json(id: &str, secs: u64, started: &str, comment: Option<&str>) -> serde_json::Value {
    let mut w = serde_json::json!({"id": id, "author": {"name": "jdoe", "displayName": "Jane Doe"}, "started": started,
        "created": "2026-10-19T09:00:00.000-0700", "updated": "2026-10-19T09:00:00.000-0700",
        "timeSpent": "x", "timeSpentSeconds": secs});
    if let Some(c) = comment {
        w["comment"] = serde_json::json!(c);
    }
    w
}

async fn server_with_links() -> MockServer {
    let server = MockServer::start().await;
    let get = |p: &str| Mock::given(wiremock::matchers::method("GET")).and(path(p.to_string()));
    get("/rest/api/2/issueLinkType").respond_with(json(LINK_TYPES)).mount(&server).await;
    get("/rest/api/2/issue/BCE-62886").respond_with(json(ISSUE)).mount(&server).await;
    server
}

#[tokio::test]
async fn subtask_list_shows_the_subtasks_and_json() {
    let server = server_with_links().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["subtask", "ls", "62886"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.trim(), "BCE-62890  To Do  Write unit tests");
    let out = ctx.jui().args(["subtask", "list", "BCE-62886", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[0]["key"], "BCE-62890");
    let reqs = server.received_requests().await.unwrap();
    assert!(reqs[0].url.query_pairs().any(|(k, v)| k == "fields" && v == "summary,subtasks"));
}

#[tokio::test]
async fn subtask_create_is_new_with_parent() {
    let server = server_with_meta().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue"))
        .respond_with(ResponseTemplate::new(201).set_body_raw(r#"{"id":"9","key":"BCE-70001"}"#, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui_w().args(["subtask", "create", "62886", "-s", "Child", "-c", "DCO-Flow"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "BCE-70001");
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body["fields"]["parent"], serde_json::json!({"key": "BCE-62886"}));
}

#[tokio::test]
async fn link_list_and_types() {
    let server = server_with_links().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["link", "list", "62886"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("501  blocks      BCE-63001  To Do  Release checklist"), "{text}");
    assert!(text.contains("502  relates to  BCE-61200  Done   Upload timeouts"), "{text}");
    let out = ctx.jui().args(["link", "types"]).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Blocks         blocks         is blocked by"), "{text}");
}

#[tokio::test]
async fn link_add_sends_the_outward_ticket_as_the_inward_issue() {
    let server = server_with_links().await;
    Mock::given(wiremock::matchers::method("POST")).and(path("/rest/api/2/issueLink")).respond_with(ResponseTemplate::new(201)).expect(2).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["link", "add", "62886", "blocks", "BCE-70000", "-m", "**Why**"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Added the Link: BCE-62886 blocks BCE-70000."));
    let out = ctx.jui().args(["link", "add", "62886", "is blocked", "70000"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let w = writes(&reqs);
    let b0: serde_json::Value = serde_json::from_slice(&w[0].body).unwrap();
    assert_eq!(b0, serde_json::json!({"type": {"name": "Blocks"}, "inwardIssue": {"key": "BCE-62886"}, "outwardIssue": {"key": "BCE-70000"}, "comment": {"body": "*Why*"}}));
    let b1: serde_json::Value = serde_json::from_slice(&w[1].body).unwrap();
    assert_eq!(b1["inwardIssue"]["key"], "BCE-70000");
    assert_eq!(b1["outwardIssue"]["key"], "BCE-62886");
}

#[tokio::test]
async fn link_add_existing_unknown_and_ambiguous_send_nothing() {
    let server = server_with_links().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["link", "add", "62886", "blocks", "63001"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("The Link exists: BCE-62886 blocks BCE-63001."));
    let out = ctx.jui().args(["link", "add", "62886", "fixes", "63001"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("No Link relation matches `fixes`."));
    let out = ctx.jui().args(["link", "add", "62886", "is parent of", "63001"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "more matches stop in a script");
    assert!(String::from_utf8_lossy(&out.stderr).contains("matches more than one"));
    let out = ctx.jui().args(["link", "add", "62886", "blocks", "62886"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(writes(&server.received_requests().await.unwrap()).is_empty());
}

#[tokio::test]
async fn link_delete_by_key_needs_yes_in_a_script() {
    let server = server_with_links().await;
    Mock::given(wiremock::matchers::method("DELETE")).and(path("/rest/api/2/issueLink/502")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["link", "delete", "62886", "BCE-61200"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Use --yes"));
    let out = ctx.jui().args(["link", "rm", "62886", "BCE-1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("has no Link `BCE-1`"));
    let out = ctx.jui().args(["link", "rm", "62886", "61200", "--yes"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "61200 is not an ID or a full key");
    let out = ctx.jui().args(["link", "rm", "62886", "BCE-61200", "--yes"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Deleted the Link: BCE-62886 relates to BCE-61200."));
}

#[tokio::test]
async fn worklog_list_shows_ids_and_total() {
    let server = MockServer::start().await;
    let body = serde_json::json!({"startAt": 0, "maxResults": 2, "total": 2, "worklogs": [
        worklog_json("300", 5400, "2026-10-12T10:00:00.000-0700", Some("Investigation")),
        worklog_json("301", 1800, "2026-10-13T11:00:00.000-0700", None)]});
    Mock::given(path("/rest/api/2/issue/BCE-1/worklog")).respond_with(json(&body.to_string())).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["worklog", "ls", "1"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("300  2026-10-12 10:00  1h 30m  @jdoe  Investigation"), "{text}");
    assert!(text.contains("301  2026-10-13 11:00  30m     @jdoe"), "{text}");
    assert!(text.contains("Total: 2h in 2 Worklogs"), "{text}");
}

#[tokio::test]
async fn worklog_add_with_a_time_posts_at_once() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue/BCE-1/worklog"))
        .respond_with(ResponseTemplate::new(201).set_body_raw(worklog_json("302", 5400, "2026-10-19T10:00:00.000-0700", None).to_string(), "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().env("TZ", "America/Los_Angeles")
        .args(["worklog", "add", "1", "1h30m", "-s", "2026-10-19 10:00", "-m", "Review of **PR**", "--adjust", "new", "--new-estimate", "2h"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "302");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Added Worklog 302 to BCE-1: 1h 30m on 2026-10-19 10:00."));
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1, "a time needs no other request");
    let q: Vec<(String, String)> = reqs[0].url.query_pairs().map(|(k, v)| (k.into(), v.into())).collect();
    assert_eq!(q, [("adjustEstimate".to_string(), "new".to_string()), ("newEstimate".to_string(), "2h".to_string())]);
    let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body, serde_json::json!({"timeSpentSeconds": 5400, "started": "2026-10-19T10:00:00.000-0700", "comment": "Review of *PR*"}));
}

#[tokio::test]
async fn worklog_add_bad_input_and_no_terminal() {
    let server = MockServer::start().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["worklog", "add", "1", "1 hour"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is not a time"));
    let out = ctx.jui().args(["worklog", "add", "1", "1h", "--adjust", "new"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--new-estimate"));
    let out = ctx.jui().args(["worklog", "add", "1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("needs a terminal"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn worklog_add_from_a_file_and_rejected_writes_a_draft() {
    let server = MockServer::start().await;
    let ctx_body = serde_json::json!({"key": "BCE-1", "fields": {"summary": "S", "timetracking": {}, "worklog": {"total": 0, "worklogs": []}}});
    Mock::given(wiremock::matchers::method("GET")).and(path("/rest/api/2/issue/BCE-1")).respond_with(json(&ctx_body.to_string())).mount(&server).await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue/BCE-1/worklog"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(r#"{"errorMessages":[],"errors":{"timeLogged":"Too much."}}"#, "application/json"))
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let file = ctx.dir.path().join("w.md");
    std::fs::write(&file, "+++\ntime_spent = \"2h\"\nstarted = \"2026-10-19 10:00\"\n+++\nText\n# ------------------------ >8 ------------------------\nignored\n").unwrap();
    let out = ctx.jui_w().env("TZ", "America/Los_Angeles").args(["worklog", "add", "1", "--from", file.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("time_spent: Too much."), "{err}");
    assert!(err.contains("jui worklog add BCE-1 --from"), "{err}");
    assert_eq!(drafts(&ctx).len(), 1);
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&writes(&reqs)[0].body).unwrap();
    assert_eq!(body, serde_json::json!({"timeSpentSeconds": 7200, "started": "2026-10-19T10:00:00.000-0700", "comment": "Text"}));
}

#[tokio::test]
async fn worklog_edit_flags_change_only_the_given_fields() {
    let server = MockServer::start().await;
    let w = worklog_json("300", 3600, "2026-10-12T10:00:00.000-0700", Some("Old"));
    Mock::given(wiremock::matchers::method("GET")).and(path("/rest/api/2/issue/BCE-1/worklog/300")).respond_with(json(&w.to_string())).mount(&server).await;
    Mock::given(wiremock::matchers::method("PUT")).and(path("/rest/api/2/issue/BCE-1/worklog/300")).respond_with(json(&w.to_string())).expect(2).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["worklog", "edit", "1", "300", "2h"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = ctx.jui().args(["worklog", "edit", "1", "300", "1h"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("No changes."));
    // A new comment shows the diff and needs --yes in a script.
    let out = ctx.jui().args(["worklog", "edit", "1", "300", "-m", "New"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("-Old"));
    let out = ctx.jui().args(["worklog", "edit", "1", "300", "-m", "New", "--yes"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    let w = writes(&reqs);
    let b0: serde_json::Value = serde_json::from_slice(&w[0].body).unwrap();
    assert_eq!(b0, serde_json::json!({"timeSpentSeconds": 7200, "started": "2026-10-12T10:00:00.000-0700", "comment": "Old"}));
    assert!(w[0].url.query().unwrap().contains("adjustEstimate=auto"));
    let b1: serde_json::Value = serde_json::from_slice(&w[1].body).unwrap();
    assert_eq!(b1["comment"], "New");
}

#[tokio::test]
async fn worklog_delete_needs_yes_and_sends_increase_by() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::method("DELETE")).and(path("/rest/api/2/issue/BCE-1/worklog/300")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["worklog", "delete", "1", "300"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Use --yes"));
    let out = ctx.jui().args(["worklog", "rm", "1", "300", "--adjust", "manual"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--increase-by"));
    let out = ctx.jui().args(["worklog", "rm", "1", "300", "--adjust", "manual", "--increase-by", "30m", "-y"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs[0].url.query(), Some("adjustEstimate=manual&increaseBy=30m"));
}

// ------------------------------------------------------------------ attachments

fn attachment_json(server: &str, id: &str, name: &str, size: u64) -> serde_json::Value {
    serde_json::json!({
        "id": id, "filename": name, "size": size, "mimeType": "text/plain",
        "author": {"name": "jdoe", "displayName": "Jane Doe"}, "created": "2026-10-11T09:00:00.000-0700",
        "content": format!("{server}/secure/attachment/{id}/{name}")
    })
}

/// BCE-1 has three attachments: two files with the same name, and a PNG.
async fn server_with_attachments() -> MockServer {
    let server = MockServer::start().await;
    let u = server.uri();
    let body = serde_json::json!({"key": "BCE-1", "fields": {"summary": "S", "attachment": [
        attachment_json(&u, "900", "trace.log", 14), attachment_json(&u, "901", "trace.log", 3), attachment_json(&u, "902", "screen.png", 2_500_000)]}});
    Mock::given(wiremock::matchers::method("GET")).and(path("/rest/api/2/issue/BCE-1")).respond_with(json(&body.to_string())).mount(&server).await;
    Mock::given(path("/secure/attachment/900/trace.log")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"line 1\nline 2\n".to_vec())).mount(&server).await;
    Mock::given(path("/secure/attachment/901/trace.log")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc".to_vec())).mount(&server).await;
    Mock::given(path("/secure/attachment/902/screen.png")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"PNG".to_vec())).mount(&server).await;
    Mock::given(path("/rest/api/2/attachment/meta")).respond_with(json(r#"{"enabled":true,"uploadLimit":10}"#)).mount(&server).await;
    server
}

#[tokio::test]
async fn attachment_list_shows_ids_and_json() {
    let server = server_with_attachments().await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["attachment", "ls", "1"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("900  trace.log     14 B  2026-10-11 09:00  @jdoe"), "{text}");
    assert!(text.contains("902  screen.png  2.4 MB  2026-10-11 09:00  @jdoe"), "{text}");
    let out = ctx.jui().args(["attachment", "list", "BCE-1", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[2]["filename"], "screen.png");
    assert_eq!(v[2]["size"], 2_500_000);
}

#[tokio::test]
async fn attachment_download_by_id_name_all_and_stdout() {
    let server = server_with_attachments().await;
    let ctx = Ctx::new(&server.uri());
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_str().unwrap();

    // A name that matches two attachments stops in a script.
    let out = ctx.jui().args(["attachment", "download", "1", "trace.log", "-o", d]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("2 attachments of BCE-1 match `trace.log`"));

    let out = ctx.jui().args(["attachment", "get", "1", "900", "scr", "-o", d]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read(dir.path().join("trace.log")).unwrap(), b"line 1\nline 2\n");
    assert_eq!(std::fs::read(dir.path().join("screen.png")).unwrap(), b"PNG");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Downloaded trace.log (14 B) to"));

    // The file exists: a script needs --force. Nothing is written.
    let out = ctx.jui().args(["attachment", "get", "1", "--all", "-o", d]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Use --force"));
    // --all with --force: the second trace.log gets the ID in its name.
    let out = ctx.jui().args(["attachment", "get", "1", "--all", "-o", d, "--force"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read(dir.path().join("901-trace.log")).unwrap(), b"abc");
    assert_eq!(std::fs::read(dir.path().join("trace.log")).unwrap(), b"line 1\nline 2\n");

    let out = ctx.jui().args(["attachment", "get", "1", "901", "-o", "-"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"abc");
    // More attachments to stdout or to one file is an error.
    let out = ctx.jui().args(["attachment", "get", "1", "--all", "-o", "-"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    // No name and no --all in a script.
    let out = ctx.jui().args(["attachment", "get", "1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("use --all"));
}

#[tokio::test]
async fn attachment_upload_checks_all_files_first() {
    let server = server_with_attachments().await;
    let resp = serde_json::json!([attachment_json(&server.uri(), "950", "a.txt", 5)]);
    Mock::given(wiremock::matchers::method("POST"))
        .and(path("/rest/api/2/issue/BCE-1/attachments"))
        .and(header("X-Atlassian-Token", "no-check"))
        .respond_with(json(&resp.to_string()))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = Ctx::new(&server.uri());
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("a.txt");
    let big = dir.path().join("big.bin");
    std::fs::write(&small, b"hello").unwrap();
    std::fs::write(&big, [0u8; 11]).unwrap();
    // The second file is too big (the limit is 10 B): nothing is sent.
    let out = ctx.jui().args(["attachment", "upload", "1"]).arg(&small).arg(&big).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("The upload limit of Jira is 10 B."), "{}", String::from_utf8_lossy(&out.stderr));
    let out = ctx.jui().args(["attachment", "upload", "1"]).arg(dir.path().join("none.txt")).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not exist"));
    // One good file: one POST, the new ID on stdout.
    let out = ctx.jui().args(["attachment", "add", "1"]).arg(&small).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "950\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Uploaded a.txt (5 B) to BCE-1."));
}

#[tokio::test]
async fn attachment_delete_needs_yes_in_a_script() {
    let server = server_with_attachments().await;
    Mock::given(wiremock::matchers::method("DELETE")).and(path("/rest/api/2/attachment/902")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    let ctx = Ctx::new(&server.uri());
    let out = ctx.jui().args(["attachment", "delete", "1", "screen.png"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Use --yes"));
    let out = ctx.jui().args(["attachment", "rm", "1", "nothing", "--yes"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("BCE-1 has no attachment `nothing`."));
    let out = ctx.jui().args(["attachment", "rm", "1", "trace", "--yes"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "two matches stop in a script, also with --yes");
    let out = ctx.jui().args(["attachment", "rm", "1", "902", "-y"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Deleted attachment 902 (screen.png) on BCE-1."));
}

// ------------------------------------------------------------------ completions

#[test]
fn completions_need_no_token_no_config_and_no_server() {
    // A bad config file, no token, and a base URL with no server: completion still works.
    let ctx = Ctx::new("http://127.0.0.1:9");
    std::fs::write(ctx.config(), "[jira\n").unwrap();
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let out = ctx.jui().env_remove("JIRA_TOKEN").args(["completions", shell]).output().unwrap();
        assert!(out.status.success(), "{shell}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(!out.stdout.is_empty(), "{shell}");
    }
}

#[test]
fn completions_include_all_commands_and_aliases() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    let out = ctx.jui().args(["completions", "zsh"]).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("#compdef jui"));
    for cmd in [
        "tui", "ls", "search", "view", "mv", "new", "edit", "open", "assign", "ticket", "filter", "board", "sprint", "epic",
        "comment", "subtask", "link", "worklog", "attachment", "config", "completions",
    ] {
        assert!(text.contains(&format!("'{cmd}:")), "no command {cmd}");
    }
    // The subcommands and aliases of the later build tickets.
    for sub in ["upload", "download", "watchers", "backlog", "unset", "types", "init"] {
        assert!(text.contains(&format!("'{sub}:")), "no subcommand {sub}");
    }
    for alias in ["'get:", "'rm:", "'ls:", "'add:"] {
        assert!(text.contains(alias), "no alias {alias}");
    }
    // Fixed values complete too.
    assert!(text.contains("auto") && text.contains("never"), "no --color values");
}

#[test]
fn completions_reject_an_unknown_shell() {
    let ctx = Ctx::new("http://127.0.0.1:9");
    let out = ctx.jui().args(["completions", "tcsh"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("possible values"));
}
