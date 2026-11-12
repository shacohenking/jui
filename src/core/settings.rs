//! Settings: read and write `config.toml`, with default values.
//! See ticket 07 "jui Settings: config file schema and Settings page".

use std::path::{Path, PathBuf};

use serde::Serialize;
use toml_edit::{DocumentMut, Item, Table, value};

use super::error::{Error, Result};

/// A placeholder. Set the real server with `JUI_BASE_URL` or `jira.base_url` in the config file.
pub const DEFAULT_BASE_URL: &str = "https://jira.example.com";
pub const DEFAULT_MY_TICKETS_JQL: &str =
    "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC";
pub const DEFAULT_PROJECT: &str = "BCE";
pub const DEFAULT_TYPE: &str = "Task";
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

pub const ENV_BASE_URL: &str = "JUI_BASE_URL";
pub const ENV_CONFIG: &str = "JUI_CONFIG";
pub const ENV_TOKEN: &str = "JIRA_TOKEN";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    Relative,
    Iso,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardRef {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Where a Setting value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Default,
    File,
    Env,
}

#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    pub base_url: String,
    pub timeout_secs: u64,
    pub my_tickets_jql: String,
    /// My Tickets (the TUI tab and `jui ls`) shows the Tickets in groups by Sprint.
    pub group_by_sprint: bool,
    pub default_project: String,
    pub default_type: String,
    pub editor_command: String,
    pub date_format: DateFormat,
    pub color: ColorMode,
    /// The Boards tab shows only the Tickets that are assigned to the current user.
    pub boards_only_mine: bool,
    /// The Sprint tab shows only the Tickets that are assigned to the current user.
    pub sprint_only_mine: bool,
    pub my_boards: Vec<BoardRef>,
    /// The `jira.base_url` value of the file when `JUI_BASE_URL` replaces it.
    #[serde(skip)]
    pub file_base_url: Option<String>,
    /// The environment that `load` used. `reload` uses it again.
    #[serde(skip)]
    pub env: Env,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    pub sources: Vec<(&'static str, Source)>,
    /// Warnings for values that are not valid or keys that are not known.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            base_url: DEFAULT_BASE_URL.into(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            my_tickets_jql: DEFAULT_MY_TICKETS_JQL.into(),
            group_by_sprint: true,
            default_project: DEFAULT_PROJECT.into(),
            default_type: DEFAULT_TYPE.into(),
            editor_command: String::new(),
            date_format: DateFormat::Relative,
            color: ColorMode::Auto,
            boards_only_mine: false,
            sprint_only_mine: false,
            my_boards: Vec::new(),
            file_base_url: None,
            env: Env::default(),
            path: PathBuf::new(),
            sources: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// All Setting keys that `jui config get/set/unset` accept.
pub const KEYS: &[&str] = &[
    "jira.base_url",
    "jira.timeout_secs",
    "tickets.my_tickets_jql",
    "tickets.group_by_sprint",
    "tickets.default_project",
    "tickets.default_type",
    "editor.command",
    "display.date_format",
    "display.color",
    "boards.only_mine",
    "sprint.only_mine",
];

/// The type of a Setting. It selects the edit control of the TUI Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A URL that starts with `https://` or `http://`.
    Url,
    /// A number larger than 0. The input accepts only digits.
    Number,
    /// A JQL query. The popup is wider.
    Jql,
    /// A project key. jui checks it with `GET /project/<key>` before it saves.
    Project,
    /// A Ticket type. The values come from the Ticket types of the default project.
    TicketType,
    /// Free text.
    Text,
    /// One of the fixed values.
    Choice(&'static [&'static str]),
}

/// One row of the TUI Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub key: &'static str,
    /// The group (the part before the dot, as a title).
    pub group: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub help: &'static str,
}

