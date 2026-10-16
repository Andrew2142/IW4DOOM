# Golden images

PNG files compared by `room/tests/support/golden.rs` (`assert_golden`). Exact match by default;
`BLESS=1 cargo test -p room --test <file>` rewrites them, and a blessed change is reviewed like
code. A failing comparison leaves `<name>.actual.png` and `<name>.diff.png` in
`target/test-artifacts/`.

## Provenance and licences

- Frames of the 3D world (walls, floors, sprites, the pistol) are rendered from the pinned
  Freedoom 1 IWAD (`wads/freedoom1.wad`, Freedoom 0.13.0, fetched by `tools/fetch-freedoom.sh`).
  Freedoom is licensed under the BSD 3-Clause licence:

  Copyright (c) Contributors to the Freedoom project. All rights reserved.
  Redistribution and use in source and binary forms, with or without modification, are
  permitted provided that the licence conditions at
  <https://github.com/freedoom/freedoom/blob/master/COPYING.adoc> are met.

- Pixels of the synthetic fixture gun (sheet 03) are our own (GPL-3.0-or-later).
- Never commit a frame rendered with an MW2 pack (sheet 13 section 8).

## Files

| File | Test | Content |
|---|---|---|
| `harness_open_room.png` | `room/tests/harness_smoke.rs` | 320x160 3D view at `OPEN_ROOM`, E1M1, monsters removed |
| `freelook_up.png` | `room/tests/freelook_headless.rs` (T6.3) | 320x168 view at `OPEN_ROOM`, MW2 mode, pitch -20 (looking up): horizon 58 rows lower, pistol unmoved |
| `freelook_level.png` | `room/tests/freelook_headless.rs` (T6.3) | same spot, pitch 0 (identical to vanilla) |
| `freelook_down.png` | `room/tests/freelook_headless.rs` (T6.3) | same spot, pitch +20 (looking down): horizon 58 rows higher |
| `freelook_zoom45.png` | `room/tests/freelook_headless.rs` (T6.5) | same spot, pitch 0, 45-degree horizontal FOV (door at the end of the hall magnified 2.4x, light unchanged) |
| `freelook_sky_up.png` | `room/tests/freelook_headless.rs` (T6.6) | `SKY_VIEW`, pitch -37 (input limit plus 5 degrees of kick): sky above the building, no wrap seam |
| `viewmodel_e1m1_t60.png` | `room/tests/viewmodel_headless.rs` (T11.9) | 320x200 E1M1 start after 60 tics with `-fixturepack`: the fixture pistol drawn by the software rasterizer in the lower right (grey box gun pointing into the screen, olive box in front of the muzzle, left hand at the bottom), status bar untouched |
| `e1m1_t60_statusbar.png` | `viewmodel_headless.rs` and `viewmodel_nomw2_headless.rs` (T11.9) | 320x32 status bar rows of that frame, max-ammo column (x 300..316) blanked; both the `-fixturepack` and the `-nomw2` run must match it |
| `viewmodel_fixture_idle.png` | `viewmodel_headless.rs` (T11.8) | 320x168 fixture pistol alone (engine draw path, COLORMAP level 8, directional shading) on a flat background, idle pose |
| `viewmodel_fixture_fire.png` | `viewmodel_headless.rs` (T11.8) | same, on the firing tic: Freedoom `PISFA0` flash patch fullbright at `tag_flash` |
| `viewmodel_fixture_reload.png` | `viewmodel_headless.rs` (T11.8) | same, 14 tics into the reload: gun pitched about 10 degrees muzzle-down, magazine dropped out of view |
| `viewmodel_fixture_ads.png` | `viewmodel_headless.rs` (T11.8) | same, full ADS: gun centred below the aim point, top face receding to the muzzle box at the centre |
