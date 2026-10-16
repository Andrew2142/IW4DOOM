#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Failure-mode tests for ./iw4doom.sh (sheet 14 T14.2-T14.4, as amended by sheet 15 section 11).
# Needs no game data: the fake MW2 install is two empty files, the IWAD is Freedoom
# (tools/fetch-freedoom.sh). Every case must stop before building anything. Runs in CI.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
freedoom="$root/wads/freedoom1.wad"
[ -f "$freedoom" ] || { echo "test-launcher: $freedoom missing; run tools/fetch-freedoom.sh" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/empty" "$tmp/fd/doom" "$tmp/fake/doom" "$tmp/fake/iw4/zone/english" "$tmp/home"
cp "$freedoom" "$tmp/fd/doom/"
cp "$freedoom" "$tmp/fake/doom/"
: > "$tmp/fake/iw4/zone/english/common_mp.ff"
: > "$tmp/fake/iw4/zone/english/mp_rust.ff"

failures=0
# Never stop at an install prompt (stdin is /dev/null as well).
export IW4DOOM_NONINTERACTIVE=1
# expect NAME WANT_EXIT "needle|needle..." "unwanted|..." -- command...
expect() {
    local name="$1" want="$2" needles="$3" unwanted="$4"
    shift 5
    local out status=0 n
    out="$("$@" < /dev/null 2>&1)" || status=$?
    local ok=1
    [ "$status" = "$want" ] || { ok=0; echo "  exit status $status, expected $want"; }
    IFS='|' read -r -a want_list <<< "$needles"
    for n in "${want_list[@]}"; do
        [ -z "$n" ] && continue
        grep -qF -- "$n" <<< "$out" || { ok=0; echo "  missing from output: $n"; }
    done
    IFS='|' read -r -a bad_list <<< "$unwanted"
    for n in "${bad_list[@]}"; do
        [ -z "$n" ] && continue
        if grep -qF -- "$n" <<< "$out"; then ok=0; echo "  unexpected in output: $n"; fi
    done
    if [ "$ok" = 1 ]; then
        echo "ok    $name"
    else
        echo "FAIL  $name; output was:"
        sed 's/^/      | /' <<< "$out"
        failures=$((failures + 1))
    fi
}

expect "--help prints usage" 0 "Usage: ./iw4doom.sh|--setup-only" "" -- \
    ./iw4doom.sh --help
expect "empty games dir names both missing items" 1 \
    "[1/8]|missing: your Modern Warfare 2|missing: a Doom IWAD|games/README.md" "[2/8]" -- \
    ./iw4doom.sh --games "$tmp/empty"
expect "Freedoom only names the missing MW2 install" 1 "missing: your Modern Warfare 2" "a Doom IWAD|[2/8]" -- \
    ./iw4doom.sh --games "$tmp/fd"
expect "--list shows the IWAD" 0 "freedoom1.wad  (default)" "" -- \
    ./iw4doom.sh --games "$tmp/fd" --list
expect "--iwad with an unknown name" 1 "no IWAD 'nope'" "" -- \
    ./iw4doom.sh --games "$tmp/fake" --iwad nope

if PATH=/usr/bin:/bin command -v cargo >/dev/null 2>&1; then
    echo "skip  no Rust (cargo is in /usr/bin on this machine)"
else
    expect "no Rust prints the rustup command" 1 \
        "[2/8]|Rust is not installed|https://sh.rustup.rs" "[3/8]|Install Rust now?" -- \
        env PATH=/usr/bin:/bin HOME="$tmp/home" CARGO_HOME="$tmp/home/.cargo" \
        RUSTUP_HOME="$tmp/home/.rustup" ./iw4doom.sh --games "$tmp/fake"
fi

expect "missing libraries print the install line, no prompt" 1 \
    "[3/8]|Missing:|alsa development files|Install them with:" "Install now?|[4/8]" -- \
    env PKG_CONFIG_LIBDIR="$tmp/empty" PKG_CONFIG_PATH="$tmp/empty" \
    ./iw4doom.sh --games "$tmp/fake"

if [ "$failures" -gt 0 ]; then
    echo "test-launcher: $failures case(s) failed" >&2
    exit 1
fi
echo "test-launcher: all cases passed"