/// The Settings page rows, in the order of the page. My Boards is not here: the page shows
/// one row for each Board. The keys are the same as `KEYS`.
pub const PAGE: &[Meta] = &[
    Meta { key: "jira.base_url", group: "Jira", label: "Base URL", kind: Kind::Url, help: "The Jira server. JUI_BASE_URL replaces it." },
    Meta { key: "jira.timeout_secs", group: "Jira", label: "Timeout (seconds)", kind: Kind::Number, help: "The HTTP timeout for each request." },
    Meta {
        key: "tickets.my_tickets_jql",
        group: "Tickets",
        label: "My Tickets JQL",
        kind: Kind::Jql,
        help: "The query of the My Tickets tab and of `jui ls`.",
    },
    Meta {
        key: "tickets.group_by_sprint",
        group: "Tickets",
        label: "Group by Sprint",
        kind: Kind::Choice(&["on", "off"]),
        help: "My Tickets and `jui ls` show the Tickets in groups: active Sprints, future Sprints, then No Sprint.",
    },
    Meta {
        key: "tickets.default_project",
        group: "Tickets",
        label: "Default project",
        kind: Kind::Project,
        help: "Short keys (62886 becomes BCE-62886), new Tickets, and the Board suggestions.",
    },
    Meta { key: "tickets.default_type", group: "Tickets", label: "Default Ticket type", kind: Kind::TicketType, help: "The type of new Tickets." },
    Meta {
        key: "editor.command",
        group: "Editor",
        label: "Editor command",
        kind: Kind::Text,
        help: "Empty means $VISUAL, then $EDITOR, then vi. Arguments are permitted, for example `code --wait`.",
    },
    Meta {
        key: "display.date_format",
        group: "Display",
        label: "Date format",
        kind: Kind::Choice(&["relative", "iso", "local"]),
        help: "Dates in lists: relative (3d), iso (2026-10-16), or local (Oct 16 14:02).",
    },
    Meta {
        key: "display.color",
        group: "Display",
        label: "Colors (CLI)",
        kind: Kind::Choice(&["auto", "always", "never"]),
        help: "Colors in the CLI. NO_COLOR turns colors off when the value is auto. The TUI always uses colors.",
    },
    Meta {
        key: "boards.only_mine",
        group: "Boards",
        label: "Only my Tickets",
        kind: Kind::Choice(&["off", "on"]),
        help: "The Boards tab shows only the Tickets that are assigned to you. m on the Boards tab also changes it.",
    },
    Meta {
        key: "sprint.only_mine",
        group: "Sprint",
        label: "Only my Tickets",
        kind: Kind::Choice(&["off", "on"]),
        help: "The Sprint tab shows only the Tickets that are assigned to you. m on the Sprint tab also changes it.",
    },
];

/// The page row of a key.
pub fn meta(key: &str) -> Option<&'static Meta> {
    PAGE.iter().find(|m| m.key == key)
}

/// The default value of a key as text.
pub fn default_value(key: &str) -> Option<String> {
    Settings::default().get(key)
}

/// The environment variable that replaces a key, if there is one.
pub fn env_name(key: &str) -> Option<&'static str> {
    (key == "jira.base_url").then_some(ENV_BASE_URL)
}

/// The environment that the Settings read. A struct so that tests do not
/// change the process environment.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub base_url: Option<String>,
    pub config: Option<PathBuf>,
}

impl Env {
    pub fn from_process() -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Env { base_url: get(ENV_BASE_URL), config: get(ENV_CONFIG).map(PathBuf::from) }
    }
}

/// The default path: `$XDG_CONFIG_HOME/jui/config.toml` or `~/.config/jui/config.toml`.
pub fn default_path() -> PathBuf {
    use etcetera::BaseStrategy;
    match etcetera::choose_base_strategy() {
        Ok(s) => s.config_dir().join("jui").join("config.toml"),
        Err(_) => PathBuf::from(".config/jui/config.toml"),
    }
}

pub fn config_path(env: &Env) -> PathBuf {
    env.config.clone().unwrap_or_else(default_path)
}

