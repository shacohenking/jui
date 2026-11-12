//! Debug log. `-v` writes HTTP lines to stderr; `-vv` adds bodies.
//! `JUI_LOG=debug` writes the same lines to `~/.local/state/jui/jui.log`.
//! The token is never logged: requests log only the method, path, query, and status.

use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub fn log_path() -> PathBuf {
    use etcetera::BaseStrategy;
    match etcetera::choose_base_strategy() {
        Ok(s) => s.state_dir().unwrap_or_else(|| s.data_dir()).join("jui").join("jui.log"),
        Err(_) => PathBuf::from("jui.log"),
    }
}

pub fn init(verbose: u8) {
    let level = match verbose {
        0 => None,
        1 => Some("jui=debug"),
        _ => Some("jui=trace"),
    };
    if let Some(l) = level {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(l))
            .with_writer(std::io::stderr)
            .without_time()
            .with_target(false)
            .with_level(false)
            .try_init();
        return;
    }
    let Ok(spec) = std::env::var("JUI_LOG") else { return };
    let path = log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    let filter = if spec.contains('=') { spec } else { format!("jui={spec}") };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .try_init();
}
