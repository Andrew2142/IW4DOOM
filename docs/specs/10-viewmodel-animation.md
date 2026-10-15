# 10 — Viewmodel Animation and Placement (`dmw_weapons`, anim half)

Owner: build agent (wave 4)
Status: implementation-ready
Depends on: 00 (§2 D4, D9; §7.2 `ViewState`, `GunPlacement`, `ViewmodelPose`, `ViewmodelAnimator`), 01, 03 (`ViewmodelFile`, `ClipSet`, `Skeleton`), 07 (shadow `PlayerState`, kick/sway state, call site of `placement::step`)

This sheet turns the sim's state into a posed MW2 viewmodel: which MW2 clips play with which weights (a port of iw4L's `ViewmodelController` driven by `weap_anim` edges exactly as iw4L's FPV presenter does), where the gun sits in front of the eye (MW2's `weapon_placement_assemble`: movement offsets, bob, idle sway, look sway, gun recoil), and the final eye-space skinning matrices the rasterizer draws.

---

## 1. Purpose

- Real MW2 animation: raise, first raise, fire, ADS fire, last shot, rechamber, reload / empty reload / segmented reload, sprint in/loop/out, drop, melee, ADS up/down, idle and empty idle — chosen by the same `weap_anim` signal MW2's own client uses.
- Real MW2 gun placement and motion.
- Notetrack sounds (magazine out/in, bolt, etc.) at the authored clip times.
- Output an eye-space pose sheet 11 can draw without knowing anything about MW2.

## 2. Scope / Out of scope

In scope (files 00 §6.2 owned by 10): `placement.rs`, `clip_scheduler.rs`, `controller.rs`, `pose.rs`, `notetracks.rs`, plus `convert.rs` (pack mirrors → `xmodel_runtime` types).

Out of scope: rasterization and visibility rules for rocket/knife/lens surfaces (11), sound playback (12), left-hand/akimbo rigs, landing dips, damage view kick on the gun (no MW2 damage events), lean, stance offsets (always standing).

## 3. Dependencies

| From | What |
|---|---|
| 03 | `ViewmodelFile { skeleton, mesh, clips }`, `ClipData`, `TrackData`, `Skeleton`, `BoneData`, `FpvFacts`, `AnimTimes`, `WeaponSounds::notetracks` |
| 07 | shadow `PlayerState` (`weap_anim`, `weaponstate_primary`, `f_weapon_pos_frac`, `bob_cycle`, `velocity`, `viewangles`, `pm_flags`, `weapon_time`, `hold_breath_scale`), `ViewSwayState::springs()`, `ViewKickState::gun` and `kick_angles`, `msec` |
| iw4L `xmodel_runtime` | `DObj`, `Bone`, `ModelSlot`, `AnimClip`, `Track`, `Rotation`, `Translation`, `Keyed`, `FrameIndices`, `ClipNotify`, `AnimInstance`, `DObj::pose`, `DObj::skin_matrices` |
| iw4L `anim_iw4` | `PartBits` (`WORDS = 6`, capacity 192) |
| iw4L `weapon_iw4` | `weapon_placement_assemble`, `WeaponPlacementState`, `WeaponPlacementPsInputs`, `WeaponStanceStaticOfsInputs`, `StanceTransitionFadeGlobals`, `WeaponMovementKinematics`, `WeaponBobInputs`, `calculate_weapon_movement_bob_waveform`, `WeaponBobWaveformInputs`, `GunRecoilResponse`, `PLACEMENT_ASSEMBLE_STEP_COUNT`, `WeaponPlacementAssembleStep`, `viewweapon_save_gun_pitch_yaw`, `viewweapon_iron_ads_saves_composed_axis`, `ads_overlay_scrub`, `WEAP_ANIM_EVENT_MASK`, `slot_for_weap_anim_event`, `weap_anim_event`, `WEAPON_ANIM_SLOTS` |
| iw4L `math_iw4` | `angle_subtract`, `angles_to_axis`, `angle_vectors` |
| iw4L sources to port (read) | `asset_anim/src/clip_scheduler.rs` (358 lines, std-only), `render_anim/src/anim/viewmodel_controller.rs` (644 lines), `render_anim/src/anim/fpv.rs` (`tick_equipped_fpv_with_extra_events`, `observe_weap_anim_edge_on`), `render_anim/src/anim/fpv_rig.rs::pose_clips` (posing and collapsed bones), `render_anim/src/anim/fpv_pose.rs` (`tag_view` fold, `placement_angles_to_bevy_camera_quat`), `render_anim/src/occupancy/fpv_present.rs::apply_fpv_placement` (lines 1032–1250), `asset_game/src/weapon_animations.rs` (`WeaponAnimSlot`, `ACTION_GOAL_TIME_SECS`, `ACTIVE_GOAL_WEIGHT`, `INACTIVE_GOAL_WEIGHT`, `IDLE_INTERRUPT_GOAL_TIME_SECS`, `playback_rate`, `slot_uses_native_rate`) |

