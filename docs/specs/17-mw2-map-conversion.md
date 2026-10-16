# 17 — MW2 Map Conversion (stage 1: Rust)

Owner: build agent (wave 9)
Status: implementation-ready
Depends on: 00 (Revision 12), 01, 04 (exporter, zone load), 13 (headless harness), 14/15 (launchers), 16 (uncapped renderer; unaffected but must keep passing)

Convert the MW2 multiplayer map **Rust** (`mp_rust`) from the user's own MW2 install into a playable Doom map, generated locally like the weapon pack. Nothing from MW2 enters the repository. Stage 2 (zombies mode) builds on the result and is a separate sheet.

---

## 1. Purpose

- `./iw4doom.sh --map rust` (and `iw4doom.bat --map rust`) generates the map once, then starts the game on it with the MW2 weapons.
- The layout is recognisable: the same open ground, the central tower block, the pipes, crates and walls, at Doom scale (`DOOM_PER_MW2 = 0.8`, 00 §5.1). It is a 2.5D slice of the ground level: no room-over-room, no slopes (they become steps), no ladders, no jumping.
- The map runs in the unmodified Doom renderer and physics (raised static limits aside, §5.9) with no crash, no visplane overflow and no hall-of-mirrors from every spawn point.

## 2. Scope / Out of scope

In scope:
- A pure, MW2-free crate `crates/dmw_mapgen`: height-grid → sectors/linedefs → node builder → blockmap/reject → PWAD writer, plus a validator. Fully tested on synthetic grids in CI.
- An exporter subcommand `dmw_export map --zone mp_rust` that samples the clip collision of the loaded zone into a height grid (`MapGrid`, §5.2) and saves it as `packs/maps/mp_rust.grid` (postcard), then writes WADs per IWAD (§5.6).
- Engine: allow the generated PWAD with shareware `doom1.wad` (§5.8) and raise renderer static limits (§5.9).
- Launchers: `--map NAME` / `-Map NAME` (§5.10).
- Textures, stage 1A: Doom textures from the chosen IWAD, picked per MW2 surface type (§5.5). Stage 1B (§5.11): MW2's own textures quantized into the PWAD. 1B is done only after 1A is green; if 1B blocks, ship 1A and report.

Out of scope: any other map (the code takes `--zone` so others can be tried later, but only Rust is tested and tuned); zombies, waves, monster AI (stage 2); doors, lifts and moving parts; MW2 lighting beyond the simple model in §5.4; room-over-room; GL nodes; ZDoom/UDMF features.

## 3. Facts to start from