impl Settings {
    /// Load the Settings from the file in `env` (or the default path) and apply the environment.
    pub fn load(env: &Env) -> Result<Settings> {
        let path = config_path(env);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::Settings { path, message: e.to_string() }),
        };
        let mut s = match text {
            Some(t) => Settings::parse(&t, &path)?,
            None => Settings { path: path.clone(), ..Settings::default() },
        };
        if let Some(url) = &env.base_url {
            if s.source("jira.base_url") == Source::File {
                s.file_base_url = Some(s.base_url.clone());
            }
            s.base_url = url.clone();
            s.set_source("jira.base_url", Source::Env);
        }
        s.base_url = s.base_url.trim_end_matches('/').to_string();
        s.env = env.clone();
        Ok(s)
    }

    /// Read the file again with the same environment (after a save or an edit of the file).
    pub fn reload(&self) -> Result<Settings> {
        let mut env = self.env.clone();
        if env.config.is_none() && !self.path.as_os_str().is_empty() {
            env.config = Some(self.path.clone());
        }
        Settings::load(&env)
    }

    /// The value that the Settings page edits: the file value, also when an environment
    /// variable replaces it.
    pub fn page_value(&self, key: &str) -> Option<String> {
        if key == "jira.base_url" && self.source(key) == Source::Env {
            return Some(self.file_base_url.clone().unwrap_or_else(|| DEFAULT_BASE_URL.into()));
        }
        self.get(key)
    }

    /// Parse the file text. A TOML syntax error is an error. A value that is not
    /// valid gives a warning and the default value.
    pub fn parse(text: &str, path: &Path) -> Result<Settings> {
        let doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| Error::Settings {
            path: path.to_path_buf(),
            message: e.to_string().trim().to_string(),
        })?;
        let mut s = Settings { path: path.to_path_buf(), ..Settings::default() };

        for (section, item) in doc.iter() {
            match section {
                "jira" | "tickets" | "editor" | "display" | "boards" | "sprint" => {
                    let Some(table) = item.as_table() else {
                        s.warnings.push(format!("[{section}] must be a table. jui ignores it."));
                        continue;
                    };
                    for (key, v) in table.iter() {
                        let full = format!("{section}.{key}");
                        s.apply(&full, v);
                    }
                }
                "my_boards" => s.apply_boards(item),
                other => s.warnings.push(format!("Unknown Setting `{other}`. Check the spelling.")),
            }
        }
        Ok(s)
    }

    fn apply(&mut self, key: &str, v: &Item) {
        let as_str = || v.as_str().map(str::to_string);
        let ok = match key {
            "jira.base_url" => as_str()
                .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
                .map(|u| self.base_url = u)
                .is_some(),
            "jira.timeout_secs" => v
                .as_integer()
                .filter(|n| *n > 0)
                .map(|n| self.timeout_secs = n as u64)
                .is_some(),
            "tickets.my_tickets_jql" => as_str().filter(|s| !s.trim().is_empty()).map(|s| self.my_tickets_jql = s).is_some(),
            "tickets.group_by_sprint" => v.as_bool().map(|b| self.group_by_sprint = b).is_some(),
            "boards.only_mine" => v.as_bool().map(|b| self.boards_only_mine = b).is_some(),
            "sprint.only_mine" => v.as_bool().map(|b| self.sprint_only_mine = b).is_some(),
            "tickets.default_project" => as_str()
                .filter(|s| !s.is_empty())
                .map(|s| self.default_project = s.to_uppercase())
                .is_some(),
            "tickets.default_type" => as_str().filter(|s| !s.is_empty()).map(|s| self.default_type = s).is_some(),
            "editor.command" => as_str().map(|s| self.editor_command = s).is_some(),
            "display.date_format" => match v.as_str() {
                Some("relative") => { self.date_format = DateFormat::Relative; true }
                Some("iso") => { self.date_format = DateFormat::Iso; true }
                Some("local") => { self.date_format = DateFormat::Local; true }
                _ => false,
            },
            "display.color" => match v.as_str() {
                Some("auto") => { self.color = ColorMode::Auto; true }
                Some("always") => { self.color = ColorMode::Always; true }
                Some("never") => { self.color = ColorMode::Never; true }
                _ => false,
            },
            _ => {
                self.warnings.push(format!("Unknown Setting `{key}`. Check the spelling."));
                return;
            }
        };
        if ok {
            self.set_source(key_static(key), Source::File);
        } else {
            self.warnings.push(format!(
                "The value of `{key}` is not valid ({}). jui uses the default value.",
                v.to_string().trim()
            ));
        }
    }

    fn apply_boards(&mut self, item: &Item) {
        let Some(arr) = item.as_array_of_tables() else {
            self.warnings.push("`my_boards` must be a list of [[my_boards]] tables. jui ignores it.".into());
            return;
        };
        for t in arr.iter() {
            match t.get("id").and_then(|i| i.as_integer()).filter(|i| *i > 0) {
                Some(id) => self.my_boards.push(BoardRef {
                    id: id as u64,
                    name: t.get("name").and_then(|n| n.as_str()).map(str::to_string),
                }),
                None => self.warnings.push("A [[my_boards]] entry has no valid `id`. jui ignores it.".into()),
            }
        }
        self.set_source("my_boards", Source::File);
    }

    fn set_source(&mut self, key: &'static str, src: Source) {
        self.sources.retain(|(k, _)| *k != key);
        self.sources.push((key, src));
    }

    pub fn source(&self, key: &str) -> Source {
        self.sources.iter().find(|(k, _)| *k == key).map(|(_, s)| *s).unwrap_or(Source::Default)
    }

    /// The value of one key as text.
    pub fn get(&self, key: &str) -> Option<String> {
        Some(match key {
            "jira.base_url" => self.base_url.clone(),
            "jira.timeout_secs" => self.timeout_secs.to_string(),
            "tickets.my_tickets_jql" => self.my_tickets_jql.clone(),
            "tickets.group_by_sprint" => (if self.group_by_sprint { "on" } else { "off" }).into(),
            "tickets.default_project" => self.default_project.clone(),
            "tickets.default_type" => self.default_type.clone(),
            "editor.command" => self.editor_command.clone(),
            "display.date_format" => format!("{:?}", self.date_format).to_lowercase(),
            "display.color" => format!("{:?}", self.color).to_lowercase(),
            "boards.only_mine" => (if self.boards_only_mine { "on" } else { "off" }).into(),
            "sprint.only_mine" => (if self.sprint_only_mine { "on" } else { "off" }).into(),
            _ => return None,
        })
    }
}

