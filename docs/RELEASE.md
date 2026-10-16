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
- [ ] CI (`.github/workflows/ci.yml`) is green on the branch, including the `windows` job;
      run the manual exporter job once (both the Linux and the Windows entry).

## 3. End-to-end on the maintainer's machine

- [ ] `games/` holds the real files (MW2 multiplayer install, at least one IWAD).
- [ ] `./iw4doom.sh --reexport --setup-only` passes steps 1-7; the smoke test reports MW2 mode on.
- [ ] `./iw4doom.sh --list` shows the IWADs; `./iw4doom.sh` plays: weapons raise, fire, reload,
      aim. A second `./iw4doom.sh` starts the game within seconds (no smoke test).
- [ ] `DMW_IWAD=wads/freedoom1.wad cargo test --workspace --exclude c2rust-intermediate` passes,
      including `demo_playthrough` (needs `games/doom/doom1.wad`, the shareware IWAD).

## 3b. Windows, by hand (T15.5, the owner on a real Windows 10/11 PC with their own files)

CI proves the build and the tests on Windows; it cannot play. Before announcing Windows support:

- [ ] Fresh clone; `games\iw4` as a junction (`mklink /J`) to the Steam MW2 folder, an IWAD in
      `games\doom\`.
- [ ] Double-click `iw4doom.bat` on a machine without Rust or Build Tools: it offers the winget
      installs, and after yes finishes steps 1-8 and the game starts. Answering no prints the
      rustup-init.exe / Build Tools instructions and exits.
- [ ] `iw4doom.log` has the whole run; a second double-click starts the game within seconds.
- [ ] In the game: the mouse is captured (the log shows `mouse captured (confined)` or
      `(locked)`), sound and music play, the real MW2 guns raise, fire, reload and aim.
- [ ] Save and load a game; quit and start again with `iw4doom.bat --iwad <other>`.

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
