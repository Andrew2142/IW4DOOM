#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Replaced by ./iw4doom.sh, which sets up and starts the game in one command. Kept so older
# instructions still work: this does the setup part only.
exec "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/iw4doom.sh" --setup-only "$@"