fn key_static(key: &str) -> &'static str {
    KEYS.iter().find(|k| **k == key).copied().unwrap_or("unknown")
}

// ---------------------------------------------------------------- write

/// The text of a new config file, with comments.
pub fn template() -> String {
    format!(
        r#"# jui Settings. The TUI Settings page also edits this file and keeps your comments.
# Remove a line to use the default value.

[jira]
base_url = "{DEFAULT_BASE_URL}"
timeout_secs = {DEFAULT_TIMEOUT_SECS}

[tickets]
my_tickets_jql = "{DEFAULT_MY_TICKETS_JQL}"
group_by_sprint = true     # My Tickets in groups by Sprint
default_project = "{DEFAULT_PROJECT}"
default_type = "{DEFAULT_TYPE}"

[editor]
# Empty means $VISUAL, then $EDITOR, then vi.
command = ""

[display]
date_format = "relative"   # relative, iso, local
color = "auto"             # auto, always, never

[boards]
only_mine = false          # the Boards tab shows only your Tickets (m changes it)

[sprint]
only_mine = false          # the Sprint tab shows only your Tickets (m changes it)

# My Boards. Use `jui board add <id>` or the TUI Boards tab (+).
# The name is only for display. jui updates it when it changes in Jira.
# [[my_boards]]
# id = 4166
# name = "BCE board"
"#
    )
}