| Fact | Source |
|---|---|
| The exporter already loads a full `assets::PreparedMatch` for `mp_rust` (it is the default `--zone`) | `tools/dmw_export/src/load.rs`, `main.rs` |
| `PreparedMatch.clip: Option<Arc<asset_world::ClipCollision>>` has `sweep_box(start, end, mins, maxs, mask) -> ClipSweepHit { fraction, normal, endpos, startsolid, allsolid, surface_flags }` and `box_sight_clear` | iw4L `asset_world/src/clip_collision.rs` |
| `PreparedMatch.prepared_map.spawns: Vec<asset_world::SpawnPoint>` (`classname`, `origin`, `angles`); DM classnames `mp_dm_spawn`, `mp_dm_spawn_start`, … | iw4L `assets/src/prepared.rs`, `asset_world/src/map_entities.rs` |
| Masks: `MASK_PLAYER_SOLID = 0x0281_0011`, `CONTENTS_SOLID = 0x1`, `CONTENTS_SKY = 0x800`; surface type = `(surface_flags >> 20) & 0x1f`, names in `movement_iw4::SURFACE_TYPE_NAMES` (31 entries: default, bark, brick, carpet, cloth, concrete, dirt, flesh, foliage, glass, grass, gravel, ice, metal, mud, paper, plaster, rock, sand, snow, water, wood, asphalt, ceramic, plastic, rubber, cushion, fruit, paintedmetal, riotshield, slush) | iw4L `sim/src/bullet_collision.rs`, `movement_iw4/src/footstep.rs`. Find the sky surface flag and water contents bit in iw4L (`SURF_SKY`, `CONTENTS_WATER`) and cite them in code |
| MW2 axes: x forward, y left, z up, yaw counter-clockwise from +x. Doom: same handedness, angle counter-clockwise from +x. Map `(x, y) → (x·0.8, y·0.8)`, no flip | 00 §5.1 |
| Doom player: radius 16, height 56, step 24, view 41. MW2 player: radius 15, height 70 | Doom source, 00 §5.1 |
| Shareware `doom1.wad` refuses any `-file` (`d_main.rs` "You cannot -file with the shareware version") | room |
| Renderer limits: `MAXVISPLANES 128` (`r_plane.rs`), `MAXDRAWSEGS 256` (`r_segs.rs`, `r_bsp.rs`), `MAXOPENINGS SCREENWIDTH*64`, `MAXVISSPRITES 128` (`r_things.rs`), `MAXSEGS 32` (`r_bsp.rs`) | room |
| A PWAD `TEXTURE1`/`PNAMES` or `F_START…F_END` replaces the IWAD's whole set, and Doom's switch list (`P_InitSwitchList`) and animated flats (`P_InitPicAnims`) then fail or index out of range | Doom texture loading |
| The user's IWADs: `games/doom/doom1.wad` (shareware), `games/doom/freedoom2.wad`; CI has Freedoom 1 and 2 | launcher |

## 4. File inventory

| Path | Content |
|---|---|
| `crates/dmw_mapgen/` (workspace member) | `grid.rs` (`MapGrid`, §5.2), `regions.rs` (§5.3), `lines.rs` (§5.4), `textures.rs` (§5.5 selection tables), `nodes.rs` (§5.7), `blockmap.rs`, `reject.rs`, `wad.rs` (§5.6), `validate.rs` (§5.7.4), `fixtures.rs` (synthetic grids for tests) |
| `tools/dmw_export/src/map/` | `sample.rs` (§5.1), `mod.rs` (subcommand, report), `textures_mw2.rs` (1B) |
| `room/src/doom/d_main.rs` | shareware gate exception (§5.8), DOOMMW2 comment |
| `room/src/doom/r_plane.rs`, `r_segs.rs`, `r_bsp.rs`, `r_things.rs` | raised limits (§5.9) |
| `room/src/dmw/test_api.rs` | render counters (visplanes, drawsegs, vissprites used last frame) |
| `room/tests/mapgen_headless.rs` | CI: synthetic maps load and render (§9) |
| `room/tests/mw2map_headless.rs` | local only, `#[ignore]` unless `packs/maps/mp_rust.grid` exists (§9) |
| `iw4doom.sh`, `iw4doom.ps1`, `tools/test-iw4doom-launcher.{sh,ps1}` | `--map` (§5.10) |
| `README.md`, `docs/INSTALL.md` | one short "Play Rust" section |
| `.gitignore` | `packs/maps/` (already under `packs/`) — verify |

## 5. Detailed design

### 5.1 Sampling the clip collision (exporter, MW2 units)

Bounds: the union of all clip brush bounds and mesh AABB roots, padded 64. Grid step `--cell 10` MW2 units (8 Doom units). For every cell centre `(x, y)`:

