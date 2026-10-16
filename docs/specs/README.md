# DOOMMW2 Build Specs

DOOMMW2 is the `room` Rust Doom engine with Call of Duty: Modern Warfare 2 (2009) gunplay grafted in: MW2's real weapon state machine, spread, recoil, sway, ADS, reload timing and damage falloff, driven by `iw4L`'s Rust port of that logic, and real 3D MW2 viewmodels drawn by a software rasterizer that pushes every pixel through Doom's own palette and light tables.

These sheets describe the full build. Each sheet is sized to be handed to one build agent.

## How to assign work

1. Every agent reads, in order: `00-overview-and-contracts.md`, `01-engineering-standards.md`, then its own sheet. Agents touching engine code also read `/AGENTS.md` at the repo root (created by sheet 02).
2. Agents code against the contracts in sheet 00 and stub neighbours in tests; they do not wait for other sheets' implementations.
3. An agent may not change a contract in sheet 00 on its own. It writes the change into its sheet's "Contract changes requested" section and stops on that item; the lead updates 00 (as a new Revision section) first.
4. A sheet is done when every item in its "Acceptance criteria" is checked and the Definition of Done in sheet 01 §5.12 holds.
5. Nothing in this repository may contain MW2 game data. Everything that needs MW2 data is either tested against the synthetic fixture pack (sheet 03 §5.9) or listed as a manual QA step (sheet 13 §5.6).

## Sheets and build waves

| Wave | Sheets | Can run in parallel |
|---|---|---|
| 1 | 02 Workspace scaffold | — |
| 2 | 03 Asset pack format + fixture, 05 Input and controls, 06 Freelook and presentation | yes |
| 3 | 04 MW2 exporter, 07 Weapon simulation bridge, 11 Software rasterizer (crate half, §5.1–5.8) | yes |
| 4 | 08 Doom integration, 09 Ballistics and damage, 10 Viewmodel animation | yes (08 owns the shared hook file; 09/10 add calls through 08's hook table) |
| 5 | 11 (engine half, §6), 12 HUD, crosshair and audio | yes |
| 6 | 13 Full test catalogue + manual QA with real MW2 data; 04 MW2 exporter verified against a real install | — |
| 7 | 14 Packaging, one-command setup and GitHub release | — |
| 8 | 15 Windows support | — |
| 9 | 16 Uncapped frame rate (done), 17 MW2 map conversion (stage 1: Rust) | — |

Sheet 01 is a reference for every wave, not a build task. Sheet 13 §5.1–5.4 (test helpers, headless harness extensions) is built in wave 2 alongside 03, because waves 3–5 need it.

| Sheet | File |
|---|---|
| 00 | [Overview and shared contracts](00-overview-and-contracts.md) |
| 01 | [Engineering standards](01-engineering-standards.md) |
| 02 | [Workspace scaffold and tooling](02-workspace-scaffold.md) |
| 03 | [Asset pack format and synthetic fixture](03-asset-pack-format.md) |
| 04 | [MW2 asset exporter](04-mw2-exporter.md) |
| 05 | [Input, mouse look and controls](05-input-and-controls.md) |
| 06 | [Freelook renderer, FOV zoom and presentation](06-freelook-and-presentation.md) |
| 07 | [Weapon simulation bridge](07-weapon-sim-bridge.md) |
| 08 | [Doom integration: weapons, inventory, ammo, movement, saves](08-doom-integration.md) |
| 09 | [Ballistics and damage](09-ballistics-and-damage.md) |
| 10 | [Viewmodel animation and placement](10-viewmodel-animation.md) |
| 11 | [Software viewmodel rasterizer and retro pipeline](11-viewmodel-rasterizer.md) |
| 12 | [HUD, crosshair, scope overlays and audio](12-hud-and-audio.md) |
| 13 | [Testing, fixtures and QA](13-testing-and-qa.md) |
| 14 | [Packaging, one-command setup and GitHub release](14-packaging-and-release.md) |
| 15 | [Windows support](15-windows-support.md) |
| 16 | [Uncapped frame rate](16-uncapped-framerate.md) |
| 17 | [MW2 map conversion (stage 1: Rust)](17-mw2-map-conversion.md) |

## Dependency picture

```text
                 iw4L (read-only, pinned rev, path deps)
                 ├─ weapon_iw4, movement_iw4, playerstate_iw4, math_iw4,
                 │  asset_iw4, xmodel_runtime, anim_iw4      ──► runtime crates (no Bevy)
                 └─ assets, asset_game, asset_model, asset_audio, …  ──► tools/dmw_export only (Bevy)

 MW2 install ─► tools/dmw_export (04) ─► packs/mw2/  (gitignored, dmw_format v1, sheet 03)
                                              │
 dmw_format (03) ◄────────────────────────────┤
     ▲                                        ▼
 dmw_weapons (07, 10) ──► room engine fork: room/src/dmw/* (05, 06, 08, 09, 11-engine, 12)
 dmw_viewmodel (11)   ──┘
```