/// Check a value for a key, and return the TOML item to write.
pub fn validate(key: &str, raw: &str) -> Result<Item> {
    let bad = |why: &str| Error::input_hint(format!("`{raw}` is not a valid value for {key}."), why.to_string());
    match key {
        "jira.base_url" => {
            if !(raw.starts_with("https://") || raw.starts_with("http://")) {
                return Err(bad("The URL must start with https://"));
            }
            url::Url::parse(raw).map_err(|_| bad("The URL is not valid."))?;
            Ok(value(raw.trim_end_matches('/')))
        }
        "jira.timeout_secs" => match raw.parse::<i64>() {
            Ok(n) if n > 0 => Ok(value(n)),
            _ => Err(bad("Use a number of seconds larger than 0.")),
        },
        "tickets.my_tickets_jql" | "tickets.default_type" if raw.trim().is_empty() => Err(bad("The value must not be empty.")),
        "tickets.default_project" => {
            if raw.is_empty() || !raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(bad("Use a project key, for example BCE."));
            }
            Ok(value(raw.to_uppercase()))
        }
        "tickets.my_tickets_jql" | "tickets.default_type" | "editor.command" => Ok(value(raw)),
        "tickets.group_by_sprint" | "boards.only_mine" | "sprint.only_mine" => match raw {
            "on" | "true" => Ok(value(true)),
            "off" | "false" => Ok(value(false)),
            _ => Err(bad("Permitted: on, off.")),
        },
        "display.date_format" => match raw {
            "relative" | "iso" | "local" => Ok(value(raw)),
            _ => Err(bad("Permitted: relative, iso, local.")),
        },
        "display.color" => match raw {
            "auto" | "always" | "never" => Ok(value(raw)),
            _ => Err(bad("Permitted: auto, always, never.")),
        },
        _ => Err(unknown_key(key)),
    }
}

pub fn unknown_key(key: &str) -> Error {
    Error::input_hint(format!("Unknown Setting `{key}`."), format!("Permitted keys: {}", KEYS.join(", ")))
}

/// Read the file as an editable document. A missing file gives the template.
fn read_doc(path: &Path) -> Result<DocumentMut> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => template(),
        Err(e) => return Err(Error::Settings { path: path.into(), message: e.to_string() }),
    };
    text.parse().map_err(|e: toml_edit::TomlError| Error::Settings {
        path: path.into(),
        message: e.to_string().trim().to_string(),
    })
}

/// Write the text to a temporary file in the same directory, then rename it.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let err = |e: std::io::Error| Error::Settings { path: path.into(), message: e.to_string() };
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(err)?;
    let tmp = dir.join(format!(".config.toml.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).map_err(err)?;
    std::fs::rename(&tmp, path).map_err(err)
}

/// The result of a check of a config file text after `jui config edit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileCheck {
    /// The text is valid. The list has the warnings (values that are not valid, unknown keys).
    Valid(Vec<String>),
    /// A TOML syntax error. jui cannot use the file.
    Invalid(String),
}

/// Check the text of a config file.
pub fn check_text(text: &str, path: &Path) -> FileCheck {
    match Settings::parse(text, path) {
        Ok(s) => FileCheck::Valid(s.warnings),
        Err(Error::Settings { message, .. }) => FileCheck::Invalid(message),
        Err(e) => FileCheck::Invalid(e.to_string()),
    }
}

/// The text that `jui config edit` opens: the file, or the template when there is no file.
pub fn edit_text(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(template()),
        Err(e) => Err(Error::Settings { path: path.into(), message: e.to_string() }),
    }
}

/// Save the text from `jui config edit`.
pub fn save_text(path: &Path, text: &str) -> Result<()> {
    write_atomic(path, text)
}

fn split_key(key: &str) -> Result<(&str, &str)> {
    if !KEYS.contains(&key) {
        return Err(unknown_key(key));
    }
    Ok(key.split_once('.').expect("all keys have a section"))
}

/// Set one key in the file. Keeps comments, order, and blank lines.
pub fn set(path: &Path, key: &str, raw: &str) -> Result<()> {
    let item = validate(key, raw)?;
    let (section, name) = split_key(key)?;
    let mut doc = read_doc(path)?;
    if !doc.contains_key(section) {
        doc[section] = Item::Table(Table::new());
    }
    let table = doc[section].as_table_mut().ok_or_else(|| Error::Settings {
        path: path.into(),
        message: format!("[{section}] is not a table"),
    })?;
    match table.get_mut(name) {
        // Keep the decoration (the comment at the end of the line).
        Some(existing) if existing.is_value() => {
            let decor = existing.as_value().map(|v| v.decor().clone());
            *existing = item;
            if let (Some(d), Some(v)) = (decor, existing.as_value_mut()) {
                *v.decor_mut() = d;
            }
        }
        _ => {
            table.insert(name, item);
        }
    }
    write_atomic(path, &doc.to_string())
}

