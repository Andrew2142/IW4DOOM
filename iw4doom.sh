#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# IW4DOOM: the one command (docs/specs/14-packaging-and-release.md section 5.2, as amended by
# sheet 15 section 11). Put your game files in games/ and run it, every time:
#
#   ./iw4doom.sh [options] [game arguments]
#
# The first run checks your game files and this machine, offers to install what is missing,
# builds the exporter, turns your Modern Warfare 2 (2009) multiplayer install into a pack under
# packs/mw2, builds the game, runs a headless smoke test and starts the game. Later runs only
# check that everything is up to date and start the game within seconds.
#
# Linux only (Windows: iw4doom.bat). Installs happen only after you answer yes at a prompt;
# without a terminal to ask on, it prints the commands and stops. Full output: iw4doom.log.
set -euo pipefail

ROOT="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
CALLER_PWD="$PWD"
cd "$ROOT"

LOG="$ROOT/iw4doom.log"
STEPS=8
MIN_RUST="1.97.1"
EXPORT_ZONE="mp_rust"
# IWAD preference when several are present (sheet 14 section 5.1).
IWAD_ORDER=(doom2.wad doom.wad plutonia.wad tnt.wad freedoom2.wad freedoom1.wad doom1.wad)

usage() {
    cat <<'EOF'
Usage: ./iw4doom.sh [options] [game arguments]

Checks and builds what is needed (the first run takes 15-20 minutes), then starts the game.

  --list          List the Doom IWADs in games/doom/ and exit.
  --iwad NAME     Play this IWAD (a name from --list, or a path) instead of the preferred one.
  --games DIR     Folder with your game files (default: games/ in this repository).
                  It must contain iw4/ (MW2 multiplayer install) and doom/ (Doom IWADs);
                  see games/README.md.
  --reexport      Export the MW2 pack again even if packs/mw2 is up to date.
  --clean         Remove previous build output first (cargo clean), then do everything,
                  including the export.
  --setup-only    Do everything except starting the game.
  -h, --help      Show this help.

Anything else goes to the game, for example: ./iw4doom.sh -warp 1 1 -skill 4
EOF
}

GAMES="$ROOT/games"
REEXPORT=0
CLEAN=0
LIST=0
SETUP_ONLY=0
IWAD_ARG=""
GAME_ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --games)
            [ $# -ge 2 ] || { echo "iw4doom: --games needs a directory" >&2; exit 1; }
            GAMES="$2"
            shift 2
            ;;
        --games=*) GAMES="${1#--games=}"; shift ;;
        --iwad)
            [ $# -ge 2 ] || { echo "iw4doom: --iwad needs a name or path" >&2; exit 1; }
            IWAD_ARG="$2"
            shift 2
            ;;
        --iwad=*) IWAD_ARG="${1#--iwad=}"; shift ;;
        --list) LIST=1; shift ;;
        --reexport) REEXPORT=1; shift ;;
        --clean) CLEAN=1; REEXPORT=1; shift ;;
        --setup-only) SETUP_ONLY=1; shift ;;
        -h|--help) usage; exit 0 ;;
        --) shift; GAME_ARGS+=("$@"); break ;;
        *) GAME_ARGS+=("$1"); shift ;;
    esac
