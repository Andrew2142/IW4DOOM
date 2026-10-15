#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Verifies that ../iw4L is checked out at the revision pinned in IW4L_REV and that its
# crates/ tree has no local modifications. iw4L is a read-only dependency of DOOMMW2.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
iw4l="${IW4L_DIR:-$root/../iw4L}"
pinned="$(tr -d '[:space:]' < "$root/IW4L_REV")"

if [ ! -d "$iw4l/.git" ] && ! git -C "$iw4l" rev-parse --git-dir >/dev/null 2>&1; then
    echo "check-iw4l-rev: $iw4l is not a git checkout; clone iw4L next to doommw2 at $pinned" >&2
    exit 1
fi

actual="$(git -C "$iw4l" rev-parse --short=7 HEAD)"
if [ "$actual" != "$pinned" ]; then
    echo "check-iw4l-rev: iw4L HEAD is $actual but IW4L_REV pins $pinned" >&2
    exit 1
fi

dirty="$(git -C "$iw4l" status --porcelain crates)"
if [ -n "$dirty" ]; then
    echo "check-iw4l-rev: iw4L has uncommitted changes under crates/ (it must stay read-only):" >&2
    echo "$dirty" >&2
    echo "pinned=$pinned actual=$actual" >&2
    exit 1
fi

echo "check-iw4l-rev: iw4L $actual matches IW4L_REV"