/// Remove one key from the file, so that jui uses the default value.
pub fn unset(path: &Path, key: &str) -> Result<()> {
    let (section, name) = split_key(key)?;
    if !path.exists() {
        return Ok(());
    }
    let mut doc = read_doc(path)?;
    if let Some(t) = doc.get_mut(section).and_then(|t| t.as_table_mut()) {
        t.remove(name);
    }
    write_atomic(path, &doc.to_string())
}

/// Write the My Boards list to the file, in this order. Keeps the comments of the file and
/// the comments in each `[[my_boards]]` entry whose ID stays. A new entry goes to the end
/// of the file. An empty list removes all entries.
pub fn write_boards(path: &Path, boards: &[BoardRef]) -> Result<()> {
    let mut doc = read_doc(path)?;
    let mut old: Vec<Table> = match doc.remove("my_boards") {
        Some(Item::ArrayOfTables(a)) => a.into_iter().collect(),
        _ => Vec::new(),
    };
    if boards.is_empty() {
        return write_atomic(path, &doc.to_string());
    }
    // A table keeps its position in the file. The list goes after all other tables, in
    // the order of `boards`.
    let first = doc
        .iter()
        .flat_map(|(_, item)| match item {
            Item::Table(t) => vec![t.position()],
            Item::ArrayOfTables(a) => a.iter().map(Table::position).collect(),
            _ => vec![],
        })
        .flatten()
        .max()
        .unwrap_or(0)
        + 1;
    // The first entry of a new list: the comment at the end of the file (the example in the
    // template) goes above it.
    let mut prefix = None;
    if old.is_empty() && !doc.trailing().as_str().unwrap_or("").trim().is_empty() {
        prefix = doc.trailing().as_str().map(|s| format!("{}\n", s.trim_end()));
        doc.set_trailing("");
    }
    let mut arr = toml_edit::ArrayOfTables::new();
    for (n, b) in boards.iter().enumerate() {
        let mut t = match old.iter().position(|t| t.get("id").and_then(|i| i.as_integer()) == Some(b.id as i64)) {
            Some(i) => old.remove(i),
            None => Table::new(),
        };
        t.set_position(Some(first + n as isize));
        if n == 0
            && let Some(p) = prefix.take()
        {
            t.decor_mut().set_prefix(p);
        }
        set_keep_decor(&mut t, "id", value(b.id as i64));
        match &b.name {
            Some(n) => set_keep_decor(&mut t, "name", value(n.as_str())),
            None => {
                t.remove("name");
            }
        }
        arr.push(t);
    }
    doc.insert("my_boards", Item::ArrayOfTables(arr));
    write_atomic(path, &doc.to_string())
}

/// Set a key in a table and keep the comment at the end of the line.
fn set_keep_decor(t: &mut Table, key: &str, item: Item) {
    match t.get_mut(key) {
        Some(existing) if existing.is_value() => {
            let decor = existing.as_value().map(|v| v.decor().clone());
            *existing = item;
            if let (Some(d), Some(v)) = (decor, existing.as_value_mut()) {
                *v.decor_mut() = d;
            }
        }
        _ => {
            t.insert(key, item);
        }
    }
}

