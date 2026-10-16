#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Verifies that every iw4L dependency is pinned to the commit in IW4L_REV:
#   - the `rev` of every iw4L git dependency in Cargo.toml and tools/dmw_export/Cargo.toml,
#   - the iw4L `source` lines in Cargo.lock and tools/dmw_export/Cargo.lock.
# When a local checkout is in use (.cargo/config.toml patches iw4L, see
# .cargo/config.toml.example) or ../iw4L exists, it also checks that checkout's HEAD and that
# its crates/ tree is unmodified. iw4L is a read-only dependency.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
iw4l="${IW4L_DIR:-$root/../iw4L}"
url="https://github.com/vladtrc/iw4L"
pinned="$(tr -d '[:space:]' < "$root/IW4L_REV")"
fail=0

err() { echo "check-iw4l-rev: $*" >&2; fail=1; }

if ! [[ "$pinned" =~ ^[0-9a-f]{40}$ ]]; then
    err "IW4L_REV must hold a full 40-character commit hash, found '$pinned'"
    exit 1
fi

for manifest in "$root/Cargo.toml" "$root/tools/dmw_export/Cargo.toml"; do
    rel="${manifest#"$root"/}"
    lines="$(grep -E "git = \"$url\"" "$manifest" || true)"
    if [ -z "$lines" ]; then
        err "$rel has no iw4L git dependencies"
        continue
    fi
    while IFS= read -r line; do
        rev="$(sed -nE 's/.*rev = "([0-9a-f]+)".*/\1/p' <<<"$line")"
        name="${line%% *}"
        if [ "$rev" != "$pinned" ]; then
            err "$rel: $name has rev '${rev:-<none>}', IW4L_REV pins $pinned"
        fi
    done <<<"$lines"
done

override=0
if [ -f "$root/.cargo/config.toml" ] && grep -qF "[patch.\"$url\"]" "$root/.cargo/config.toml"; then
    override=1
fi

for lock in "$root/Cargo.lock" "$root/tools/dmw_export/Cargo.lock"; do
    rel="${lock#"$root"/}"
    [ -f "$lock" ] || { err "$rel is missing"; continue; }
    sources="$(grep -E "^source = \"git\+$url" "$lock" || true)"
    if [ -z "$sources" ]; then
        if [ "$override" = 1 ]; then
            echo "check-iw4l-rev: $rel resolves iw4L from the local override (not committable)"
        else
            err "$rel has no iw4L git sources (run cargo metadata to refresh it)"
        fi
        continue
    fi
    bad="$(grep -vF "#$pinned\"" <<<"$sources" | sort -u || true)"
    if [ -n "$bad" ]; then
        err "$rel locks iw4L at another commit:"$'\n'"$bad"
    fi
done

if [ -d "$iw4l" ] && git -C "$iw4l" rev-parse --git-dir >/dev/null 2>&1; then
    actual="$(git -C "$iw4l" rev-parse HEAD)"
    dirty="$(git -C "$iw4l" status --porcelain crates)"
    problem=""
    [ "$actual" != "$pinned" ] && problem="iw4L checkout $iw4l is at $actual, IW4L_REV pins $pinned"
    [ -n "$dirty" ] && problem="${problem:+$problem; }iw4L checkout $iw4l has uncommitted changes under crates/:"$'\n'"$dirty"
    if [ -n "$problem" ]; then
        if [ "$override" = 1 ]; then
            err "$problem"
        else
            echo "check-iw4l-rev: note (not used by the build, no .cargo/config.toml override): $problem" >&2
        fi
    fi
elif [ "$override" = 1 ]; then
    err ".cargo/config.toml patches iw4L but $iw4l is not a git checkout"
fi

if [ "$fail" != 0 ]; then
    exit 1
fi
echo "check-iw4l-rev: iw4L pinned at ${pinned:0:7} in both manifests and lockfiles$([ "$override" = 1 ] && echo ' (local override active)')"
