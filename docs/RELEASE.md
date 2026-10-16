# Release checklist

Sheet 14 section 5.7. Work through it before the first public push and before every tag.
Publishing is an outward-facing action: it happens only after the owner has confirmed the
repository name, the visibility and that every box below is ticked.

## 1. Nothing copyrighted is in the repository or its history

- [ ] `git status --short --ignored games packs` lists only ignored files (`games/README.md` is
      the one tracked file under `games/`; nothing under `packs/` is tracked).
- [ ] No game data, packs, IWADs, `.env`, `iw4l-artifacts/` or MW2-derived images anywhere in
      history. Check the largest blobs and every path that ever existed:

      git rev-list --objects --all \
        | git cat-file --batch-check='%(objecttype) %(objectname) %(objectsize) %(rest)' \
        | awk '$1 == "blob"' | sort -k3 -n -r | head -n 20

      git log --all --name-only --format= | sort -u \
        | grep -Ei '\.(wad|ff|iwd|sf2|pk3|wav|dds|iwi)$|^packs/|^games/|iw4l-artifacts|(^|/)\.env$'

      The only expected hits are the old Git LFS pointer files `doom1.wad` and
      `soundfonts/SC55Soundfont-1.2b/SC-55 SoundFont v1.2b.sf2` (about 130 bytes of text each,
      removed by sheet 14). Any other hit blocks the release.
- [ ] Every image under `docs/images/` and `room/tests/golden/` was rendered from the fixture pack
      or Freedoom only (never from an exported MW2 pack). `target/test-artifacts/` is never
      committed.
- [ ] README and docs use no game logos or official art.

## 2. A clean clone builds without siblings or LFS

In a temporary directory with no `../room` and no `../iw4L` next to it:

    git clone <repo> iw4doom && cd iw4doom
    cargo check --workspace --exclude c2rust-intermediate
    cargo tree -p weapon_iw4          # resolves from https://github.com/vladtrc/iw4L
    tools/check-iw4l-rev.sh
    tools/fetch-freedoom.sh
    DMW_IWAD=wads/freedoom1.wad cargo test --workspace --exclude c2rust-intermediate -- --skip demo_playthrough

- [ ] All of the above pass.
- [ ] CI (`.github/workflows/ci.yml`) is green on the branch; run the manual exporter job once.

## 3. End-to-end on the maintainer's machine

- [ ] `games/` holds the real files (MW2 multiplayer install, at least one IWAD).
- [ ] `./setup.sh --reexport` passes all eight steps; the smoke test reports MW2 mode on.
- [ ] `./play.sh --list` shows the IWADs; `./play.sh` plays: weapons raise, fire, reload, aim.
- [ ] `DMW_IWAD=wads/freedoom1.wad cargo test --workspace --exclude c2rust-intermediate` passes,
      including `demo_playthrough` (needs `games/doom/doom1.wad`, the shareware IWAD).

## 4. Documents

- [ ] README legal section present and current; NOTICE lists every Apache-2.0 file.
- [ ] Repository name, description and visibility chosen by the owner. Suggested description:
      "Unofficial: Modern Warfare 2 (2009) gunplay and real 3D viewmodels in the classic Doom
      engine. Bring your own game files."

## 5. Publish (owner-confirmed only)

    gh repo create Andrew2142/IW4DOOM --private --source . --remote origin \
      --description "Unofficial: Modern Warfare 2 (2009) gunplay and real 3D viewmodels in the classic Doom engine. Bring your own game files."
    git push -u origin main
    git tag -a v0.1.0 -m "IW4DOOM 0.1.0" && git push origin v0.1.0

Then: add the demo video to the README (`<!-- DEMO VIDEO -->` comment), set the repository
topics, and switch visibility to public when the owner says so.
