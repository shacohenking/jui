//! Attachments: pure functions for the CLI and the TUI. See ticket 25 "Build: attachments".
//!
//! The rules:
//! - An attachment argument is an ID (only digits, an exact ID), or a file name: exact,
//!   then exact with no case, then a unique prefix with no case. Jira permits two files
//!   with the same name, so a name can find more attachments.
//! - jui never writes a file outside the target directory: the server file name becomes
//!   one safe path part (`safe_name`).
//! - jui sends the token only to its base URL (`download_path`).
//! - The upload limit comes from the server (`GET /rest/api/2/attachment/meta`).

use std::path::{Path, PathBuf};

use super::error::{Error, Result};
use super::format::human_size;
use super::model::Attachment;

/// The upload limit when the server does not tell it (the value of this server: 50 MB).
pub const DEFAULT_LIMIT: u64 = 52_428_800;

/// The server settings for attachments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub enabled: bool,
    /// The upload limit in bytes.
    pub limit: u64,
}

/// One file that jui can upload: the name for Jira, the MIME type, and the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub name: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

impl Upload {
    pub fn describe(&self) -> String {
        format!("{} ({})", self.name, human_size(self.bytes.len() as u64))
    }
}

/// Find the attachments that an argument names. See the module rules.
pub fn select<'a>(all: &'a [Attachment], arg: &str) -> Vec<&'a Attachment> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Vec::new();
    }
    if arg.chars().all(|c| c.is_ascii_digit()) {
        let by_id: Vec<&Attachment> = all.iter().filter(|a| a.id == arg).collect();
        if !by_id.is_empty() {
            return by_id;
        }
    }
    let exact: Vec<&Attachment> = all.iter().filter(|a| a.filename == arg).collect();
    if !exact.is_empty() {
        return exact;
    }
    let low = arg.to_lowercase();
    let no_case: Vec<&Attachment> = all.iter().filter(|a| a.filename.to_lowercase() == low).collect();
    if !no_case.is_empty() {
        return no_case;
    }
    all.iter().filter(|a| a.filename.to_lowercase().starts_with(&low)).collect()
}

/// One line for a list or a question: `900  trace.log  2 KB  @jdoe  2026-10-11`.
pub fn one_line(a: &Attachment) -> String {
    let who = a.author.as_ref().map(|u| format!("  @{}", u.name)).unwrap_or_default();
    let day = a.created.map(|d| format!("  {}", d.format("%Y-%m-%d"))).unwrap_or_default();
    format!("{}  {}{who}{day}", a.filename, human_size(a.size))
}

/// The error when no attachment matches.
pub fn not_found(key: &str, arg: &str) -> Error {
    Error::input_hint(format!("{key} has no attachment `{arg}`."), format!("`jui attachment list {key}` shows the IDs and the names."))
}

/// The error when more attachments match in a script.
pub fn ambiguous(key: &str, arg: &str, found: &[&Attachment]) -> Error {
    let list = found.iter().map(|a| format!("{} ({})", a.id, one_line(a))).collect::<Vec<_>>().join(", ");
    Error::input_hint(format!("{} attachments of {key} match `{arg}`: {list}.", found.len()), "Give the attachment ID.")
}

/// A server file name as one safe file name. jui never writes outside the target directory.
pub fn safe_name(name: &str, id: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        format!("attachment-{id}")
    } else {
        cleaned
    }
}

/// The path of a download on the base URL. jui uses the path of `content` only when it
/// starts with the base URL; else it makes `/secure/attachment/{id}/{name}`. So the token
/// never goes to a different host.
pub fn download_path(a: &Attachment, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if let Some(url) = &a.url
        && let Some(rest) = url.strip_prefix(base)
        && rest.starts_with("/secure/attachment/")
    {
        return rest.to_string();
    }
    let name = url::form_urlencoded::byte_serialize(a.filename.as_bytes()).collect::<String>().replace('+', "%20");
    format!("/secure/attachment/{}/{name}", a.id)
}

/// Clean a path that the user typed or pasted: quotes at the two ends, `\ ` (a pasted
/// path from a file manager), `file://`, and `~` at the start.
pub fn clean_path(input: &str, home: Option<&Path>) -> PathBuf {
    let mut s = input.trim().to_string();
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            s = s[1..s.len() - 1].to_string();
            break;
        }
    }
    if let Some(rest) = s.strip_prefix("file://") {
        s = url::form_urlencoded::parse(format!("x={}", rest.replace('+', "%2B")).as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_else(|| rest.to_string());
    }
    // `\ ` becomes a space, `\(` becomes `(`. A backslash with no next character stays.
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&n)) if !n.is_alphanumeric() => {
                out.push(n);
                chars.next();
            }
            _ => out.push(c),
        }
    }
    match home {
        Some(h) if out == "~" => h.to_path_buf(),
        Some(h) if out.starts_with("~/") => h.join(&out[2..]),
        _ => PathBuf::from(out),
    }
}