1. **Layers.** Trace straight down with a point box (`mins = maxs = 0`) and `MASK_PLAYER_SOLID` from `top = bounds.max.z + 16`. Record the hit `(z, normal, surface_flags)`. Then find the next layer below: step down from the hit in 4-unit increments until a point trace starting there is not `startsolid`, and trace down again. Collect up to 8 layers, top to bottom. A layer is *walkable* when `normal.z ≥ 0.7`.
2. **Ceiling per layer.** For each walkable layer, trace up from `z + 1` to `top`: no hit, or a hit with the sky flag → sky; otherwise ceiling = hit z.
3. **Choose the layer**: the lowest walkable layer with headroom `ceiling - floor ≥ 70` (or sky). None → the cell is *solid*.
4. **Stand test.** Sweep a box `mins (-4,-4,18) maxs (4,4,70)` (relative to the floor) from floor+18 to floor+19: `startsolid` → solid (catches pillars, crates, thin walls thicker than the cell).
5. **Water.** If the layer point is inside water contents, floor = water surface − 8 and mark `water`.
6. **Thin blockers between cells.** For each cell and its +x and +y neighbour (both non-solid, floor difference ≤ 24 Doom units), sweep a box `mins (-1,-1,-8) maxs (1,1,8)` at floor + 32 between the two centres. Blocked → record an edge blocker (fences, railings, chain-link) with the surface type hit.

Output `MapGrid { origin, cell_mw2, w, h, cells: Vec<Cell>, blockers: Vec<(u32 cell, Dir)> , spawns, surface names }`, `Cell { solid, floor_mw2: f32, ceil_mw2: Option<f32> /*None = sky*/, floor_surface: u8, ceil_surface: u8, water: bool }`, saved as `packs/maps/<zone>.grid` (postcard, versioned header `DMWGRID1`). Run time is printed; expect tens of seconds in release for Rust. Sampling runs on rayon or scoped threads per row.

### 5.2 Grid in Doom units (`dmw_mapgen::grid`)

Convert every cell: floor/ceiling × 0.8, then quantize: floor to 8, ceiling to 8 (sky ceilings get one shared height, §5.4). Reachability: flood fill from every DM spawn's cell over 4-neighbours where a move is allowed when the target is not solid, no blocker is on the shared edge, the target floor ≤ source floor + 24 and the target headroom ≥ 56. Cells never reached become solid (this trims out-of-bounds terrain and unreachable rooftops). A cell reachable only by dropping is kept.

Smoothing, in order, each pass logged with counts: (a) 3×3 majority filter on floor height restricted to cells whose 3×3 range is ≤ 16 (removes single-cell noise on dunes without eroding real steps); (b) regions (§5.3) smaller than 6 cells merge into the neighbour with the closest floor height when the difference ≤ 16; (c) repeat reachability.

### 5.3 Regions

Connected components (4-neighbour) of non-solid cells with equal `(floor, ceiling-or-sky, floor flat, ceiling flat, light, water)`; no blocker splits a region (blockers become lines inside or between regions, §5.4). Each region is one Doom sector. Budget for Rust (logged, asserted in the local test): ≤ 3000 sectors, ≤ 12000 linedefs. If over budget, coarsen floor quantization to 16 for regions under sky only, then re-run; log it.

### 5.4 Lines, sectors, things (Doom units, integer coordinates)

- Every unit edge between two cells with different region ids (or region vs solid) is a boundary segment. Merge collinear consecutive segments that have the same left and right region into one linedef; vertices are deduplicated by coordinate. All coordinates are multiples of 8, all lines axis-aligned (this keeps node splitting exact, §5.7).
- Orientation: the front side faces the region with the lower id when two-sided; for region vs solid the single side faces the region (Doom: right side = front).
- One-sided: middle texture, `ML_BLOCKING`. Two-sided: `ML_TWOSIDED`, lower/upper textures where floors/ceilings differ, `ML_DONTPEGBOTTOM` on lower textures. Edges carrying a blocker: two-sided, `ML_BLOCKING`, middle texture from the fence table (§5.5) when the IWAD has one, else none (invisible wall).
- Sectors: floor/ceiling heights, flats (§5.5), light: sky 200, roofed 160, roofed with headroom < 96: 136, water floor: 176. Specials 0, tags 0.
- Sky: every sky ceiling = `max(floor of all regions) + 256` (Doom draws no upper wall between two sky ceilings, so one height avoids sky-coloured strips); flat `F_SKY1`.
- Things (Doom thing types): player starts 1–4 at the first four DM spawns sorted by distance from the map centre, deathmatch starts (11) at up to 16 DM spawns; angle = MW2 yaw rounded to 45°; position = the spawn's cell centre if its own cell is solid after smoothing, else the spawn origin × 0.8. Weapons and ammo so every mapped MW2 gun can be picked up: shotgun (2001), chaingun (2002), rocket launcher (2003), plasma (2004), BFG (2006, Doom 2 / registered only), super shotgun (82, Doom 2 only) at distinct spawn points far from the player 1 start, each with a matching ammo box; types missing from the target IWAD are skipped. No monsters by default; `--test-monsters N` places N former humans (3004) / imps (3001) on random reachable cells ≥ 512 units from player 1, flags all skills (for the user's test run).

