#!/bin/sh
# Install jui. Checks for Rust (1.88 or newer), offers to install or update it with rustup,
# then runs `cargo install` for jui.
#
#   curl -fsSL https://raw.githubusercontent.com/shacohenking/jui/main/install.sh | sh
#
# Set JUI_YES=1 to skip the questions (for scripts and CI).

set -eu

REPO_URL="https://github.com/shacohenking/jui"
MIN_MAJOR=1
MIN_MINOR=88

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# Ask a yes/no question. Reads from the terminal, because stdin is the script when piped.
confirm() {
    [ "${JUI_YES:-}" = "1" ] && return 0
    if [ -r /dev/tty ]; then
        printf '%s [y/N] ' "$1" > /dev/tty
        read -r answer < /dev/tty || answer=""
        case "$answer" in y | Y | yes | YES) return 0 ;; esac
        return 1
    fi
    die "$1 No terminal to ask. Run again with JUI_YES=1 to say yes."
}

# True when `rustc` exists and is at least MIN_MAJOR.MIN_MINOR.
rust_ok() {
    command -v rustc > /dev/null 2>&1 && command -v cargo > /dev/null 2>&1 || return 1
    v=$(rustc --version 2> /dev/null | awk '{print $2}')
    major=${v%%.*}
    rest=${v#*.}
    minor=${rest%%.*}
    case "$major$minor" in *[!0-9]* | "") return 1 ;; esac
    [ "$major" -gt "$MIN_MAJOR" ] || { [ "$major" -eq "$MIN_MAJOR" ] && [ "$minor" -ge "$MIN_MINOR" ]; }
}

load_cargo_env() {
    # shellcheck disable=SC1091
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    return 0
}

load_cargo_env

if rust_ok; then
    say "Rust $(rustc --version | awk '{print $2}') found."
elif command -v rustup > /dev/null 2>&1; then
    say "Rust is older than $MIN_MAJOR.$MIN_MINOR (or incomplete). rustup can update it."
    confirm "Run 'rustup update stable'?" || die "jui needs Rust $MIN_MAJOR.$MIN_MINOR or newer."
    rustup update stable
    rustup default stable > /dev/null 2>&1 || true
    load_cargo_env
elif command -v rustc > /dev/null 2>&1; then
    die "Rust $(rustc --version | awk '{print $2}') is older than $MIN_MAJOR.$MIN_MINOR, and it was not installed with rustup. Update it with your package manager, or install rustup from https://rustup.rs, then run this again."
else
    say "Rust is not installed. jui needs Rust $MIN_MAJOR.$MIN_MINOR or newer."
    confirm "Install Rust with rustup (https://rustup.rs)?" || die "Rust is required."
    command -v curl > /dev/null 2>&1 || die "curl is required to install rustup."
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
    load_cargo_env
fi

rust_ok || die "Rust $MIN_MAJOR.$MIN_MINOR or newer is still not available. Open a new terminal and run this again."

say "Installing jui..."
cargo install --git "$REPO_URL" --locked --force

say ""
say "jui is installed: $(command -v jui 2> /dev/null || echo "$HOME/.cargo/bin/jui")"
case ":$PATH:" in
    *":$HOME/.cargo/bin:"*) ;;
    *) say "Add ~/.cargo/bin to your PATH (add this line to ~/.zshrc, then open a new terminal):"
       say "  export PATH=\"\$HOME/.cargo/bin:\$PATH\"" ;;
esac
say ""
say "Next, set your Jira server and token (see the README):"
say "  export JUI_BASE_URL=\"https://jira.your-company.com\""
say "  export JIRA_TOKEN=\"<your personal access token>\""