done
# Absolute, without requiring it to exist (step 1 reports a missing folder).
case "$GAMES" in
    /*) ;;
    *) GAMES="$CALLER_PWD/$GAMES" ;;
esac
GAMES="${GAMES%/}"

# Shows a path relative to the repository when it is inside it.
show() {
    case "$1" in
        "$ROOT"/*) printf '%s' "${1#"$ROOT"/}" ;;
        *) printf '%s' "$1" ;;
    esac
}

log() { printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG"; }
step() {
    printf '[%s/%s] %s\n' "$1" "$STEPS" "$2"
    log "[$1/$STEPS] $2"
}
info() { printf '      %s\n' "$*"; log "      $*"; }
fail() {
    local line
    for line in "$@"; do
        printf '      %s\n' "$line" >&2
        log "      $line"
    done
    printf '      Details: %s\n' "$(show "$LOG")" >&2
    exit 1
}
# Runs a command with its output appended to the log; returns the command's status.
run_logged() {
    log "\$ $*"
    local status=0
    "$@" >> "$LOG" 2>&1 || status=$?
    log "exit status $status"
    return "$status"
}
# True when we may ask the user a question: stdin is a terminal and this is not CI.
interactive() {
    [ -t 0 ] && [ -z "${CI:-}" ] && [ -z "${IW4DOOM_NONINTERACTIVE:-}" ]
}
# Asks "QUESTION [Y/n]"; true on yes (Enter counts as yes).
confirm() {
    local answer=""
    printf '      %s [Y/n] ' "$1"
    read -r answer || answer="n"
    log "      $1 [Y/n] -> '${answer}'"
    case "$answer" in
        ""|y|Y|yes|Yes|YES) return 0 ;;
        *) return 1 ;;
    esac
}

# Every file in DIR whose first four bytes are "IWAD", one per line.
list_iwads() {
    local f
    [ -d "$1" ] || return 0
    for f in "$1"/*; do
        [ -f "$f" ] || continue
        if [ "$(head -c 4 "$f" 2>/dev/null)" = "IWAD" ]; then
            printf '%s\n' "$f"
        fi
    done
}

# The preferred IWAD among list_iwads, by IWAD_ORDER (case-insensitive), then any other.
pick_iwad() {
    local want f base
    local -a found=()
    mapfile -t found < <(list_iwads "$1")
    for want in "${IWAD_ORDER[@]}"; do
        for f in "${found[@]}"; do
            base="$(basename "$f")"
            if [ "${base,,}" = "$want" ]; then
                printf '%s\n' "$f"
                return 0
            fi
        done
    done
    if [ ${#found[@]} -gt 0 ]; then
        printf '%s\n' "${found[0]}"
    fi
    return 0
}

if [ "$(uname -s)" != "Linux" ]; then
    echo "iw4doom: this script supports Linux only (this is $(uname -s))." >&2
    echo "iw4doom: on Windows run iw4doom.bat; macOS is not supported yet." >&2
    exit 1
fi

# ---------------------------------------------------------------------------------------------
# --list
# ---------------------------------------------------------------------------------------------
if [ "$LIST" = 1 ]; then
    default="$(pick_iwad "$GAMES/doom")"
    if [ -z "$default" ]; then
        echo "No IWADs in $(show "$GAMES/doom")/ (see games/README.md)"
        exit 1
    fi
    echo "IWADs in $(show "$GAMES/doom")/:"
    while IFS= read -r f; do
        if [ "$f" = "$default" ]; then
            echo "  $(basename "$f")  (default)"
        else
            echo "  $(basename "$f")"
        fi
    done < <(list_iwads "$GAMES/doom")
    exit 0
fi

: > "$LOG"
log "IW4DOOM in $ROOT, games folder $GAMES"

# ---------------------------------------------------------------------------------------------
# 1. Inputs
# ---------------------------------------------------------------------------------------------
step 1 "Checking your game files in $(show "$GAMES")/"
problems=()
iw4="$GAMES/iw4"
if [ ! -d "$iw4" ]; then
    problems+=("missing: $(show "$iw4")/ - your Modern Warfare 2 (2009) MULTIPLAYER install, a copy or a symlink to the folder with iw4mp.exe, main/ and zone/")
elif [ ! -d "$iw4/zone" ]; then
    problems+=("$(show "$iw4")/ has no zone/ folder - point it at the MW2 install folder itself (the one with iw4mp.exe)")
else
    common="$(find -L "$iw4/zone" -iname 'common_mp.ff' -print -quit 2>/dev/null || true)"
    zone="$(find -L "$iw4/zone" -iname "$EXPORT_ZONE.ff" -print -quit 2>/dev/null || true)"
    if [ -z "$common" ]; then
        problems+=("$(show "$iw4")/zone has no common_mp.ff - the multiplayer install is needed (single-player only has common.ff)")
    elif [ -z "$zone" ]; then
        problems+=("$(show "$iw4")/zone has no $EXPORT_ZONE.ff - the exporter loads that multiplayer map; check the install is complete")
    else
        log "MW2: $common, $zone"
    fi
fi
if [ -n "$IWAD_ARG" ]; then
    IWAD=""
    iwad_path="$IWAD_ARG"
    case "$iwad_path" in /*) ;; *) iwad_path="$CALLER_PWD/$iwad_path" ;; esac
    if [ -f "$iwad_path" ]; then
        IWAD="$(readlink -f "$iwad_path")"
    else
        while IFS= read -r f; do
            base="$(basename "$f")"
            if [ "${base,,}" = "${IWAD_ARG,,}" ] || [ "${base,,}" = "${IWAD_ARG,,}.wad" ]; then
                IWAD="$f"
                break
            fi
        done < <(list_iwads "$GAMES/doom")
    fi
    if [ -z "$IWAD" ]; then
        problems+=("no IWAD '$IWAD_ARG' in $(show "$GAMES/doom")/ (./iw4doom.sh --list shows what is there)")
    fi
else
    IWAD="$(pick_iwad "$GAMES/doom")"
    if [ -z "$IWAD" ]; then
        problems+=("missing: a Doom IWAD in $(show "$GAMES/doom")/ - doom2.wad, doom.wad, plutonia.wad, tnt.wad, doom1.wad (shareware) or Freedoom's freedoom1.wad / freedoom2.wad")
    fi
fi
if [ ${#problems[@]} -gt 0 ]; then
    fail "${problems[@]}" "See $(show "$ROOT/games/README.md") for the folder layout."
fi
info "MW2 multiplayer install: $(show "$iw4")"
iwad_note=""
if [ -z "$IWAD_ARG" ] && [ "$(list_iwads "$GAMES/doom" | wc -l)" -gt 1 ]; then
    iwad_note=" (the preferred one; --list shows the others, --iwad NAME picks one)"
fi
info "Doom IWAD: $(show "$IWAD")$iwad_note"
SF2=""
if [ -f "$GAMES/soundfont.sf2" ]; then
    SF2="$GAMES/soundfont.sf2"
    info "Soundfont: $(show "$SF2")"
else
    info "No soundfont.sf2: the game runs without music (optional, see games/README.md)"
fi

# ---------------------------------------------------------------------------------------------
# 2. Rust
# ---------------------------------------------------------------------------------------------
step 2 "Checking the Rust toolchain"
rustup_line="curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
if ! command -v cargo >/dev/null 2>&1 && ! command -v rustup >/dev/null 2>&1 \
    && [ -f "$HOME/.cargo/env" ]; then
    # Installed earlier but this shell has not picked up ~/.cargo/bin yet.
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1 && ! command -v rustup >/dev/null 2>&1; then
    if interactive && command -v curl >/dev/null 2>&1; then
        info "Rust is not installed. It can be installed now with the official installer"
        info "(https://rustup.rs; it goes into ~/.rustup and ~/.cargo, no sudo):"
        info "  $rustup_line -s -- -y"
        if confirm "Install Rust now?"; then
            log "\$ $rustup_line -s -- -y"
            status=0
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y >> "$LOG" 2>&1 \
                || status=$?
            log "exit status $status"
            [ "$status" = 0 ] || fail "The Rust installer failed (exit $status)."
            # shellcheck disable=SC1091
            . "$HOME/.cargo/env"
            info "Rust installed."
        else
            fail "Rust is needed. Install it yourself with:" "  $rustup_line" \
                "then open a new terminal and run ./iw4doom.sh again."
        fi
    else
        fail "Rust is not installed. Install it with the official installer (https://rustup.rs):" \
            "  $rustup_line" \
            "then open a new terminal (or run: . \"\$HOME/.cargo/env\") and run ./iw4doom.sh again."
    fi
fi
if command -v rustup >/dev/null 2>&1; then
    channel="$(sed -nE 's/^channel *= *"([^"]+)".*/\1/p' rust-toolchain.toml | head -n 1)"
    channel="${channel:-stable}"
    if rustup run "$channel" rustc --version >/dev/null 2>&1; then
        log "toolchain $channel already installed"
    else
        info "Installing the Rust $channel toolchain (rust-toolchain.toml)..."
        run_logged rustup toolchain install "$channel" --profile minimal -c rustfmt -c clippy \
            || fail "rustup could not install the $channel toolchain."
    fi