### 5.5 Textures, stage 1A (from the chosen IWAD)

The WAD writer gets the IWAD's available texture names (`TEXTURE1` + `TEXTURE2`) and flat names (`F_START…F_END`, also `F1_START` etc.) and picks, per MW2 surface type, the first name present from an ordered preference list. Initial tables (adjust after looking at Rust; keep each list ≥ 3 names so shareware, Doom 2 and Freedoom all resolve):

| Surface | Wall preference | Flat preference |
|---|---|---|
| concrete, plaster, asphalt, default | GRAY1, GRAY4, STARGR1, BROWN1 | FLAT1, FLOOR4_8, MFLR8_1 |
| metal, paintedmetal | METAL, SHAWN2, SUPPORT2, STARG3 | FLOOR4_8, FLAT20, FLOOR5_1 |
| rock, gravel, dirt, sand, mud | BROWN1, BROWNHUG, STONE3, STONE2 | FLOOR7_1, RROCK19, MFLR8_3, FLOOR6_2 |
| wood, bark | WOOD1, WOOD3, BROWN96 | FLAT5_2, FLOOR0_1, FLAT1 |
| brick | BRICK1, BROWN1, STARTAN3 | FLAT1 |
| grass, foliage | BROWNGRN, BROWN1 | GRASS1, FLOOR7_1 |
| water | — | FWATER1, NUKAGE1 |
| ceilings (roofed) | — | CEIL3_5, FLAT5_4, CEIL5_1 |
| fence (blocker mid texture) | MIDGRATE, MIDBARS3, MIDSPACE | — |

The final fallback is the IWAD's first texture/flat. A name in the table that a Doom 1 IWAD lacks is simply skipped. Water uses no damage special. The generator writes no `TEXTURE*`, `PNAMES` or flats in 1A (the PWAD only holds map lumps and the marker), so all IWAD-wide lists stay intact.

### 5.6 WAD output