## 4. File inventory

| Path | Content |
|---|---|
| `crates/dmw_weapons/src/convert.rs` | `Skeleton → DObj`, `ClipData → AnimClip` (§5.2) |
| `src/clip_scheduler.rs` | port of `ClipScheduler` (§5.4) |
| `src/controller.rs` | port of `ViewmodelController` minus left hand / perks (§5.5) |
| `src/placement.rs` | `PlacementState`, `ViewInputs`, `step`, `gun_angle_offset` (§5.3) |
| `src/pose.rs` | `ViewmodelAnimator::pose` (§5.6) |
| `src/notetracks.rs` | notification → `SoundCue` (§5.7) |
| `src/anim.rs` | `ViewmodelAnimator` (00 §7.2) gluing the above |
| `tests/anim_fixture.rs`, `tests/golden/anim_*.json` | §9 |

## 5. Detailed design

### 5.1 Coordinate conventions

- Model/bind space: MW2 inches, x forward, y left, z up (03 §5.5.1).
- **Eye space** (output of this sheet, input of 11): MW2 view axes — x forward (into the screen), y left, z up, origin at the eye, inches. iw4L folds into Bevy axes with `tag_view_to_bevy_camera()`; this port **omits the fold** everywhere (`eye_from_world = world[view_bone]⁻¹` instead of `fold · world[view_bone]⁻¹`), and converts placement angles with `math_iw4::angles_to_axis` directly instead of `placement_angles_to_bevy_camera_quat`.

### 5.2 Conversion (`convert.rs`)