elif ! command -v cargo >/dev/null 2>&1; then
    fail "rustup is installed but cargo is not on PATH; open a new terminal or run: . \"\$HOME/.cargo/env\""
fi
rustc_version="$(rustc --version 2>/dev/null | awk '{print $2}')"
if [ -z "$rustc_version" ]; then
    fail "rustc does not run. Reinstall Rust with:" "  $rustup_line"
fi
lowest="$(printf '%s\n%s\n' "$MIN_RUST" "${rustc_version%%-*}" | sort -V | head -n 1)"
if [ "$lowest" != "$MIN_RUST" ]; then
    fail "rustc $rustc_version is too old; IW4DOOM needs $MIN_RUST or newer. Update with:" \
        "  rustup update stable"
fi
info "rustc $rustc_version"

# ---------------------------------------------------------------------------------------------
# 3. System libraries
# ---------------------------------------------------------------------------------------------
step 3 "Checking system build tools and libraries"
missing_libs() {
    missing=()
    command -v cc >/dev/null 2>&1 || missing+=("a C compiler (cc)")
    if command -v pkg-config >/dev/null 2>&1; then
        for lib in alsa wayland-client xkbcommon libudev; do
            pkg-config --exists "$lib" 2>/dev/null || missing+=("$lib development files")
        done
    else
        missing+=("pkg-config")
    fi
}
missing_libs
if [ ${#missing[@]} -gt 0 ]; then
    pm=()
    install=()
    yes_flag=""
    if command -v apt-get >/dev/null 2>&1; then
        pm=(sudo apt install)
        install=(build-essential pkg-config libasound2-dev libwayland-dev libxkbcommon-dev libudev-dev)
        yes_flag="-y"
    elif command -v dnf >/dev/null 2>&1; then
        pm=(sudo dnf install)
        install=(gcc pkgconf-pkg-config alsa-lib-devel wayland-devel libxkbcommon-devel systemd-devel)
        yes_flag="-y"
    elif command -v pacman >/dev/null 2>&1; then
        pm=(sudo pacman -S --needed)
        install=(base-devel pkgconf alsa-lib wayland libxkbcommon systemd-libs)
        yes_flag="--noconfirm"
    elif command -v zypper >/dev/null 2>&1; then
        pm=(sudo zypper install)
        install=(gcc pkg-config alsa-devel wayland-devel libxkbcommon-devel libudev-devel)
        yes_flag="-y"
    fi
    lines=("Missing: $(IFS=,; echo "${missing[*]}" | sed 's/,/, /g')")
    if [ ${#install[@]} -eq 0 ]; then
        lines+=("Install a C compiler, pkg-config and the ALSA, Wayland, xkbcommon and udev development packages with your package manager.")
        fail "${lines[@]}" "Then run ./iw4doom.sh again."
    fi
    if interactive; then
        for line in "${lines[@]}"; do info "$line"; done
        info "They can be installed now with (sudo asks for your password):"
        info "  ${pm[*]} $yes_flag ${install[*]}"
        if confirm "Install now?"; then
            log "\$ ${pm[*]} $yes_flag ${install[*]}"
            status=0
            "${pm[@]}" "$yes_flag" "${install[@]}" || status=$?
            log "exit status $status"
            [ "$status" = 0 ] || fail "The package install failed (exit $status)."
            missing_libs
            if [ ${#missing[@]} -gt 0 ]; then
                fail "Still missing after the install: $(IFS=,; echo "${missing[*]}" | sed 's/,/, /g')"
            fi
        else
            fail "Install them yourself with:" "  ${pm[*]} ${install[*]}" "Then run ./iw4doom.sh again."
        fi
    else
        fail "${lines[@]}" "Install them with:" "  ${pm[*]} ${install[*]}" "Then run ./iw4doom.sh again."
    fi
fi
info "cc, pkg-config, alsa, wayland-client, xkbcommon, libudev found"

if [ "$CLEAN" = 1 ]; then
    info "--clean: removing previous build output"
    run_logged cargo clean || true
    run_logged cargo clean --manifest-path tools/dmw_export/Cargo.toml || true
fi

# Modification time and size of a file, or "none"; used to see whether cargo rebuilt it.
stamp_of() { stat -c '%Y %s' "$1" 2>/dev/null || echo none; }

# ---------------------------------------------------------------------------------------------
# 4. Build the exporter
# ---------------------------------------------------------------------------------------------
EXPORTER="$ROOT/tools/dmw_export/target/release/dmw_export"
if [ -x "$EXPORTER" ]; then
    step 4 "Building the exporter (up to date unless the code changed)"
else
    step 4 "Building the exporter (tools/dmw_export; the first build takes about 10-15 minutes)"
fi
run_logged cargo build --release --locked --manifest-path tools/dmw_export/Cargo.toml \
    || fail "The exporter did not build. The compiler output is in $(show "$LOG")."
log "exporter: $EXPORTER"

# ---------------------------------------------------------------------------------------------
# 5. Export the pack
# ---------------------------------------------------------------------------------------------
PACK="$ROOT/packs/mw2"
manifest="$PACK/manifest.json"
json_field() { sed -nE "s/^  \"$1\": \"([^\"]*)\",?$/\1/p" "$2" | head -n 1; }
want_generator="$("$EXPORTER" --version 2>/dev/null | head -n 1)"
want_rev="$(tr -d '[:space:]' < IW4L_REV | cut -c1-7)"
reason=""
if [ "$REEXPORT" = 1 ]; then
    reason="--reexport"
elif [ ! -f "$manifest" ]; then
    reason="no pack yet"
else
    have_generator="$(json_field generator "$manifest")"
    have_rev="$(json_field iw4l_rev "$manifest")"
    if [ "$have_generator" != "$want_generator" ]; then
        reason="pack made by '$have_generator', exporter is '$want_generator'"
    elif [ "$have_rev" != "$want_rev" ]; then
        reason="pack made with iw4L $have_rev, exporter uses $want_rev"
    fi
fi

if [ -z "$reason" ]; then
    step 5 "Exporting your MW2 weapons to packs/mw2 (up to date; --reexport to redo)"
else
    step 5 "Exporting your MW2 weapons to packs/mw2 ($reason; about a minute)"
    out_file="$(mktemp)"
    status=0
    log "\$ DMW_MW2_GAMES=$GAMES $EXPORTER --out $PACK --zone $EXPORT_ZONE"
    DMW_MW2_GAMES="$GAMES" "$EXPORTER" --out "$PACK" --zone "$EXPORT_ZONE" > "$out_file" 2>&1 || status=$?
    cat "$out_file" >> "$LOG"
    log "exit status $status"
    case "$status" in
        0)
            while IFS= read -r line; do info "$line"; done < <(grep -E '^(ok|FAILED) ' "$out_file" || true)
            ;;
        2)
            rm -f "$out_file"
            fail "Export failed: MW2 multiplayer was not found at $(show "$iw4")." \
                "Check that it is the install folder (with iw4mp.exe, main/ and zone/)."
            ;;
        3)
            rm -f "$out_file"
            fail "Export failed: iw4L refused to load the game files (exit 3)." \
                "The exporter output is in $(show "$LOG"); iw4L's own logs, if any, are under" \
                "$(show "$ROOT/packs/.dmw_export-cache/iw4l-artifacts/logs")/."
            ;;
        4)
            mapfile -t errs < <(grep -E '^error: ' "$out_file" | sed 's/^error: //' || true)
            rm -f "$out_file"
            fail "Export failed: a weapon could not be exported:" "${errs[@]/#/  }" \
                "The previous pack (if any) is unchanged."
            ;;
        5)
            rm -f "$out_file"
            fail "Export failed: the pack could not be written to $(show "$PACK") (disk full or no permission?)." \
                "The previous pack (if any) is unchanged."
            ;;
        *)
            rm -f "$out_file"
            fail "Export failed with exit status $status."
            ;;
    esac
    rm -f "$out_file"
