#!/usr/bin/env bash
# Run similarity-rs over the shell-executor workspace with intentional-boilerplate
# directories and files filtered out.
#
# Background: in similarity-rs v0.5.0 the `--exclude` flag (and the
# `exclude` key in `similarity.toml`) are not honored by the default
# function-similarity analyzer. The only reliable way to filter the default
# mode is to pass an explicit list of files.
#
# Other settings (min_lines, threshold) come from `similarity.toml` at the
# repo root.
#
# Usage:
#   ./check-similarity.sh                # standard run
#   ./check-similarity.sh --print        # show duplicate code blocks
#   ./check-similarity.sh --threshold 0.9
#   ./check-similarity.sh --fail-on-duplicates  # CI mode

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_ROOT"

# Resolve the similarity-rs binary. Falls back to ~/.cargo/bin which is
# where `cargo install` puts it but which isn't always on $PATH in CI shells.
if command -v similarity-rs >/dev/null 2>&1; then
    SIMILARITY_BIN="$(command -v similarity-rs)"
elif [[ -x "${HOME}/.cargo/bin/similarity-rs" ]]; then
    SIMILARITY_BIN="${HOME}/.cargo/bin/similarity-rs"
else
    echo "error: similarity-rs not found" >&2
    echo "install with: cargo install similarity-rs" >&2
    exit 127
fi

# Build the explicit file list. Excludes:
#   - target/    build output
#   - */tests/*  integration tests routinely share scaffolding
FILE_LIST="$(
    find . -type f -name '*.rs' \
        -not -path './target/*' \
        -not -path '*/tests/*' \
        | sort
)"

if [[ -z "$FILE_LIST" ]]; then
    echo "error: no Rust files matched after filtering" >&2
    exit 1
fi

# Word-splitting is intentional here — the file list has no whitespace
# in any path inside the workspace.
# shellcheck disable=SC2086
exec "$SIMILARITY_BIN" "$@" $FILE_LIST
