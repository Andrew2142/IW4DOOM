#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Enforces the crate dependency boundaries of docs/specs/01-engineering-standards.md section 5.3
# by scanning `cargo tree -e normal` for forbidden crates. Exits 1 naming the crate and the
# offending dependency.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# crate -> space-separated deny list. Entries are crate names matched exactly.
declare -A DENY=(
    [dmw_format]="room dmw_weapons dmw_viewmodel bevy bevy_ecs bevy_app bevy_render assets asset_game asset_model asset_anim asset_audio asset_material asset_transport asset_core frame"
    [dmw_weapons]="room dmw_viewmodel rand bevy bevy_ecs bevy_app bevy_render assets asset_game asset_model asset_anim asset_audio asset_material asset_transport asset_core frame"
    [dmw_viewmodel]="room dmw_weapons bevy bevy_ecs bevy_app bevy_render assets asset_game asset_model asset_anim asset_audio asset_material asset_transport asset_core frame"
    [room]="bevy bevy_ecs bevy_app bevy_render assets asset_game asset_model asset_anim asset_audio asset_material asset_transport asset_core frame"
)

# crate -> crates it must not list as a *direct* dependency. dmw_viewmodel must not use iw4L
# itself, but it reaches weapon_iw4 transitively through dmw_format (allowed there, 01 section 5.3).
declare -A DENY_DIRECT=(
    [dmw_viewmodel]="weapon_iw4 movement_iw4 playerstate_iw4 math_iw4 asset_iw4 trace_iw4 xmodel_runtime anim_iw4"
)

status=0
for crate in dmw_format dmw_weapons dmw_viewmodel room; do
    deps="$(cargo tree -e normal -p "$crate" --prefix none --format '{p}' | awk '{print $1}' | sort -u)"
    for banned in ${DENY[$crate]}; do
        if grep -qx -- "$banned" <<<"$deps"; then
            echo "check-deps: crate '$crate' must not depend on '$banned' (01 section 5.3)" >&2
            status=1
        fi
    done
done

for crate in "${!DENY_DIRECT[@]}"; do
    direct="$(cargo tree -e normal -p "$crate" --depth 1 --prefix none --format '{p}' | awk '{print $1}' | sort -u)"
    for banned in ${DENY_DIRECT[$crate]}; do
        if grep -qx -- "$banned" <<<"$direct"; then
            echo "check-deps: crate '$crate' must not depend directly on '$banned' (01 section 5.3)" >&2
            status=1
        fi
    done
done

if [ "$status" -eq 0 ]; then
    echo "check-deps: crate boundaries OK"
fi
exit "$status"
