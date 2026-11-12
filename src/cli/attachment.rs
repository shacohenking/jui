//! `jui attachment`. See ticket 25 "Build: attachments".
//!
//! The rules:
//! - `upload` checks all files (exist, not a directory, in the server limit) before it sends
//!   the first one. It sends one request for each file. When a later file fails, jui tells
//!   which files are in Jira.
//! - `download` writes to the current directory, to `-o <dir>`, to `-o <file>` (one
//!   attachment), or to stdout (`-o -`, one attachment). A file that exists is not replaced:
//!   jui asks on a terminal, and a script needs `--force`. jui checks all targets before the
//!   first download. With no name and no `--all`, jui asks on a terminal and stops in a script.
//! - `delete` asks on a terminal. `--yes` skips the question. A script without `--yes` stops.
//! - A name that matches more attachments asks on a terminal and stops in a script.

use std::path::{Path, PathBuf};

use super::prompt;
use super::write::Io;
use crate::core::attachment::{self as at, Upload};
use crate::core::format::human_size;
use crate::core::model::Attachment;
use crate::core::{Error, Jira, Result};

/// The lines of `jui attachment list`: the ID, the name, the size, the author, the date.
pub fn render_list(atts: &[Attachment]) -> String {
    let iw = atts.iter().map(|a| a.id.len()).max().unwrap_or(0);
    let nw = atts.iter().map(|a| a.filename.chars().count()).max().unwrap_or(0);
    let sw = atts.iter().map(|a| human_size(a.size).len()).max().unwrap_or(0);
    atts.iter()
        .map(|a| {
            let who = a.author.as_ref().map(|u| format!("@{}", u.name)).unwrap_or_default();
            let day = a.created.map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
            format!("{:<iw$}  {:<nw$}  {:>sw$}  {day}  {who}", a.id, a.filename, human_size(a.size)).trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Find one attachment by its argument. More matches ask on a terminal.
fn pick_one(key: &str, all: &[Attachment], arg: &str, io: &mut Io<'_>) -> Result<Attachment> {
    let found = at::select(all, arg);
    match found.as_slice() {
        [] => Err(at::not_found(key, arg)),
        [one] => Ok((*one).clone()),
        many => {
            let Some(input) = io.input.as_deref_mut() else { return Err(at::ambiguous(key, arg, many)) };
            let items: Vec<String> = many.iter().map(|a| format!("{}  {}", a.id, at::one_line(a))).collect();
            let i = prompt::select(input, io.out, &format!("More than one attachment matches `{arg}`:"), &items)?;
            Ok(many[i].clone())
        }
    }
}

// ------------------------------------------------------------------ upload

/// `jui attachment upload`. Returns the new attachments.
pub async fn upload(j: &Jira, key: &str, files: &[PathBuf], io: &mut Io<'_>) -> Result<Vec<Attachment>> {
    let meta = j.attachment_meta().await?;
    // Check all files first, so that a bad file stops before the first upload.
    let mut uploads: Vec<Upload> = Vec::new();
    for f in files {
        uploads.push(at::read_upload(f, meta.limit)?);
    }
    let mut done: Vec<Attachment> = Vec::new();
    for u in &uploads {
        match j.upload(key, u).await {
            Ok(mut a) => {
                let _ = writeln!(io.out, "Uploaded {} to {key}.", u.describe());
                done.append(&mut a);
            }
            Err(e) if done.is_empty() => return Err(e),
            Err(e) => {
                let names = done.iter().map(|a| a.filename.as_str()).collect::<Vec<_>>().join(", ");
                return Err(Error::input_hint(
                    format!("The upload of {} failed: {e}", u.name),
                    format!("These files are in Jira now: {names}. Upload only the other files again."),
                ));
            }
        }
    }
    Ok(done)
}

// ------------------------------------------------------------------ download

/// The arguments of `jui attachment download`.
pub struct DownloadCmd<'a> {
    pub key: &'a str,
    pub names: &'a [String],
    pub all: bool,
    pub output: Option<&'a Path>,
    pub force: bool,
}

/// One downloaded file: the attachment, the target (`None` is stdout), and the size.
#[derive(Debug, Clone)]
pub struct Downloaded {
    pub attachment: Attachment,
    pub target: Option<PathBuf>,
    pub size: u64,
}

/// The attachments to download.
fn choose(c: &DownloadCmd<'_>, all: &[Attachment], io: &mut Io<'_>) -> Result<Vec<Attachment>> {
    if c.all {
        return Ok(all.to_vec());
    }
    if c.names.is_empty() {
        let Some(input) = io.input.as_deref_mut() else {
            return Err(Error::input_hint(
                format!("{} has {} attachments. Which ones?", c.key, all.len()),
                format!("Give an ID or a name, or use --all. `jui attachment list {}` shows them.", c.key),
            ));
        };
        if let [one] = all {
            return Ok(vec![one.clone()]);
        }
        let mut items: Vec<String> = all.iter().map(|a| format!("{}  {}", a.id, at::one_line(a))).collect();
        items.push("All attachments".into());
        let i = prompt::select(input, io.out, &format!("Download from {}:", c.key), &items)?;
        return Ok(if i == all.len() { all.to_vec() } else { vec![all[i].clone()] });
    }
    let mut out: Vec<Attachment> = Vec::new();
    for n in c.names {
        let a = pick_one(c.key, all, n, io)?;
        if !out.iter().any(|o| o.id == a.id) {
            out.push(a);
        }
    }
    Ok(out)
}

/// The target of each attachment. Two attachments with the same name get `<id>-<name>`
/// for the second one, so that one file does not replace the other.
fn targets(c: &DownloadCmd<'_>, chosen: &[Attachment]) -> Result<Vec<Option<PathBuf>>> {
    let to_stdout = c.output == Some(Path::new("-"));
    let is_dir = c.output.is_some_and(Path::is_dir);
    if chosen.len() > 1 && c.output.is_some() && !is_dir {
        let what = if to_stdout { "stdout" } else { "one file" };
        return Err(Error::input_hint(
            format!("jui cannot write {} attachments to {what}.", chosen.len()),
            "Give one attachment, or give a directory that exists with -o.",
        ));
    }
    let mut out: Vec<Option<PathBuf>> = Vec::new();
    for a in chosen {
        let mut t = at::target(a, c.output);
        if let Some(p) = &t
            && out.iter().flatten().any(|o| o == p)
        {
            let name = format!("{}-{}", a.id, at::safe_name(&a.filename, &a.id));
            t = Some(p.with_file_name(name));
        }
        out.push(t);
    }
    Ok(out)
}

/// Files that exist are replaced only with `--force` or a "yes" on a terminal.
fn check_existing(c: &DownloadCmd<'_>, targets: &[Option<PathBuf>], io: &mut Io<'_>) -> Result<()> {
    let existing: Vec<&PathBuf> = targets.iter().flatten().filter(|p| p.exists()).collect();
    if existing.is_empty() || c.force {
        return Ok(());
    }
    if let Some(d) = existing.iter().find(|p| p.is_dir()) {
        return Err(Error::input(format!("{} is a directory.", d.display())));
    }
    let names = existing.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
    let Some(input) = io.input.as_deref_mut() else {
        return Err(Error::input_hint(format!("The file exists: {names}."), "Use --force to replace it, or -o to write to a different place."));
    };
    let answer = prompt::ask(input, io.out, &format!("Replace {names}? [y/N] "))?;
    if matches!(answer.to_lowercase().as_str(), "y" | "yes") { Ok(()) } else { Err(Error::Cancelled) }
}

/// `jui attachment download`.
pub async fn download(j: &Jira, c: DownloadCmd<'_>, io: &mut Io<'_>) -> Result<Vec<Downloaded>> {
    let (_, all) = j.attachments(c.key).await?;
    if all.is_empty() {
        return Err(Error::input(format!("{} has no attachments.", c.key)));
    }
    let chosen = choose(&c, &all, io)?;
    let targets = targets(&c, &chosen)?;
    check_existing(&c, &targets, io)?;
    let mut done = Vec::new();
    for (a, t) in chosen.into_iter().zip(targets) {
        let size = match &t {
            Some(p) => j.download_to(&a, p).await?,
            None => {
                let mut stdout = tokio::io::stdout();
                let n = j.download(&a, &mut stdout).await?;
                use tokio::io::AsyncWriteExt;
                let _ = stdout.flush().await;
                n
            }
        };
        if let Some(p) = &t {
            let _ = writeln!(io.out, "Downloaded {} ({}) to {}.", a.filename, human_size(size), p.display());
        }
        done.push(Downloaded { attachment: a, target: t, size });
    }
    Ok(done)
}

// ------------------------------------------------------------------ delete

/// `jui attachment delete`. Returns the deleted attachment, or `None` when the user says no.
pub async fn delete(j: &Jira, key: &str, arg: &str, yes: bool, io: &mut Io<'_>) -> Result<Option<Attachment>> {
    let (_, all) = j.attachments(key).await?;
    let a = pick_one(key, &all, arg, io)?;
    if !yes {
        let Some(input) = io.input.as_deref_mut() else {
            return Err(Error::input_hint(
                format!("jui must ask before it deletes the attachment {} ({}).", a.filename, a.id),
                "Use --yes to delete it. A delete cannot be reversed.",
            ));
        };
        let _ = writeln!(io.out, "{}", at::one_line(&a));
        let answer = prompt::ask(input, io.out, &format!("Delete attachment {} on {key}? [y/N] ", a.id))?;
        if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
            return Ok(None);
        }
    }
    j.delete_attachment(&a.id).await?;
    Ok(Some(a))
}
