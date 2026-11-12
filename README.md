# jui

jui is a personal Jira client for one Jira Data Center server. It has a CLI and a TUI with the same functions.

## Install

jui needs Rust 1.88 or a newer version.

### One-liner

This checks for Rust (and installs or updates it with rustup, after it asks you), then installs jui:

```sh
curl -fsSL https://raw.githubusercontent.com/shacohenking/jui/main/install.sh | sh
```

Set `JUI_YES=1` before `sh` to skip the questions (`curl ... | JUI_YES=1 sh`). To read the script first, open `install.sh` in this repository.

If you already have Rust 1.88 or newer, you can use Cargo directly:

```sh
cargo install --git https://github.com/shacohenking/jui --locked
```

### From a clone

Do this command in the directory of the repository:

```sh
cargo install --path . --locked
```

### After the install

Cargo puts the `jui` binary in `~/.cargo/bin`. Make sure that `~/.cargo/bin` is in your `PATH`:

```sh
command -v jui      # must show ~/.cargo/bin/jui
jui --version       # shows the version from Cargo.toml
```

If `command -v jui` shows nothing, add this line to `~/.zshrc`, and then open a new terminal:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

## Required environment variables

jui has no built-in Jira server. Set these two variables before you start jui. Add them to `~/.zshrc` (or your shell profile) to keep them:

```sh
export JUI_BASE_URL="https://jira.your-company.com"   # your Jira Data Center server
export JIRA_TOKEN="<your personal access token>"      # see "Token" below
```

Instead of `JUI_BASE_URL`, you can set `jira.base_url` in the config file (`jui config set jira.base_url https://jira.your-company.com`). Without one of them, jui uses the placeholder `https://jira.example.com`, and every request fails.

## Update

Do the install command again (the one-liner, or `cargo install --path . --locked` in a clone):

```sh
cargo install --git https://github.com/shacohenking/jui --locked --force
```

Cargo builds jui again and replaces the old binary. There is no update command.

## Token

jui reads the token only from the `JIRA_TOKEN` environment variable. jui sends it as `Authorization: Bearer <token>`. jui does not keep the token in the config file. Create a personal access token in your Jira profile.

## Settings

The config file is `~/.config/jui/config.toml`. If `XDG_CONFIG_HOME` is set, the file is `$XDG_CONFIG_HOME/jui/config.toml`. The file is optional. Each Setting has a default value in the code.

```sh
jui config path     # show the path of the config file
jui config init     # write the file with comments and the default values
jui config show     # show all Settings with their values and sources
jui config edit     # open the file in $EDITOR
```

In the TUI, tab 6 is the Settings page. It saves each change immediately.

The order of priority is:

1. Command flags.
2. `JUI_BASE_URL` (the Jira server) and `JUI_CONFIG` (a different config file).
3. The config file.
4. The default values.

jui does not use `JIRA_URL`.

### My Tickets by Sprint

My Tickets (TUI tab 1 and `jui ls`) shows the Tickets in groups: the active Sprints first, then the future Sprints, then "No Sprint". In each group, the order is the order of the JQL query. A Ticket that is only in closed Sprints is in "No Sprint". `jui ls --json` does not change the order, and each item has a `sprint` field.

To turn off the groups, set "Group by Sprint" to `off` on the Settings page, or do this command:

```sh
jui config set tickets.group_by_sprint off
```

### Only my Tickets on a Board

On the Boards tab, push `m` to show only the Tickets that are assigned to you. Push `m` again to show all Tickets. jui saves this in the Setting `boards.only_mine`, so the Boards tab shows the same Tickets when jui starts again. The line above the Board shows "(only mine)".

In the CLI, use the `--mine` (or `-m`) flag. The Setting does not change the CLI:

```sh
jui board view 4537 --mine
```

### Only my Tickets on the Sprint tab

On the Sprint tab, push `m` to show only the Tickets that are assigned to you, in the active Sprints and in the backlog. Push `m` again to show all Tickets. jui saves this in the Setting `sprint.only_mine`. This Setting is different from `boards.only_mine`. The line above the list shows "only mine".

In the CLI, use the `--mine` (or `-m`) flag:

```sh
jui sprint active 4537 --mine
jui sprint view 43198 --mine
```

### My Epics

The Epics tab (TUI tab 4) shows the Epics that have at least one Ticket that is assigned to you and is not Done. The "Mine" column shows the number of these Tickets in each Epic. A Done Epic is dim. Push Enter to see all Tickets of the Epic. Push `r` to get the Epics again.

In the zoom of an Epic (from any tab), push `n` to create a Ticket in this Epic. The Template has `epic = "KEY"`, and the project and the type are the defaults. You can change all of them in the Template. In the CLI, use `jui new --epic KEY`.

In the CLI:

```sh
jui epic mine          # key, the number of your open Tickets, and the Epic name
jui epic mine --json   # each item has a "my_open_tickets" field
```

## Shell completion

For zsh, write the script into a directory in `fpath`, and then open a new terminal:

```sh
jui completions zsh > "${fpath[1]}/_jui"
```

For other shells:

```sh
jui completions bash > ~/.local/share/bash-completion/completions/jui
jui completions fish > ~/.config/fish/completions/jui.fish
```

Do the command again after an update, because the commands can change.

## Start

```sh
jui            # start the TUI
jui ls         # list My Tickets
jui BCE-1234   # show one Ticket
jui --help     # show all commands
```

## Logs

`-v` shows the HTTP requests on stderr. `-vv` also shows the bodies. The TUI uses the terminal, thus use `JUI_LOG=debug` for the TUI. jui then writes the log to `~/.local/state/jui/jui.log`. The log does not contain the token.
