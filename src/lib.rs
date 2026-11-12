//! jui: a CLI and TUI client for one Jira Data Center server.
//!
//! The `core` module is shared by the `cli` and `tui` modules. The `cli` and
//! `tui` modules must not import each other.

pub mod cli;
pub mod clipboard;
pub mod core;
pub mod editor;
pub mod tui;
