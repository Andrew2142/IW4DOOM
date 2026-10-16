//! WASD movement, F use and the door/switch prompt (sheet 05 section 11, sheet 12 section 11).
//!
//! The real engine with the fixture pack on Freedoom 1 E1M1 / E2M8. One `#[test]`, one engine.
//! Spots are derived from the map itself (`stand_before`: a point 32 units in front of a
//! line's front side, facing it), so they only depend on the pinned WAD's line coordinates
//! recorded in the constants below.

mod support;

use room::dmw::hud::{self, crosshair_layout, ViewRect};
use room::dmw::test_api;
use room::doom::d_main::D_Display;
use room::doom::doomkeys::KEY_FIRE;
use support::locations::{MapLine, MapSnapshot, GUN_SWITCH, OPEN_ROOM};
use support::{assert_golden, Harness, Rect};

const VIEW_CROP: Rect = Rect::new(0, 0, 320, 168);

/// E1M1 line 537, special 1 (DR door open, wait, close), front side west.
const DOOR_LINE: ((f32, f32), (f32, f32)) = ((544.0, 744.0), (544.0, 680.0));
/// E1M1 line 593, special 88 (WR lift): walk-triggered, never a use prompt.
const WALK_LINE: ((f32, f32), (f32, f32)) = ((192.0, 320.0), (64.0, 320.0));

fn tap(h: &mut Harness, key: u8) {
    h.key(key, true);
    h.run_tics(2);
    h.key(key, false);
}

/// A point `dist` units in front of the line `(a, b)`'s front side (right of a to b) and the
/// angle (degrees) that faces the line.
fn stand_before(a: (f32, f32), b: (f32, f32), dist: f32) -> (f32, f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    let n = (dy / len, -dx / len);
    let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let face = (-n.1).atan2(-n.0).to_degrees();
    (
        mid.0 + n.0 * dist,
        mid.1 + n.1 * dist,
        face.rem_euclid(360.0),
    )
}

fn find_line(snap: &MapSnapshot, a: (f32, f32), b: (f32, f32)) -> MapLine {
    *snap
        .lines
        .iter()
        .find(|l| l.a == a && l.b == b)
        .unwrap_or_else(|| panic!("line {a:?}-{b:?} not in the map (different WAD?)"))
}

/// Teleports to `(x, y, angle)`, lets the player settle and draws one frame.
fn go(h: &mut Harness, (x, y, angle): (f32, f32, f32)) {
    h.teleport(x, y, angle);
    h.run_tics(2);
    D_Display();
}

fn prompt() -> Option<String> {
    hud::use_prompt_text()
}

fn view_rect() -> ViewRect {
    ViewRect {
        x: 0,
        y: 0,
        w: 320,
        h: 168,
    }
}

fn delta(h: &Harness, from: (f32, f32)) -> (f32, f32) {
    let p = h.pos();
    (p.0 - from.0, p.1 - from.1)
}

/// Holds `key` for 8 tics, lets the momentum die, returns the displacement.
fn walk(h: &mut Harness, key: u8) -> (f32, f32) {
    h.teleport(OPEN_ROOM.x, OPEN_ROOM.y, OPEN_ROOM.angle_deg);
    h.run_tics(2);
    let from = h.pos();
    h.hold(key, 8);
    h.run_tics(40);
    delta(h, from)
}

/// The door sector's ceiling is above its floor.
fn door_open(back_sector: usize) -> bool {
    let snap = MapSnapshot::capture();
    let s = snap.sectors[back_sector];
    s.ceil > s.floor
}