- One PWAD per IWAD: `packs/maps/<zone>/<iwad-file-stem>.wad` (e.g. `packs/maps/mp_rust/doom1.wad`), regenerated when the grid, the generator version or the IWAD (size + mtime + first 64 KiB hash) changed. Writing a WAD from a saved grid must take < 5 s.
- Map slot: `E1M1` for Doom 1 IWADs (shareware, registered, retail, Freedoom 1), `MAP01` for Doom 2 / Freedoom 2 (from the IWAD's lump names: `MAP01` present → Doom 2).
- Lumps: `DMWMAP` marker (text: `IW4DOOM generated map <zone> v<generator version>`), then the map header and `THINGS, LINEDEFS, SIDEDEFS, VERTEXES, SEGS, SSECTORS, NODES, SECTORS, REJECT, BLOCKMAP` in that order, vanilla binary formats.
- Limits checked before writing (vanilla formats): < 32768 vertices, linedefs, sidedefs, segs, subsectors, nodes; coordinates within ±32767; blockmap ≤ 65535 words (else error with the measured size).

### 5.7 Node builder, blockmap, reject (`dmw_mapgen`)

1. **Segs** from linedefs (one per side). Partition candidates are only axis-aligned lines through existing vertices (every line is axis-aligned, §5.4), so every split point is an exact integer.
2. **Recursive split**: pick the candidate minimising `splits * 8 + |left − right|`, sampling at most 64 candidates per node (spread evenly through the seg list) for speed. A seg set is a subsector when it is convex (all segs on one side of every seg's line) and all segs reference one sector; otherwise keep splitting. Collinear segs on the partition go to the side their direction matches (Doom convention). Node bounding boxes are the boxes of the child seg sets. Seg `angle` and `offset` are computed the vanilla way; split segs inherit `offset` plus the distance from the linedef start.
3. **Blockmap**: 128-unit blocks from the map's min corner, each list `0, linedefs…, -1`; **reject**: all zeros (`ceil(sectors² / 8)` bytes).
4. **Validator** (`validate.rs`, run after every build and in tests): every subsector convex and single-sector, every seg inside its node's box, every linedef covered by its segs exactly once, node child indices in range, no zero-length seg, vertex and line counts under limits, every sector closed (each sector's boundary forms closed loops). The exporter refuses to write a WAD that fails validation and prints the failing check.

### 5.8 Shareware exception (engine)

In the `-file` shareware check of `D_DoomMain`: allow the file set when every PWAD loaded contains a lump named `DMWMAP` and contains only that lump plus map lumps (`E1M1`/`MAP01` header and the ten map lumps). Anything else keeps the original error. Log `info` once: `generated map <file> allowed with the shareware IWAD`. `modifiedgame` stays true (so the shareware "modified game" notice behaves as vanilla). Demos are unaffected (`demo_playthrough` uses no PWAD).

### 5.9 Raised renderer limits (engine)

`MAXVISPLANES 1024`, `MAXDRAWSEGS 2048` (both definitions), `MAXOPENINGS SCREENWIDTH * 256`, `MAXVISSPRITES 1024`, `MAXSEGS 128`. They are static array sizes only; vanilla rendering of vanilla maps is unchanged (`demo_playthrough` and the goldens must pass untouched). `test_api::render_counts()` returns the visplanes, drawsegs, openings and vissprites used by the last `R_RenderPlayerView`.

### 5.10 Launchers

`./iw4doom.sh --map rust` (aliases: `rust`, `mp_rust`; unknown names list the supported maps): after step 5 (weapon export), step 5b "Converting MW2 map Rust" runs `dmw_export map --zone mp_rust` when `packs/maps/mp_rust.grid` is missing or older than the exporter binary, then writes the WAD for the chosen IWAD (§5.6). Step 8 starts the game with `-file packs/maps/mp_rust/<stem>.wad` and `-warp 1 1` (Doom 1) or `-warp 1` (Doom 2) prepended to the user's extra arguments. `--reexport` also redoes the grid. PowerShell: `-Map rust`. Without `--map` nothing changes. The launcher tests gain one case: `--map nope` exits 1 listing `rust`.

### 5.11 Stage 1B: MW2 textures

After 1A passes and the user has tested it:

- Sampling also records, per hit, the material index (`ClipCollision.materials` / `tri_material_index`, or brush side material); `MapGrid` stores the material name table.
- The exporter resolves each material's colour map image through `PreparedMatch.materials` (the same path sheet 04 uses for viewmodel textures), downsamples to 64×64 (flats) and 64×128 (walls), quantizes to the target IWAD's `PLAYPAL` (the existing quantizer in `dmw_viewmodel`), and writes them into the PWAD as a complete merged set: `PNAMES` = IWAD names + new patch names; `TEXTURE1` = IWAD `TEXTURE1` + new textures (and the IWAD's `TEXTURE2` copied as `TEXTURE2` when present); flats `FF_START…FF_END` with every IWAD flat copied plus the new ones (so animated flat and switch lookups still resolve); patches as plain lumps named `DMWP0000`… The §5.8 exception then also allows these lump names inside a `DMWMAP` PWAD.
- At most 64 MW2 textures (the most used by cell count); the rest fall back to 1A selection.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| Zone has no clip collision | exit 3 `mp_rust: no clip collision in the loaded zone` |
| No DM spawns | exit 3; nothing to anchor reachability |
| Grid file from an older generator | regenerate, log why |
| Budget exceeded after coarsening | write anyway, `warn` with the counts; the local test fails |
| Validator failure | exit 4, no WAD written, failing check printed |
| IWAD unknown (not Doom 1/2 family) | exit 2 |
| Player start cell solid after smoothing | nearest reachable cell within 64 units, else the next spawn |

## 7. Logging

Exporter: one line per phase with counts and ms (`sample`, `reach`, `smooth`, `regions`, `lines`, `nodes`, `blockmap`, `validate`, `write`), the final budget line (`sectors N, linedefs N, segs N, subsectors N, nodes N, things N`) and the surface-type histogram. Engine: the §5.8 line.

## 8. Licensing and asset notes

The generated WAD contains MW2-derived geometry (and in 1B, textures) from the user's own install; it lives under gitignored `packs/maps/` and is never committed, uploaded or attached to CI artifacts. Synthetic fixtures in `dmw_mapgen` are hand-made grids. No MW2 data in tests that run in CI.

## 9. Tests

| Id | Test |
|---|---|
| T17.1 | `dmw_mapgen` unit tests on synthetic grids (`fixtures.rs`): a single room; a room with a pillar; two rooms joined by a 3-step staircase; an L-shaped corridor; a courtyard under sky beside a roofed room; a fenced yard (blocker lines); a noisy dune patch (smoothing counts). Each: validator passes, expected sector/line counts, reachability result, convex subsectors |
| T17.2 | Node builder property test: random axis-aligned multi-room grids (seeded, 200 cases), validator passes |
| T17.3 | WAD round trip: written lumps parse back to the same structures; lump order and sizes; marker present; map slot per IWAD kind |
| T17.4 | `room/tests/mapgen_headless.rs` (CI, Freedoom 1 and 2): load a generated synthetic PWAD with `-file`, warp, teleport to 20 spots × 8 angles, render each; no panic, `render_counts()` under the raised limits; the player can walk forward 10 tics up the fixture staircase |
| T17.5 | Shareware exception: with `doom1.wad` (local, `DMW_DOOM1_WAD`), a `DMWMAP` PWAD is accepted and a PWAD without the marker (or with an extra lump) still fails with the original message |
| T17.6 | `demo_playthrough` and every golden pass unchanged after §5.9 |
| T17.7 | Local only (`mw2map_headless.rs`, skipped without the grid): convert Rust, load it on `doom1.wad` and `freedoom2.wad`, render from every DM spawn × 8 angles: max visplanes, drawsegs, vissprites logged and under limits; budget §5.3 met; every DM spawn reachable from player 1 |
| T17.8 | Launcher: `--map nope` exits 1 listing `rust` (bash and PowerShell) |
| T17.9 | Manual (the user): `./iw4doom.sh --iwad doom1.wad --map rust`, walk the whole map, note spots that look wrong (screenshots + coordinates from the `IDMYPOS` cheat) |

## 10. Acceptance criteria

- [ ] T17.1–T17.6, T17.8 pass in CI on Linux and Windows.
- [ ] T17.7 passes locally with the user's MW2 install; the agent reports the budget line, the max render counts and the conversion time.
- [ ] `./iw4doom.sh --iwad doom1.wad --map rust` starts the game on Rust with no warnings from the generator.
- [ ] Stage 1B either done (T17.7 also passes with MW2 textures) or reported as not attempted / blocked with the reason.
- [ ] README has a short "Play Rust" section; no MW2 data in the repo.

## 11. Contract changes requested

Applied as 00 Revision 12 before this sheet starts:

1. §4.1: new crate `crates/dmw_mapgen`; `packs/maps/` (gitignored) for grids and generated PWADs.
2. §4.3: no new engine flags (the launcher uses Doom's own `-file`/`-warp`).
3. Engine edits outside 00 §7.5: the shareware `-file` exception (§5.8) and the raised renderer limits (§5.9).
4. Launcher flag `--map NAME` / `-Map NAME`.
