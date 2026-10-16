# DOOMMW2 agent instructions

Read docs/specs/README.md, then docs/specs/00-overview-and-contracts.md, then docs/specs/01-engineering-standards.md, then your sheet.

## Ground rules
- The engine in room/src/doom is a C port. Touch it only at the hook points listed in 00 §7.5; put logic in room/src/dmw.
- Never change a #[repr(C)] struct. Extension state lives in room/src/dmw/ext.rs.
- MW2 data never enters the repository. Use the fixture pack (`-fixturepack`, `Pack::fixture()`) for tests.
- iw4L is a read-only git dependency pinned to IW4L_REV (local checkout: .cargo/config.toml.example). Run tools/check-iw4l-rev.sh if builds behave oddly.
- The public name is IW4DOOM (binary `iw4doom`, config `iw4doom.toml`); internal crate and module names stay DOOMMW2-era (`room`, `dmw_*`).
- When MW2 mode is off the engine must behave exactly like room. demo_playthrough proves it.
- Contracts are in docs/specs/00. Do not change them in code; request changes in your sheet's §11.

## Commands
- task check | task lint | task test | task run | task fetch-wads | task fixture-pack | task export (needs MW2)
- ./setup.sh (end-user build: checks, export, build, smoke test, writes play.sh)

## Porting lessons inherited from room (still true)
- Pass NUL-terminated byte strings (c"..." literals) to C-shaped functions.
- Use wrapping_add/wrapping_sub/wrapping_neg for angle_t arithmetic.
- Cast state_t pointers to *mut State before touching fields.
- Do not zero memory with ptr::write_bytes when running under ASan; use explicit loops.