#[test]
fn useprompt_headless() {
    let wad = support::find_wad();
    let wad = wad.to_str().expect("utf-8 path");
    let mut h = Harness::start(&[
        "-iwad",
        wad,
        "-fixturepack",
        "-skill",
        "3",
        "-warp",
        "1",
        "1",
    ]);
    assert!(room::dmw::mode::weapons_active());
    OPEN_ROOM.go(&mut h);

    // ---- WASD moves relative to the facing (OPEN_ROOM faces north, +y) ----
    let (fx, fy) = walk(&mut h, b'w');
    assert!(fy > 20.0 && fx.abs() < 2.0, "w moves forward: ({fx}, {fy})");
    let (fx, fy) = walk(&mut h, b's');
    assert!(fy < -10.0 && fx.abs() < 2.0, "s moves back: ({fx}, {fy})");
    let (fx, fy) = walk(&mut h, b'a');
    assert!(fx < -20.0 && fy.abs() < 2.0, "a strafes left: ({fx}, {fy})");
    let (fx, fy) = walk(&mut h, b'd');
    assert!(fx > 20.0 && fy.abs() < 2.0, "d strafes right: ({fx}, {fy})");
    // Arrow up no longer moves; arrow left still turns.
    let (fx, fy) = walk(&mut h, room::doom::doomkeys::KEY_UPARROW);
    assert!(
        fx.abs() < 1.0 && fy.abs() < 1.0,
        "uparrow is unbound: ({fx}, {fy})"
    );
    let before = h.mobj().angle;
    h.hold(room::doom::doomkeys::KEY_LEFTARROW, 6);
    h.run_tics(2);
    assert_ne!(h.mobj().angle, before, "leftarrow still turns");

    // ---- prompt geometry ----
    let snap = MapSnapshot::capture();
    let door = find_line(&snap, DOOR_LINE.0, DOOR_LINE.1);
    assert_eq!(door.special, 1);
    let door_back = door.back.expect("two-sided door");
    let door_spot = stand_before(DOOR_LINE.0, DOOR_LINE.1, 32.0);
    assert_eq!(
        snap.sector_at(door_spot.0, door_spot.1),
        door.front.unwrap()
    );

    // Walk-triggered line: no prompt.
    let walk_line = find_line(&snap, WALK_LINE.0, WALK_LINE.1);
    assert_eq!(walk_line.special, 88);
    go(&mut h, stand_before(WALK_LINE.0, WALK_LINE.1, 32.0));
    assert_eq!(prompt(), None, "no prompt at a walk-trigger line");

    // Plain wall: any long one-sided line without a special, with room in front of it.
    let wall = snap
        .lines
        .iter()
        .find(|l| {
            l.special == 0 && l.back.is_none() && {
                let len = (l.b.0 - l.a.0).hypot(l.b.1 - l.a.1);
                let (x, y, _) = stand_before(l.a, l.b, 32.0);
                len >= 128.0
                    && snap.sector_at(x, y) == l.front.unwrap()
                    && snap.nearest_line_distance(x, y) >= 31.0
            }
        })
        .copied()
        .expect("a plain wall");
    go(&mut h, stand_before(wall.a, wall.b, 32.0));
    assert_eq!(prompt(), None, "no prompt at a plain wall: {wall:?}");

    // The far side of this door is a separate linedef (539) with its own special, so it
    // prompts too; the back of a single line is never used by P_UseSpecialLine.
    go(&mut h, stand_before(DOOR_LINE.1, DOOR_LINE.0, 32.0));
    assert_eq!(prompt().as_deref(), Some("Press F to open"));

    // Out of range (USERANGE is 64).
    go(&mut h, stand_before(DOOR_LINE.0, DOOR_LINE.1, 96.0));
    assert_eq!(prompt(), None, "no prompt out of range");

    // ---- low ammo first (so the door prompt can be stacked below it) ----
    OPEN_ROOM.go(&mut h);
    h.give_all();
    h.run_tics(2);
    tap(&mut h, b'4');
    for _ in 0..200 {
        let v = h.sim_view().expect("pack");
        if v.clip <= 5 && v.stock > 0 {
            break;
        }
        tap(&mut h, KEY_FIRE);
        h.run_tics(6);
    }
    h.run_tics(150);
    assert!(h.sim_view().unwrap().clip <= 5);

    // ---- door prompt, golden, then F opens the door ----
    go(&mut h, door_spot);
    // Wait for the blink's on phase so the golden shows both lines.
    for _ in 0..30 {
        D_Display();
        if hud::reload_prompt_text().is_some() {
            break;
        }
        h.run_tics(1);
    }
    assert_eq!(prompt().as_deref(), Some("Press F to open"));
    assert_eq!(
        hud::reload_prompt_text().as_deref(),
        Some("Press R to reload")
    );
    // Both lines stack: the reload line is at prompt_top, the door line ten rows below it.
    let v = h.sim_view().unwrap();
    let pack = dmw_format::Pack::fixture();
    let l = crosshair_layout(
        view_rect(),
        v.zoom_hfov_deg.unwrap_or(90.0),
        v.spread_deg,
        v.gun_angle_offset,
        &pack.facts(v.weapon.unwrap()).hud,
    );
    assert_eq!(
        hud::use_prompt_top(hud::prompt_top(&l), true),
        hud::prompt_top(&l) + 10
    );
    // Draw until the reload prompt is lit, then check both rows differ from the HUD-less view.
    let mut stacked = None;
    for _ in 0..40 {
        h.run_tics(1);
        D_Display();
        let with = h.frame();
        hud::test_set_overlays_enabled(false);
        D_Display();
        let without = h.frame();
        hud::test_set_overlays_enabled(true);
        let top = hud::prompt_top(&l) as usize;
        let band = |y0: usize| {
            (y0..y0 + 8).any(|y| {
                (l.cx as usize - 30..l.cx as usize + 30).any(|x| with.at(x, y) != without.at(x, y))
            })
        };
        if band(top) && band(top + 10) {
            stacked = Some(with);
            break;
        }
    }
    let stacked = stacked.expect("reload and door prompts lit together on separate rows");
    assert_golden("use_prompt_stacked", &stacked, Some(VIEW_CROP));

    // Repeat with the reload key rebound: the letter follows the binding.
    // SAFETY: plain int on the engine thread.
    unsafe { room::doom::m_controls::key_dmw_reload = i32::from(b't') };
    D_Display();
    assert_eq!(
        hud::reload_prompt_text().as_deref(),
        Some("Press T to reload")
    );
    unsafe { room::doom::m_controls::key_dmw_reload = i32::from(b'r') };

    // Use key rebound: the door prompt follows it.
    unsafe { room::doom::m_controls::key_use = i32::from(b'e') };
    D_Display();
    assert_eq!(prompt().as_deref(), Some("Press E to open"));
    unsafe { room::doom::m_controls::key_use = i32::from(b'f') };

    // Top up the magazine for the plain-prompt golden (no reload prompt in the way).
    tap(&mut h, b'r');
    h.run_tics(120);
    go(&mut h, door_spot);
    assert_eq!(hud::reload_prompt_text(), None);
    assert_eq!(prompt().as_deref(), Some("Press F to open"));
    assert_golden("use_prompt_door", &h.frame(), Some(VIEW_CROP));

    // Menus and the dead hide it.
    // SAFETY: plain ints on the engine thread; restored right after.
    unsafe { room::doom::m_menu::menuactive = 1 };
    D_Display();
    assert_eq!(prompt(), None, "hidden while a menu is open");
    unsafe { room::doom::m_menu::menuactive = 0 };
    D_Display();
    assert!(prompt().is_some());
    let hp = h.player().health;
    unsafe {
        (*(&raw mut room::doom::g_game::players)
            .cast::<room::doom::d_player::PlayerT>()
            .add(room::doom::g_game::consoleplayer as usize))
        .health = 0;
    }
    D_Display();
    assert_eq!(prompt(), None, "hidden when dead");
    unsafe {
        (*(&raw mut room::doom::g_game::players)
            .cast::<room::doom::d_player::PlayerT>()
            .add(room::doom::g_game::consoleplayer as usize))
        .health = hp;
    }
    D_Display();
    assert!(prompt().is_some());

    // Scoped: the sniper's overlay hides it.
    tap(&mut h, b'7');
    for _ in 0..120 {
        h.run_tics(1);
        if h.sim_view()
            .unwrap()
            .weapon
            .is_some_and(|w| pack.try_facts(w).is_some_and(|f| f.has_overlay))
        {
            break;
        }
    }
    h.run_tics(40);
    h.mouse(0, 0, 2);
    for _ in 0..120 {
        h.run_tics(1);
        if h.sim_view().unwrap().overlay_active {
            break;
        }
    }
    assert!(h.sim_view().unwrap().overlay_active, "scoped in");
    D_Display();
    assert_eq!(prompt(), None, "hidden while scoped");
    h.mouse(0, 0, 0);
    h.run_tics(60);
    D_Display();
    assert_eq!(prompt().as_deref(), Some("Press F to open"));

    // ---- F opens the door; the prompt goes away while it moves and stays away ----
    assert!(!door_open(door_back));
    h.hold(b'f', 3);
    h.run_tics(3);
    D_Display();
    assert_eq!(prompt(), None, "no prompt once the door is opening");
    h.run_tics(60);
    assert!(door_open(door_back), "F opened the door");
    D_Display();
    assert_eq!(
        prompt(),
        None,
        "no prompt while the door is open or closing"
    );
    let _ = test_api::handled_tics();

    // ---- GR (gun) line on E2M8: never a use prompt ----
    h.new_game(2, 2, 8);
    h.run_tics(2);
    let (x1, y1, x2, y2, special) = GUN_SWITCH.line.unwrap();
    assert_eq!(special, 46);
    go(&mut h, stand_before((x1, y1), (x2, y2), 32.0));
    assert_eq!(prompt(), None, "no prompt at a gun-triggered door line");

    // ---- vanilla mode: F still opens doors, no prompt is drawn ----
    h.new_game(2, 1, 1);
    h.remove_monsters();
    h.set_mw2_mode(Some(false));
    go(&mut h, door_spot);
    assert_eq!(prompt(), None, "no prompt outside MW2 mode");
    h.hold(b'f', 3);
    h.run_tics(60);
    assert!(door_open(door_back), "F opens doors in vanilla mode too");
    h.set_mw2_mode(None);
}