fi

# ---------------------------------------------------------------------------------------------
# 6. Build the game
# ---------------------------------------------------------------------------------------------
GAME="$ROOT/target/release/iw4doom"
if [ -x "$GAME" ]; then
    step 6 "Building the game (up to date unless the code changed)"
else
    step 6 "Building the game (the first build takes a few minutes)"
fi
run_logged cargo build --release --locked --bin iw4doom \
    || fail "The game did not build. The compiler output is in $(show "$LOG")."

# ---------------------------------------------------------------------------------------------
# 7. Smoke test (only when the game or the pack changed since the last passing one)
# ---------------------------------------------------------------------------------------------
SMOKE_STAMP="$ROOT/target/release/.iw4doom-smoke-ok"
smoke_key="game $(stamp_of "$GAME"); pack $(stamp_of "$manifest")"
if [ -f "$SMOKE_STAMP" ] && [ "$(cat "$SMOKE_STAMP")" = "$smoke_key" ]; then
    step 7 "Smoke test (passed before; the game and the pack are unchanged)"
else
    step 7 "Smoke test: 70 tics headless on $(basename "$IWAD") with your pack"
    smoke_file="$(mktemp)"
    status=0
    log "\$ DMW_SMOKE_IWAD=$IWAD DMW_SMOKE_PACK=$PACK cargo test --release --locked -p room --test setup_smoke -- --nocapture"
    DMW_SMOKE_IWAD="$IWAD" DMW_SMOKE_PACK="$PACK" \
        cargo test --release --locked -p room --test setup_smoke -- --nocapture > "$smoke_file" 2>&1 || status=$?
    cat "$smoke_file" >> "$LOG"
    log "exit status $status"
    if [ "$status" != 0 ] || ! grep -q '^smoke: ok' "$smoke_file"; then
        mapfile -t why < <(grep -E '^smoke: ' "$smoke_file" | head -n 3 || true)
        rm -f "$smoke_file"
        rm -f "$SMOKE_STAMP"
        if [ ${#why[@]} -eq 0 ]; then
            why=("the smoke test did not run (build error?)")
        fi
        fail "Smoke test failed: MW2 mode did not turn on." "${why[@]}"
    fi
    info "$(grep -m1 '^smoke: ok' "$smoke_file" | sed 's/^smoke: ok: //')"
    rm -f "$smoke_file"
    printf '%s\n' "$smoke_key" > "$SMOKE_STAMP"
fi

# ---------------------------------------------------------------------------------------------
# 8. Start the game
# ---------------------------------------------------------------------------------------------
# -mb 128: works around the engine's Z_FreeTags 2000-block limit (sheet 13 section 11).
launch=("$GAME" -iwad "$IWAD" -mw2pack "$PACK" -mb 128)
[ -n "$SF2" ] && launch+=(-sf2 "$SF2")
launch+=("${GAME_ARGS[@]}")
log "launch: $(printf '%q ' "${launch[@]}")"
if [ "$SETUP_ONLY" = 1 ]; then
    step 8 "Ready (--setup-only: not starting the game)"
    info "Start it with: ./iw4doom.sh"
    log "done (setup only)"
    exit 0
fi
step 8 "Starting IW4DOOM"
log "done"
exec "${launch[@]}"
