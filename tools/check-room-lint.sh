#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Clippy gate for the engine crate. The copied Doom port (room/src/doom, its tests) has
# hundreds of pre-existing clippy findings that its crate-level allow list does not cover, so
# `-D warnings` cannot be applied to the whole crate. This script runs clippy on `room` and
# fails if any diagnostic points into room/src/dmw/ (new DOOMMW2 code), or if the build fails.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

if ! cargo clippy -p room --all-targets --message-format=short >"$log" 2>&1; then
    cat "$log" >&2
    echo "check-room-lint: cargo clippy failed for room" >&2
    exit 1
fi

if grep -E '^room/src/dmw/' "$log" >&2; then
    echo "check-room-lint: diagnostics in room/src/dmw/ (new code must be warning-free)" >&2
    exit 1
fi

echo "check-room-lint: room/src/dmw is clean"