/// Write the template file. Does nothing when the file exists, unless `force`.
pub fn init(path: &Path, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    write_atomic(path, &template())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(t: &str) -> Settings {
        Settings::parse(t, Path::new("/x/config.toml")).unwrap()
    }

    #[test]
    fn empty_file_gives_defaults() {
        let s = parse("");
        assert_eq!(s.base_url, DEFAULT_BASE_URL);
        assert_eq!(s.my_tickets_jql, DEFAULT_MY_TICKETS_JQL);
        assert_eq!(s.default_project, "BCE");
        assert!(s.warnings.is_empty());
        assert_eq!(s.source("jira.base_url"), Source::Default);
    }

    #[test]
    fn template_parses_to_defaults_with_no_warnings() {
        let s = parse(&template());
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(s.timeout_secs, 30);
        assert_eq!(s.source("tickets.default_project"), Source::File);
    }

    #[test]
    fn values_and_boards_are_read() {
        let s = parse(
            r#"
[tickets]
default_project = "xsup"
[display]
date_format = "iso"
[[my_boards]]
id = 4166
name = "BCE board"
[[my_boards]]
id = 4533
"#,
        );
        assert_eq!(s.default_project, "XSUP");
        assert_eq!(s.date_format, DateFormat::Iso);
        assert_eq!(s.my_boards, vec![
            BoardRef { id: 4166, name: Some("BCE board".into()) },
            BoardRef { id: 4533, name: None },
        ]);
    }

    #[test]
    fn group_by_sprint_is_a_boolean_shown_as_on_or_off() {
        assert!(parse("").group_by_sprint);
        let s = parse("[tickets]\ngroup_by_sprint = false\n");
        assert!(!s.group_by_sprint);
        assert_eq!(s.get("tickets.group_by_sprint").as_deref(), Some("off"));
        let s = parse("[tickets]\ngroup_by_sprint = \"no\"\n");
        assert!(s.group_by_sprint, "a bad value gives the default");
        assert_eq!(s.warnings.len(), 1);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        set(&path, "tickets.group_by_sprint", "off").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("group_by_sprint = false"), "{text}");
        assert!(!Settings::parse(&text, &path).unwrap().group_by_sprint);
        assert_eq!(set(&path, "tickets.group_by_sprint", "maybe").unwrap_err().kind(), "input");
    }

    #[test]
    fn boards_only_mine_is_off_by_default_and_has_its_own_section() {
        assert!(!parse("").boards_only_mine);
        let s = parse("[boards]\nonly_mine = true\n");
        assert!(s.boards_only_mine);
        assert_eq!(s.get("boards.only_mine").as_deref(), Some("on"));
        assert_eq!(s.source("boards.only_mine"), Source::File);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# mine\n[tickets]\ndefault_project = \"BCE\"\n").unwrap();
        set(&path, "boards.only_mine", "on").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# mine") && text.contains("[boards]\nonly_mine = true"), "{text}");

        // The Sprint tab has its own Setting.
        let s = parse("[sprint]\nonly_mine = true\n");
        assert!(s.sprint_only_mine && !s.boards_only_mine);
        assert_eq!(s.get("sprint.only_mine").as_deref(), Some("on"));
    }

    #[test]
    fn bad_value_warns_and_uses_default() {
        let s = parse("[display]\ncolor = \"pink\"\n[jira]\ntimeout_secs = \"x\"\n");
        assert_eq!(s.color, ColorMode::Auto);
        assert_eq!(s.timeout_secs, 30);
        assert_eq!(s.warnings.len(), 2);
    }

    #[test]
    fn unknown_key_warns() {
        let s = parse("[jira]\nbase_ulr = \"https://x\"\n[themes]\n");
        assert_eq!(s.warnings.len(), 2);
        assert!(s.warnings[0].contains("jira.base_ulr"));
    }

    #[test]
    fn syntax_error_is_an_error() {
        let e = Settings::parse("[jira\n", Path::new("/x/c.toml")).unwrap_err();
        assert_eq!(e.kind(), "settings");
    }

    #[test]
    fn env_replaces_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[jira]\nbase_url = \"https://file.example\"\n").unwrap();
        let env = Env { base_url: Some("http://127.0.0.1:9/".into()), config: Some(path) };
        let s = Settings::load(&env).unwrap();
        assert_eq!(s.base_url, "http://127.0.0.1:9");
        assert_eq!(s.source("jira.base_url"), Source::Env);
    }

    #[test]
    fn page_rows_are_the_same_keys_as_keys() {
        let page: Vec<&str> = PAGE.iter().map(|m| m.key).collect();
        assert_eq!(page, KEYS);
        for m in PAGE {
            assert!(default_value(m.key).is_some(), "{} has no default", m.key);
            if let Kind::Choice(values) = m.kind {
                assert!(values.contains(&default_value(m.key).unwrap().as_str()));
                for v in values {
                    validate(m.key, v).unwrap();
                }
            }
        }
    }

    #[test]
    fn page_value_is_the_file_value_when_env_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[jira]\nbase_url = \"https://file.example\"\n").unwrap();
        let env = Env { base_url: Some("http://127.0.0.1:9".into()), config: Some(path.clone()) };
        let s = Settings::load(&env).unwrap();
        assert_eq!(s.page_value("jira.base_url").as_deref(), Some("https://file.example"));
        assert_eq!(env_name("jira.base_url"), Some(ENV_BASE_URL));

        // With no value in the file, the page shows the default value.
        std::fs::write(&path, "").unwrap();
        let s = s.reload().unwrap();
        assert_eq!(s.source("jira.base_url"), Source::Env);
        assert_eq!(s.page_value("jira.base_url").as_deref(), Some(DEFAULT_BASE_URL));
    }

    #[test]
    fn reload_reads_the_file_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let env = Env { base_url: None, config: Some(path.clone()) };
        let s = Settings::load(&env).unwrap();
        set(&path, "display.date_format", "iso").unwrap();
        let s = s.reload().unwrap();
        assert_eq!(s.date_format, DateFormat::Iso);
        assert_eq!(s.source("display.date_format"), Source::File);
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let env = Env { base_url: None, config: Some("/nonexistent/jui/config.toml".into()) };
        let s = Settings::load(&env).unwrap();
        assert_eq!(s.base_url, DEFAULT_BASE_URL);
    }

    #[test]
    fn set_keeps_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# my note\n[display]\ncolor = \"auto\"   # keep me\n").unwrap();
        set(&path, "display.color", "never").unwrap();
        set(&path, "tickets.default_project", "xsup").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my note"));
        assert!(text.contains("color = \"never\"   # keep me"), "{text}");
        assert!(text.contains("default_project = \"XSUP\""));
    }

    #[test]
    fn set_on_missing_file_writes_template() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        set(&path, "jira.timeout_secs", "60").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("timeout_secs = 60"));
        assert!(text.contains("# jui Settings."));
    }

    #[test]
    fn set_rejects_bad_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(set(&path, "jira.base_url", "ftp://x").unwrap_err().kind(), "input");
        assert_eq!(set(&path, "nope", "1").unwrap_err().kind(), "input");
        assert!(!path.exists());
    }

    #[test]
    fn unset_removes_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[display]\ncolor = \"never\"\n").unwrap();
        unset(&path, "display.color").unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("color"));
    }

    #[test]
    fn write_boards_keeps_comments_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# top\n[display]\ncolor = \"never\"\n\n# My Boards\n[[my_boards]]\nid = 1\nname = \"One\"   # my board\n\n[[my_boards]]\nid = 2\n",
        )
        .unwrap();
        let list = vec![
            BoardRef { id: 2, name: Some("Two".into()) },
            BoardRef { id: 1, name: Some("One new".into()) },
            BoardRef { id: 3, name: None },
        ];
        write_boards(&path, &list).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# top"), "{text}");
        assert!(text.contains("color = \"never\""));
        assert!(text.contains("name = \"One new\"   # my board"), "{text}");
        let s = Settings::parse(&text, &path).unwrap();
        assert_eq!(s.my_boards, list);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);

        write_boards(&path, &[]).unwrap();
        let s = Settings::parse(&std::fs::read_to_string(&path).unwrap(), &path).unwrap();
        assert!(s.my_boards.is_empty());
    }

    #[test]
    fn write_boards_on_missing_file_writes_template() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_boards(&path, &[BoardRef { id: 4166, name: Some("BCE board".into()) }]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# jui Settings."));
        assert!(text.trim_end().ends_with("# name = \"BCE board\"\n[[my_boards]]\nid = 4166\nname = \"BCE board\""), "{text}");
        assert!(text.contains("# My Boards. Use `jui board add <id>`"));
    }

    #[test]
    fn init_does_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# mine\n").unwrap();
        assert!(!init(&path, false).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\n");
        assert!(init(&path, true).unwrap());
    }
}