- `skeleton_to_dobj(&Skeleton) -> Result<DObj, AnimError>`: `Bone { name, model, parent, bind_rotation: Quat::from_xyzw(..), bind_translation: Vec3::from(..), bind_world: Mat4::from_cols_array(..), no_scale }`, `ModelSlot { name, base, bone_count, scale }`, `duplicates`. Reject > `PartBits::CAPACITY` (192) bones (and request 03's validation limit be 192 — see §11).
- `clip_to_anim(&ClipData) -> AnimClip`: field-for-field into `xmodel_runtime::{AnimClip, Track, Rotation, Translation, Keyed, FrameIndices, ClipNotify}`. No re-quantisation; values are copied.
- `ClipSet` → `[Option<Arc<AnimClip>>; WEAPON_ANIM_SLOTS]` + per-clip `Vec<Option<usize>>` track maps.

### 5.3 Placement (`placement.rs`)

```rust
pub struct PlacementState { inner: weapon_iw4::WeaponPlacementState }
pub struct ViewInputs<'a> {
    pub ps: &'a PlayerState, pub facts: &'a WeaponFacts,
    pub sway: SwaySpringState, pub gun_recoil: GunRecoilPlacementState,
    pub view_kick_deg: [f32; 3], pub msec: i32,
}
pub fn step(state: &mut PlacementState, v: &ViewInputs) -> GunPlacement;
pub fn gun_angle_offset(p: &GunPlacement, v: &ViewInputs) -> [f32; 2];
```

`step` ports `apply_fpv_placement` (fpv_present.rs:1032–1240) minus Bevy, killcam, dual wield, landing and `cg_gun` offsets:

1. `state.inner.sway_springs = v.sway; state.inner.gun_recoil = v.gun_recoil` (iw4L injects these each frame from `SessionViewKick`).
2. `ps_in = WeaponPlacementPsInputs { e_flags: ps.e_flags, weapon_pos_frac: ps.f_weapon_pos_frac, weapon_time: ps.weapon_time, aim_down_sight: fpv.aim_down_sight, overlay_reticle: fpv.overlay_reticle, weapon_transition_active: false, lean_fraction: 0.0 }`.
3. `stance = WeaponStanceStaticOfsInputs { ducked_ofs, prone_ofs, ads_aim_pitch, night_vision_wear_time: 0 }`.
4. `kinematics = WeaponMovementKinematics { xyspeed: |velocity.xy|, speed: ps.speed as f32, velocity, viewangles, weaponstate: ps.weaponstate_primary, weaponstate_secondary: 0, pm_flags, frametime: msec / 1000 }`.
5. `waveform = calculate_weapon_movement_bob_waveform(WeaponBobWaveformInputs { bob_cycle: ps.bob_cycle as u8, xyspeed, view_height_target: ps.view_height_target, pm_flags, weapon_pos_frac })`.
6. `idle = fpv.idle`, with `idle.ads_idle_amount *= ps.hold_breath_scale` when `fpv.can_hold_breath`.
7. `contrib = weapon_placement_assemble(&mut state.inner, ps_in, stance, StanceTransitionFadeGlobals::default(), fpv.movement, kinematics, WeaponBobInputs { ads_bob_factor }, idle, Some(waveform), GunRecoilResponse::default(), GunRecoilResponse::default(), kick.gun_max_pitch, kick.gun_max_yaw, 0.0, &mut steps)` — `dt = 0.0` exactly as iw4L, because gun recoil is advanced by `ViewKickState::advance` (07 step 12).
8. Return `GunPlacement { origin: contrib.origin, angles: contrib.angles }`.

`gun_angle_offset`: `refdef = [ps.viewangles[0] + kick[0], ps.viewangles[1] + kick[1], 0]`; `[gp, gy] = viewweapon_save_gun_pitch_yaw(p.angles, refdef, fpv.aim_down_sight, ps.f_weapon_pos_frac, fpv.overlay_reticle)`; return `[angle_subtract(gp, ps.viewangles[0]), angle_subtract(gy, ps.viewangles[1])]` (fpv_present.rs:1223).

Eye-from-placement transform: `placement_matrix(p) = Mat4::from_translation(p.origin) · rotation(angles_to_axis(p.angles))` in MW2 view axes. The agent confirms the composition order and sign conventions against `iw_view_placement_to_bevy_camera_local` (render_scene) and `placement_angles_to_bevy_camera_quat` (fpv_pose.rs), then removes the fold; a unit test (T10.4) pins it.

### 5.4 Clip scheduler port

`clip_scheduler.rs` is a straight port of `asset_anim/src/clip_scheduler.rs` (`ClipScheduler`, `ActiveAnim`, `ClipNotification`, `ClipAdvance`, `ClipSchedulerError`): per node a clip, time, rate, weight, goal weight and goal time; `advance(dt)` steps times, ramps weights, wraps looping clips and emits notifications for notifies crossed. Keep its atomic id counter only if it affects ordering; otherwise replace with a plain counter (single-threaded). Apache header.

### 5.5 Controller port

`controller.rs` ports `ViewmodelController` with these reductions: no left hand, perks fixed at 0 (`perk_scaled_action_timer` returns its input), `WeaponAnimations` replaced by `(clips, AnimTimes, AdsOverlayConvention)` from the pack. Keep: `new` (idle at active weight; AdsDown parked at its end with rate 0, or AdsUp per convention), `handle`, `dispatch_sz_xanim_index`, `state_and_timer_for_slot`, `advance`, `apply_idle_weap_anim`, `apply_force_idle_weap_anim`, `settle_fire_to_idle`, `event_allowed`, `apply_ads_overlay_frame`, `start_action`, `start_idle`, `start_idle_family`, `dispatch_range_unfinished`, `zero_dispatch_slots`, `finish_fire_cycle`, `action_finished`, `set_weight`, constants from `asset_game` (`ACTION_GOAL_TIME_SECS` etc. — copy their values with a comment citing the file).

Per-tic driver (`ViewmodelAnimator::advance(view, msec)`), porting `tick_equipped_fpv_with_extra_events` for one hand and no authority/prediction split:

```text
edge = observe_weap_anim_edge(last_raw, view.weap_anim)       // port of observe_weap_anim_edge_on
match edge:
  Unchanged  → ()
  Idle       → if !controller.apply_idle_weap_anim(view.clip == 0) { last_raw = None }
  ForceIdle  → controller.apply_force_idle_weap_anim(view.clip == 0)
  Dispatch(slot) → controller.dispatch_sz_xanim_index(slot)
advance = controller.advance(msec / 1000)
controller.apply_ads_overlay_frame(view.ads_frac)
return notetracks::to_cues(advance.notifications, facts.sounds.notetracks)
```

A missing clip for a dispatched slot logs `debug!` (`EventResult::IgnoredMissingClip`) and the previous animation continues — never the bind pose (iw4L rule).

When the held weapon changes, the engine creates a new `ViewmodelAnimator` for the new weapon at its first `RaiseStarted` (animators are cached per weapon id in `viewmodel::Registry`, 08 §5.1, and reset rather than rebuilt).

### 5.6 Pose (`ViewmodelAnimator::pose(view) -> ViewmodelPose`)

Port of `PreparedFpvRig::pose_clips` for hand 0 with no left-hand blending:

1. `instances: Vec<AnimInstance>` = every active scheduler node (weight > 0) with its clip, its track map (`tracks: &[Option<usize>]`), time and weight, `parts: None`.
2. `world = dobj.pose(&instances, &PartBits::all(), Mat4::IDENTITY)`.
3. `skin = dobj.skin_matrices(&world)`; for each collapsed bone: rotation columns zeroed, translation kept (fpv_rig.rs:663).
4. `eye_from_model = placement_matrix(view.placement) · world[view_bone].inverse()`.
5. `pose.skin[b] = eye_from_model · skin[b]`; `pose.tag_flash = flash_bone.map(|f| eye_from_model · world[f])`.

All matrices are `glam::Mat4` (same glam as `xmodel_runtime`).

### 5.7 Notetracks (`notetracks.rs`)

`ClipNotification { name, … }` → `SoundCue { alias: facts.sounds.notetracks[name], local: true }` when mapped; unmapped notetracks are ignored (`trace!`). Notetrack names are compared case-insensitively. Notifications crossed during one `advance` are emitted in clip-time order.

### 5.8 Surface visibility hints (for 11)

`ViewmodelAnimator::visibility(view) -> SurfaceMask { rocket: bool, knife: bool }`: rocket surfaces visible when `view.clip > 0`, or while reloading after `fpv.reload_show_rocket_time_ms` into the reload (iw4L `viewmodel_rocket_should_be_attached`, weapon_iw4/weaponstate.rs:111 — call it); knife surfaces visible only in weapon states `MeleeInit..=MeleeEnd`.

## 6. Error handling and edge cases

| Case | Behaviour |
|---|---|
| Clip track maps to no bone | already dropped by the exporter; defensive `debug!` |
| > 192 bones | `AnimError::TooManyBones` at load → 08 demotes the slot to vanilla |
| NaN in a pose | `debug_assert!`; release build replaces NaN matrices with the previous frame's |
| Weapon without ADS clips | ADS handled by placement only (most MW2 guns animate ADS through the AdsUp/AdsDown clips; some rely on placement) |
| Dispatch while dropping | controller rules (`event_allowed`) decide |

## 7. Logging

`dmw::anim`: `debug!` dispatches and their `EventResult`; `trace!` active nodes (slot, weight, time) per tic; `warn!` once per weapon per missing commonly-used slot (Idle, Fire, Reload, Raise, Drop).

## 8. Licensing and asset notes

`clip_scheduler.rs`, `controller.rs`, parts of `placement.rs` and `pose.rs` are ports of iw4L Apache-2.0 code → headers + NOTICE rows (01 §5.10).

## 9. Tests (fixture pack)

| Id | Test |
|---|---|
| T10.1 | `convert.rs`: fixture `ClipData` → `AnimClip`; sampling the converted clip at 20 times equals sampling a hand-built `AnimClip` with the same keys |
| T10.2 | controller: scripted `weap_anim` edges (raise → idle → fire ×3 → reload → idle) produce the expected active slots and states; golden `anim_sequence.json` |
| T10.3 | notetracks: fixture reload emits `fixture_mag_out` at 0.3 s and `fixture_mag_in` at 0.7 s (± one tic) |
| T10.4 | placement composition: with zero offsets the fixture `tag_flash` lands at eye-space (+x forward, slightly −y right, −z below) — pins the axis/sign conventions of §5.1 |
| T10.5 | pose: bind pose (no clips weight) skin matrices × bind positions = eye-space positions of the bind mesh (skinning identity check) |
| T10.6 | ADS: `ads_frac` 0 → 1 moves the fixture gun from hip offset toward the centre (ads_up clip), `tag_flash` y → ~0 |
| T10.7 | sprint: sprint_in → sprint_loop → sprint_out dispatches follow weaponstate 0x17–0x19 |
| T10.8 | determinism and performance: 10 000 `advance`+`pose` calls are deterministic and average ≤ 0.1 ms (release) |

## 10. Acceptance criteria

- [ ] 00 §7.2 anim API implemented; ports carry headers.
- [ ] The fixture gun visibly raises, idles, recoils, reloads (magazine drops), sprints and aims when driven by the sim (verified in 11's golden frames and by eye in `task run-fixture`).
- [ ] T10.1–T10.8 pass.

## 11. Contract changes requested

1. 03 §5.8 skeleton validation limit: `bones.len() <= 192` (`PartBits::CAPACITY`), not 512. — Resolved in 00 Revision 5.
2. 00 §7.2: add `ViewmodelAnimator::visibility(&self, view: &ViewState) -> SurfaceMask` (§5.8). — Resolved in 00 Revision 5.
