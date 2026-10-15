#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Downloads a pinned Freedoom release into wads/ for development and tests. Idempotent.
# Pinning matters: headless tests use map coordinates that change between Freedoom releases.
set -euo pipefail

FREEDOOM_TAG="v0.13.0"
FREEDOOM_VERSION="${FREEDOOM_TAG#v}"
FREEDOOM_ZIP_SHA256="3f9b264f3e3ce503b4fb7f6bdcb1f419d93c7b546f4df3e874dd878db9688f59"
FREEDOOM1_SHA256="7323bcc168c5a45ff10749b339960e98314740a734c30d4b9f3337001f9e703d"
FREEDOOM2_SHA256="a8772e088847032510d97ba2312406a6998f21cbab44d4ff10696faa9c0ecd4b"
URL="https://github.com/freedoom/freedoom/releases/download/${FREEDOOM_TAG}/freedoom-${FREEDOOM_VERSION}.zip"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
wads="$root/wads"

sha_of() { sha256sum "$1" | awk '{print $1}'; }

have_both() {
    [ -f "$wads/freedoom1.wad" ] && [ -f "$wads/freedoom2.wad" ] \
        && [ "$(sha_of "$wads/freedoom1.wad")" = "$FREEDOOM1_SHA256" ] \
        && [ "$(sha_of "$wads/freedoom2.wad")" = "$FREEDOOM2_SHA256" ]
}

if have_both; then
    echo "fetch-freedoom: wads/freedoom1.wad and freedoom2.wad already present (Freedoom $FREEDOOM_VERSION)"
    exit 0
fi

for tool in curl unzip sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || { echo "fetch-freedoom: '$tool' is required" >&2; exit 1; }
done

# Download to a temp dir and move into place only after verification, so a failure
# never leaves a partial WAD in wads/.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "fetch-freedoom: downloading $URL"
curl --fail --location --silent --show-error --output "$tmp/freedoom.zip" "$URL"

actual_zip="$(sha_of "$tmp/freedoom.zip")"
if [ "$actual_zip" != "$FREEDOOM_ZIP_SHA256" ]; then
    echo "fetch-freedoom: zip SHA-256 mismatch: expected $FREEDOOM_ZIP_SHA256 got $actual_zip" >&2
    exit 1
fi

unzip -q -j "$tmp/freedoom.zip" '*/freedoom1.wad' '*/freedoom2.wad' -d "$tmp/out"

if [ "$(sha_of "$tmp/out/freedoom1.wad")" != "$FREEDOOM1_SHA256" ]; then
    echo "fetch-freedoom: freedoom1.wad SHA-256 mismatch" >&2
    exit 1
fi
if [ "$(sha_of "$tmp/out/freedoom2.wad")" != "$FREEDOOM2_SHA256" ]; then
    echo "fetch-freedoom: freedoom2.wad SHA-256 mismatch" >&2
    exit 1
fi

mkdir -p "$wads"
mv "$tmp/out/freedoom1.wad" "$wads/freedoom1.wad"
mv "$tmp/out/freedoom2.wad" "$wads/freedoom2.wad"
echo "fetch-freedoom: installed Freedoom $FREEDOOM_VERSION into $wads"