/// Make an `Upload` from a file: it must exist, be a file, and be in the limit.
pub fn read_upload(path: &Path, limit: u64) -> Result<Upload> {
    let shown = path.display();
    let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::input(format!("The file {shown} does not exist.")),
        _ => Error::input(format!("Cannot read {shown}: {e}")),
    })?;
    if meta.is_dir() {
        return Err(Error::input_hint(format!("{shown} is a directory."), "Give a file. Make a zip file to upload a directory."));
    }
    if meta.len() > limit {
        return Err(Error::input(format!(
            "{shown} has {}. The upload limit of Jira is {}.",
            human_size(meta.len()),
            human_size(limit)
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| Error::input(format!("Cannot read {shown}: {e}")))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    Ok(Upload { mime: mime_type(&name), name: form_name(&name), bytes })
}

/// The name in the multipart header. A quote or a line break would break the header.
fn form_name(name: &str) -> String {
    name.chars().map(|c| if c == '"' || c.is_control() { '_' } else { c }).collect()
}

/// The MIME type from the file extension. Jira keeps it and the browser uses it.
pub fn mime_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" | "har" => "application/json",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Where `download` writes a file. `out`: the `-o` value (a file or a directory), or `None`
/// for the current directory. `-` is stdout (`None` in the result).
pub fn target(a: &Attachment, out: Option<&Path>) -> Option<PathBuf> {
    let name = safe_name(&a.filename, &a.id);
    match out {
        Some(p) if p == Path::new("-") => None,
        Some(p) if p.is_dir() => Some(p.join(name)),
        Some(p) => Some(p.to_path_buf()),
        None => Some(PathBuf::from(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::User;

    fn att(id: &str, name: &str) -> Attachment {
        Attachment {
            id: id.into(),
            filename: name.into(),
            size: 2048,
            mime_type: None,
            author: Some(User { name: "jdoe".into(), display_name: "Jane".into() }),
            created: None,
            url: Some(format!("https://jira.example/secure/attachment/{id}/{name}")),
        }
    }

    #[test]
    fn select_by_id_then_name_then_prefix() {
        let all = [att("900", "trace.log"), att("901", "Trace.log"), att("902", "screen.png"), att("903", "900")];
        assert_eq!(select(&all, "900").iter().map(|a| &a.id).collect::<Vec<_>>(), ["900"]);
        assert_eq!(select(&all, "trace.log").iter().map(|a| &a.id).collect::<Vec<_>>(), ["900"]);
        assert_eq!(select(&all, "TRACE.LOG").len(), 2, "no exact match: both match with no case");
        assert_eq!(select(&all, "scr").iter().map(|a| &a.id).collect::<Vec<_>>(), ["902"]);
        assert_eq!(select(&all, "tr").len(), 2);
        assert!(select(&all, "x").is_empty());
        assert!(select(&all, " ").is_empty());
    }

    #[test]
    fn safe_names_stay_in_the_directory() {
        assert_eq!(safe_name("../../etc/passwd", "1"), ".._.._etc_passwd");
        assert_eq!(safe_name("..", "7"), "attachment-7");
        assert_eq!(safe_name("a\\b\nc.txt", "1"), "a_b_c.txt");
        assert_eq!(safe_name("  ", "3"), "attachment-3");
        assert_eq!(safe_name("résumé.pdf", "1"), "résumé.pdf");
    }

    #[test]
    fn download_path_uses_only_the_base_url() {
        let a = att("900", "trace.log");
        assert_eq!(download_path(&a, "https://jira.example/"), "/secure/attachment/900/trace.log");
        // A different host: jui makes the path, and the token stays on the base URL.
        assert_eq!(download_path(&a, "http://127.0.0.1:9"), "/secure/attachment/900/trace.log");
        let mut b = att("5", "a b+c.txt");
        b.url = Some("https://evil.example/secure/attachment/5/x".into());
        assert_eq!(download_path(&b, "https://jira.example"), "/secure/attachment/5/a%20b%2Bc.txt");
    }

    #[test]
    fn clean_pasted_paths() {
        let home = Path::new("/Users/me");
        assert_eq!(clean_path("  '/tmp/a b.png' ", Some(home)), PathBuf::from("/tmp/a b.png"));
        assert_eq!(clean_path("\"/tmp/x.txt\"", Some(home)), PathBuf::from("/tmp/x.txt"));
        assert_eq!(clean_path("/tmp/my\\ file\\(1\\).log", Some(home)), PathBuf::from("/tmp/my file(1).log"));
        assert_eq!(clean_path("~/Desktop/s.png", Some(home)), PathBuf::from("/Users/me/Desktop/s.png"));
        assert_eq!(clean_path("~", Some(home)), PathBuf::from("/Users/me"));
        assert_eq!(clean_path("file:///tmp/a%20b.png", Some(home)), PathBuf::from("/tmp/a b.png"));
        assert_eq!(clean_path("C:\\x", None), PathBuf::from("C:\\x"));
    }

    #[test]
    fn read_upload_checks_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("note \"1\".txt");
        std::fs::write(&p, b"hello").unwrap();
        let u = read_upload(&p, 100).unwrap();
        assert_eq!((u.name.as_str(), u.mime, u.bytes.len()), ("note _1_.txt", "text/plain", 5));
        assert!(read_upload(&p, 4).unwrap_err().to_string().contains("The upload limit of Jira is 4 B."));
        assert!(read_upload(dir.path(), 100).unwrap_err().to_string().contains("is a directory"));
        assert!(read_upload(&dir.path().join("none"), 100).unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn targets() {
        let dir = tempfile::tempdir().unwrap();
        let a = att("900", "../trace.log");
        assert_eq!(target(&a, None), Some(PathBuf::from(".._trace.log")));
        assert_eq!(target(&a, Some(dir.path())), Some(dir.path().join(".._trace.log")));
        assert_eq!(target(&a, Some(Path::new("/tmp/x.log"))), Some(PathBuf::from("/tmp/x.log")));
        assert_eq!(target(&a, Some(Path::new("-"))), None);
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_type("A.PNG"), "image/png");
        assert_eq!(mime_type("x.har"), "application/json");
        assert_eq!(mime_type("Makefile"), "application/octet-stream");
    }
}
