// SPDX-License-Identifier: GPL-3.0-or-later
//! Pinned test locations on the pinned Freedoom (sheet 13 section 5.3).
//!
//! Coordinates were found by running the harness against Freedoom 0.13.0 (`wads/freedoom1.wad`,
//! SHA-256 in `tools/fetch-freedoom.sh`) with [`discover`], and recorded below. Every
//! [`Location`] carries a map fingerprint and an assertion ([`Location::assert_valid`]) that
//! fails loudly when the spot no longer satisfies its requirement, which is what a Freedoom
//! bump or a different `DMW_IWAD` looks like.
//!
//! Re-discovery: `DMW_DISCOVER_LOCATIONS=1 cargo test -p room --test locations_headless --
//! --nocapture` prints candidates for every requirement.

use std::ffi::c_void;

use room::doom::c_ffi::{line_t, sector_t, subsector_t};
use room::doom::g_game::{gameepisode, gamemap, gameskill};
use room::doom::info::MF_SOLID;
use room::doom::p_setup::{lines, numlines, numsectors, sectors};
use room::doom::r_main::R_PointInSubsector;
use room::doom::r_sky::skyflatnum;

use super::Harness;

/// `ML_BLOCKING`: the line blocks players and monsters.
const ML_BLOCKING: i16 = 1;

/// Gun-activated line specials: 24 (G1 door open), 46 (GR door open), 47 (G1 raise floor to
/// nearest higher, change texture).
pub const GUN_SPECIALS: [i16; 3] = [24, 46, 47];

/// What a [`Location`] is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Clear, level, tall floor ahead of the player; no monsters.
    OpenRoom,
    /// Stable (non-flickering) sector light at or below 96.
    Dark,
    /// A gun-activated line visible from the spot.
    GunSwitch,
    /// Outdoor spot with a sky ceiling.
    Sky,
}

/// One pinned spot.
#[derive(Debug, Clone, Copy)]
pub struct Location {
    /// Name used in messages.
    pub name: &'static str,
    /// What the spot is for.
    pub kind: Kind,
    /// Episode (1-based).
    pub episode: i32,
    /// Map number (1-based).
    pub map: i32,
    /// Player x in Doom map units.
    pub x: f32,
    /// Player y in Doom map units.
    pub y: f32,
    /// Facing, degrees counter-clockwise from +x.
    pub angle_deg: f32,
    /// `(linedef count, sector count)` of the map, to notice a different WAD at once.
    pub fingerprint: (usize, usize),
    /// For [`Kind::GunSwitch`]: the line as `(x1, y1, x2, y2, special)`.
    pub line: Option<(f32, f32, f32, f32, i16)>,
    /// Short description.
    pub desc: &'static str,
}

// ---------------------------------------------------------------------------
// Map snapshot (pure data, so the geometry checks are ordinary Rust)
// ---------------------------------------------------------------------------

/// One linedef.
#[derive(Debug, Clone, Copy)]
pub struct MapLine {
    /// First vertex.
    pub a: (f32, f32),
    /// Second vertex.
    pub b: (f32, f32),
    /// `ML_*` flags.
    pub flags: i16,
    /// Line special.
    pub special: i16,
    /// Front sector index.
    pub front: Option<usize>,
    /// Back sector index.
    pub back: Option<usize>,
}

/// One sector.
#[derive(Debug, Clone, Copy)]
pub struct MapSector {
    /// Floor height.
    pub floor: f32,
    /// Ceiling height.
    pub ceil: f32,
    /// Light level 0..=255.
    pub light: i16,
    /// Sector special.
    pub special: i16,
    /// Ceiling is the sky flat.
    pub sky: bool,
}

/// Copy of the loaded map's lines and sectors.
#[derive(Debug, Clone)]
pub struct MapSnapshot {
    /// Episode.
    pub episode: i32,
    /// Map.
    pub map: i32,
    /// Linedefs.
    pub lines: Vec<MapLine>,
    /// Sectors.
    pub sectors: Vec<MapSector>,
}

fn sector_index(p: *const c_void) -> Option<usize> {
    if p.is_null() {
        return None;
    }
    // SAFETY: `sectors` is the level's sector array; `p` points into it.
    unsafe { Some((p as usize - sectors as usize) / std::mem::size_of::<sector_t>()) }
}

impl MapSnapshot {
    /// Captures the currently loaded level.
    pub fn capture() -> MapSnapshot {
        let fu = 65536.0f32;
        // SAFETY: a level is loaded (callers run after `new_game`); the arrays are valid for
        // `numlines`/`numsectors` elements until the next level load.
        unsafe {
            let secs = (0..numsectors as usize)
                .map(|i| {
                    let s: &sector_t = &*sectors.add(i);
                    MapSector {
                        floor: s.floorheight as f32 / fu,
                        ceil: s.ceilingheight as f32 / fu,
                        light: s.lightlevel,
                        special: s.special,
                        sky: i32::from(s.ceilingpic) == skyflatnum,
                    }
                })
                .collect();
            let lns = (0..numlines as usize)
                .map(|i| {
                    let l: &line_t = &*lines.add(i);
                    let (v1, v2) = (&*l.v1, &*l.v2);
                    MapLine {
                        a: (v1.x as f32 / fu, v1.y as f32 / fu),
                        b: (v2.x as f32 / fu, v2.y as f32 / fu),
                        flags: l.flags,
                        special: l.special,
                        front: sector_index(l.frontsector),
                        back: sector_index(l.backsector),
                    }
                })
                .collect();
            MapSnapshot {
                episode: gameepisode,
                map: gamemap,
                lines: lns,
                sectors: secs,
            }
        }
    }

    /// `(linedef count, sector count)`.
    pub fn fingerprint(&self) -> (usize, usize) {
        (self.lines.len(), self.sectors.len())
    }

    /// Sector containing `(x, y)` (the engine's BSP lookup).
    pub fn sector_at(&self, x: f32, y: f32) -> usize {
        // SAFETY: a level is loaded, so the BSP tree exists.
        unsafe {
            let ss = R_PointInSubsector((x * 65536.0) as i32, (y * 65536.0) as i32);
            let ss = &*ss.cast::<subsector_t>();
            sector_index(ss.sector).expect("subsector without a sector")
        }
    }

    /// Distance from `(x, y)` to the nearest linedef.
    pub fn nearest_line_distance(&self, x: f32, y: f32) -> f32 {
        self.lines
            .iter()
            .map(|l| point_segment_distance((x, y), l.a, l.b))
            .fold(f32::INFINITY, f32::min)
    }

    /// Whether the straight segment `from`-`to` is cut by a one-sided line or by a two-sided
    /// line whose opening is lower than 48 units (a closed door, a window sill). `skip` is the
    /// target line itself.
    pub fn sight_blocked(&self, from: (f32, f32), to: (f32, f32), skip: Option<&MapLine>) -> bool {
        self.lines.iter().any(|o| {
            if skip.is_some_and(|s| std::ptr::eq(s, o)) || !segments_cross(from, to, o.a, o.b) {
                return false;
            }
            match (o.front, o.back) {
                (Some(f), Some(b)) => {
                    let (f, b) = (&self.sectors[f], &self.sectors[b]);
                    f.ceil.min(b.ceil) - f.floor.max(b.floor) < 48.0
                }
                _ => true,
            }
        })
    }

    /// Whether a line can be walked and seen across at `floor` without a step or a low ceiling.
    fn line_is_open_at(&self, l: &MapLine, floor: f32) -> bool {
        let (Some(f), Some(b)) = (l.front, l.back) else {
            return false;
        };
        if l.flags & ML_BLOCKING != 0 {
            return false;
        }
        [f, b].iter().all(|&i| {
            let s = &self.sectors[i];
            s.floor == floor && s.ceil - s.floor >= 128.0
        })
    }

    /// Length (up to `max_len`, in 16-unit steps) of the clear, level, 128-high strip of half
    /// width `half_width` that starts at `(x, y)` and runs along `angle_deg`.
    pub fn clear_length(
        &self,
        x: f32,
        y: f32,
        angle_deg: f32,
        half_width: f32,
        max_len: f32,
    ) -> f32 {
        let start = self.sector_at(x, y);
        let floor = self.sectors[start].floor;
        let (s, c) = angle_deg.to_radians().sin_cos();
        let mut len = 0.0;
        while len < max_len {
            let next = len + 16.0;
            let (px, py) = (x + c * next, y + s * next);
            let sec = &self.sectors[self.sector_at(px, py)];
            let strip_ok = sec.floor == floor && sec.ceil - sec.floor >= 128.0;
            let blocked = self.lines.iter().any(|l| {
                segment_hits_strip(l.a, l.b, (x, y), (c, s), len, next, half_width)
                    && !self.line_is_open_at(l, floor)
            });
            if !strip_ok || blocked {
                break;
            }
            len = next;
        }
        len
    }
}

fn point_segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Liang-Barsky: does segment `a`-`b` touch the strip `[u0, u1] x [-hw, hw]` whose origin is
/// `origin` and whose forward unit vector is `dir`?
fn segment_hits_strip(
    a: (f32, f32),
    b: (f32, f32),
    origin: (f32, f32),
    dir: (f32, f32),
    u0: f32,
    u1: f32,
    hw: f32,
) -> bool {
    let to_local = |p: (f32, f32)| {
        let (rx, ry) = (p.0 - origin.0, p.1 - origin.1);
        (rx * dir.0 + ry * dir.1, -rx * dir.1 + ry * dir.0)
    };
    let (ax, ay) = to_local(a);
    let (bx, by) = to_local(b);
    let (dx, dy) = (bx - ax, by - ay);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, ax - u0), (dx, u1 - ax), (-dy, ay + hw), (dy, hw - ay)] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return false;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return false;
                }
                t1 = t1.min(r);
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Pinned locations
// ---------------------------------------------------------------------------

/// Freedoom 1 E1M1 fingerprint (linedefs, sectors) of the pinned release (0.13.0).
const E1M1_FINGERPRINT: (usize, usize) = (1175, 182);
/// Freedoom 1 E1M2 fingerprint.
const E1M2_FINGERPRINT: (usize, usize) = (2290, 380);
/// Freedoom 1 E2M8 fingerprint.
const E2M8_FINGERPRINT: (usize, usize) = (364, 54);

/// Player spot with at least 512 units of clear, level floor straight ahead, ceiling at least
/// 128 above the floor, and no monsters within 1024 once the level's monsters are removed.
///
/// E1M1 sector 56, a hall at least 192 units wide; 736 units of level floor to the north.
/// Light 128.
pub const OPEN_ROOM: Location = Location {
    name: "OPEN_ROOM",
    kind: Kind::OpenRoom,
    episode: 1,
    map: 1,
    x: 864.0,
    y: 736.0,
    angle_deg: 90.0,
    fingerprint: E1M1_FINGERPRINT,
    line: None,
    desc: "E1M1 wide hall, level floor north",
};

/// Sector light 96 (the lowest in episode 1: E1M2 sector 115 is the only dark, non-flickering
/// sector of E1M1-E1M9), 256 units of clear floor to the west. Needed for lighting goldens.
pub const DARK_SPOT: Location = Location {
    name: "DARK_SPOT",
    kind: Kind::Dark,
    episode: 1,
    map: 2,
    x: 1024.0,
    y: -2272.0,
    angle_deg: 180.0,
    fingerprint: E1M2_FINGERPRINT,
    line: None,
    desc: "E1M2 sector 115, light 96",
};

/// Freedoom 1 has gun-activated doors: E2M8 line 225 is a `GR` (special 46) door-open line, 128
/// units long, visible from 160 units away facing east. T9.7 can shoot it as is, so the in-memory
/// `special = 46` fallback of sheet 13 section 5.3 is not needed.
pub const GUN_SWITCH: Location = Location {
    name: "GUN_SWITCH",
    kind: Kind::GunSwitch,
    episode: 2,
    map: 8,
    x: 968.0,
    y: -480.0,
    angle_deg: 0.0,
    fingerprint: E2M8_FINGERPRINT,
    line: Some((1128.0, -416.0, 1128.0, -544.0, 46)),
    desc: "E2M8 GR door line 225",
};

/// Outdoor E1M1 spot under the sky flat (sector 18, light 208), 624 units of open ground to the
/// west. Look up with `Harness::set_pitch(-30.0)` to fill the view with sky (T6.6).
pub const SKY_VIEW: Location = Location {
    name: "SKY_VIEW",
    kind: Kind::Sky,
    episode: 1,
    map: 1,
    x: 1920.0,
    y: 608.0,
    angle_deg: 180.0,
    fingerprint: E1M1_FINGERPRINT,
    line: None,
    desc: "E1M1 outdoor, sky ceiling",
};

/// Every pinned location.
pub const ALL: [&Location; 4] = [&OPEN_ROOM, &DARK_SPOT, &GUN_SWITCH, &SKY_VIEW];

impl Location {
    /// Puts the harness on this spot: loads the map if another one is running, removes the
    /// monsters when the requirement says so, teleports, runs one tic and asserts validity.
    pub fn go(&self, h: &mut Harness) {
        // SAFETY: plain int reads on the engine thread.
        let (ep, map, skill) = unsafe { (gameepisode, gamemap, gameskill) };
        if (ep, map) != (self.episode, self.map) {
            h.new_game(skill, self.episode, self.map);
        }
        if self.kind == Kind::OpenRoom {
            h.remove_monsters();
        }
        h.teleport(self.x, self.y, self.angle_deg);
        h.run_tics(1);
        self.assert_valid(h);
    }

    /// Fails loudly if the spot no longer satisfies its requirement on the loaded map.
    pub fn assert_valid(&self, h: &Harness) {
        let snap = MapSnapshot::capture();
        let who = format!(
            "{} (E{}M{} at {}, {} facing {}): {}",
            self.name, self.episode, self.map, self.x, self.y, self.angle_deg, self.desc
        );
        assert_eq!(
            (snap.episode, snap.map),
            (self.episode, self.map),
            "{who}: wrong map is loaded"
        );
        assert_eq!(
            snap.fingerprint(),
            self.fingerprint,
            "{who}: map fingerprint changed (lines, sectors); is this the pinned Freedoom? \
             run 'task fetch-wads' and re-pin with DMW_DISCOVER_LOCATIONS=1"
        );
        let sec = &snap.sectors[snap.sector_at(self.x, self.y)];
        assert!(
            snap.nearest_line_distance(self.x, self.y) >= 32.0,
            "{who}: spot is closer than 32 units to a wall"
        );
        match self.kind {
            Kind::OpenRoom => {
                let len = snap.clear_length(self.x, self.y, self.angle_deg, 32.0, 512.0);
                assert!(
                    len >= 512.0,
                    "{who}: only {len} units of clear level floor ahead"
                );
                self.assert_no_solids_ahead(h, &who, 512.0);
                assert_no_live_monsters_within(h, self, 1024.0, &who);
            }
            Kind::Dark => {
                assert!(
                    sec.light <= 96,
                    "{who}: sector light is {} (need <= 96)",
                    sec.light
                );
                assert_eq!(
                    sec.special, 0,
                    "{who}: sector has a lighting special (light would change)"
                );
            }
            Kind::Sky => {
                assert!(sec.sky, "{who}: sector ceiling is not the sky flat");
            }
            Kind::GunSwitch => self.assert_gun_switch(&snap, &who),
        }
    }

    fn assert_no_solids_ahead(&self, h: &Harness, who: &str, len: f32) {
        let (s, c) = self.angle_deg.to_radians().sin_cos();
        for (t, x, y, flags) in h.mobjs() {
            if flags & MF_SOLID == 0 {
                continue;
            }
            let (rx, ry) = (x - self.x, y - self.y);
            let (u, v) = (rx * c + ry * s, -rx * s + ry * c);
            // The player itself is solid and stands at u = v = 0.
            let is_self = u.abs() < 1.0 && v.abs() < 1.0;
            assert!(
                is_self || !(u >= 0.0 && u <= len + 20.0 && v.abs() <= 32.0 + 20.0),
                "{who}: solid thing type {t} at ({x}, {y}) stands in the corridor"
            );
        }
    }

    fn assert_gun_switch(&self, snap: &MapSnapshot, who: &str) {
        let (x1, y1, x2, y2, special) = self.line.expect("GunSwitch needs a line");
        let found = snap
            .lines
            .iter()
            .find(|l| l.special == special && close(l.a, (x1, y1)) && close(l.b, (x2, y2)));
        let line = found
            .unwrap_or_else(|| panic!("{who}: no line {x1},{y1}-{x2},{y2} with special {special}"));
        assert!(
            GUN_SPECIALS.contains(&line.special),
            "{who}: special {} is not gun-activated",
            line.special
        );
        let mid = ((line.a.0 + line.b.0) / 2.0, (line.a.1 + line.b.1) / 2.0);
        let dist = ((mid.0 - self.x).powi(2) + (mid.1 - self.y).powi(2)).sqrt();
        assert!(
            dist <= 256.0,
            "{who}: line is {dist} units away (need <= 256)"
        );
        let want = (mid.1 - self.y).atan2(mid.0 - self.x).to_degrees();
        let diff = (self.angle_deg - want + 540.0).rem_euclid(360.0) - 180.0;
        assert!(
            diff.abs() <= 10.0,
            "{who}: not facing the line ({diff} degrees off)"
        );
        assert!(
            !snap.sight_blocked((self.x, self.y), mid, Some(line)),
            "{who}: a wall stands between the spot and the line"
        );
    }
}

fn close(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5
}

/// Proper intersection of segments `p1`-`p2` and `p3`-`p4`, excluding the end points of the
/// first (so a ray that stops at a line does not count that line).
fn segments_cross(p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), p4: (f32, f32)) -> bool {
    let d = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let (d1, d2, d3, d4) = (d(p3, p4, p1), d(p3, p4, p2), d(p1, p2, p3), d(p1, p2, p4));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

fn assert_no_live_monsters_within(h: &Harness, loc: &Location, radius: f32, who: &str) {
    for (t, x, y, flags) in h.mobjs() {
        if flags & room::doom::info::MF_COUNTKILL == 0 {
            continue;
        }
        let d = ((x - loc.x).powi(2) + (y - loc.y).powi(2)).sqrt();
        assert!(
            d > radius,
            "{who}: monster type {t} is {d} units away (need > {radius})"
        );
    }
}

// ---------------------------------------------------------------------------
// Discovery (used once to pin the constants above; kept so a Freedoom bump can be re-pinned)
// ---------------------------------------------------------------------------

impl MapSnapshot {
    /// Whether `(x, y)` lies inside the polygon of sector `s` (even-odd ray cast along +x over
    /// the lines that bound `s`).
    pub fn inside_sector(&self, x: f32, y: f32, s: usize) -> bool {
        let mut crossings = 0;
        for l in &self.lines {
            if l.front != Some(s) && l.back != Some(s) {
                continue;
            }
            let (a, b) = (l.a, l.b);
            if (a.1 > y) == (b.1 > y) {
                continue;
            }
            let t = (y - a.1) / (b.1 - a.1);
            if a.0 + t * (b.0 - a.0) > x {
                crossings += 1;
            }
        }
        crossings % 2 == 1
    }

    fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for l in &self.lines {
            for p in [l.a, l.b] {
                b = (b.0.min(p.0), b.1.max(p.0), b.2.min(p.1), b.3.max(p.1));
            }
        }
        b
    }

    /// Grid points (step 32) that are inside their own sector and at least 48 units from walls.
    fn standable_points(&self) -> Vec<(f32, f32, usize)> {
        let (x0, x1, y0, y1) = self.bounds();
        let mut out = Vec::new();
        let mut y = (y0 / 32.0).ceil() * 32.0;
        while y <= y1 {
            let mut x = (x0 / 32.0).ceil() * 32.0;
            while x <= x1 {
                let s = self.sector_at(x, y);
                if self.inside_sector(x, y, s) && self.nearest_line_distance(x, y) >= 48.0 {
                    out.push((x, y, s));
                }
                x += 32.0;
            }
            y += 32.0;
        }
        out
    }
}

/// Prints candidate spots for every requirement of sheet 13 section 5.3. Run with
/// `DMW_DISCOVER_LOCATIONS=1`; the output is what the constants above were pinned from.
pub fn discover(h: &mut Harness) {
    let skill = 2;

    h.new_game(skill, 1, 1);
    h.remove_monsters();
    let snap = MapSnapshot::capture();
    eprintln!(
        "== E1M1 fingerprint (lines, sectors) = {:?}",
        snap.fingerprint()
    );
    let pts = snap.standable_points();
    eprintln!("   {} standable grid points", pts.len());

    let mut open = Vec::new();
    let mut sky = Vec::new();
    for &(x, y, s) in &pts {
        let sec = snap.sectors[s];
        for angle in [0.0, 90.0, 180.0, 270.0] {
            let len = snap.clear_length(x, y, angle, 32.0, 1024.0);
            if sec.special == 0 && len >= 512.0 {
                open.push((len, sec.light, x, y, angle, s));
            }
            if sec.sky && len >= 128.0 {
                sky.push((len, sec.light, x, y, angle, s));
            }
        }
    }
    // Rank by how wide the corridor is (smallest wall distance along the strip), then length.
    let width = |x: f32, y: f32, angle: f32, len: f32| {
        let (sn, cs) = angle.to_radians().sin_cos();
        (0..=(len as i32 / 64))
            .map(|i| snap.nearest_line_distance(x + cs * 64.0 * i as f32, y + sn * 64.0 * i as f32))
            .fold(f32::INFINITY, f32::min)
    };
    open.sort_by(|a, b| {
        let (wa, wb) = (width(a.2, a.3, a.4, 512.0), width(b.2, b.3, b.4, 512.0));
        wb.total_cmp(&wa).then(b.0.total_cmp(&a.0))
    });
    sky.sort_by(|a, b| b.0.total_cmp(&a.0));
    eprintln!("== OPEN_ROOM candidates (clear_len, light, x, y, angle, sector)");
    for c in open.iter().take(8) {
        eprintln!(
            "   {c:?} min wall distance along 512: {}",
            width(c.2, c.3, c.4, 512.0)
        );
    }
    eprintln!("== SKY_VIEW candidates (sky ceiling; clear_len, light, x, y, angle, sector)");
    for c in sky.iter().take(8) {
        eprintln!("   {c:?}");
    }
    let n_sky = snap.sectors.iter().filter(|s| s.sky).count();
    eprintln!("   E1M1 has {n_sky} sky-ceiling sectors");

    eprintln!("== DARK_SPOT candidates (sector light <= 96, no special)");
    for map in 1..=9 {
        h.new_game(skill, 1, map);
        let snap = MapSnapshot::capture();
        let mut dark = Vec::new();
        for (x, y, s) in snap.standable_points() {
            let sec = snap.sectors[s];
            if sec.light > 96 || sec.special != 0 {
                continue;
            }
            for angle in [0.0, 90.0, 180.0, 270.0] {
                let len = snap.clear_length(x, y, angle, 32.0, 256.0);
                dark.push((
                    sec.light,
                    -(len as i32),
                    x as i32,
                    y as i32,
                    angle as i32,
                    s,
                ));
            }
        }
        dark.sort();
        eprintln!(
            "   E1M{map}: fingerprint {:?}, {} dark candidates",
            snap.fingerprint(),
            dark.len()
        );
        for c in dark.iter().take(3) {
            eprintln!("      (light, -clear_len, x, y, angle, sector) = {c:?}");
        }
    }

    eprintln!("== GUN_SWITCH candidates (special in {GUN_SPECIALS:?})");
    for ep in 1..=3 {
        for map in 1..=9 {
            h.new_game(skill, ep, map);
            let snap = MapSnapshot::capture();
            for (i, l) in snap.lines.iter().enumerate() {
                if !GUN_SPECIALS.contains(&l.special) {
                    continue;
                }
                eprintln!(
                    "   E{ep}M{map} line {i} special {} {:?}-{:?}",
                    l.special, l.a, l.b
                );
                // Bullets trigger a gun line from either side, so try both normals.
                let mid = ((l.a.0 + l.b.0) / 2.0, (l.a.1 + l.b.1) / 2.0);
                let (dx, dy) = (l.b.0 - l.a.0, l.b.1 - l.a.1);
                let len = (dx * dx + dy * dy).sqrt();
                let (ux, uy) = (dx / len, dy / len);
                let mut found = false;
                'search: for dist in [64.0, 96.0, 128.0, 160.0, 200.0] {
                    for side in [-1.0f32, 1.0] {
                        for lateral in [0.0f32, -16.0, 16.0] {
                            let p = (
                                mid.0 + uy * dist * side + ux * lateral,
                                mid.1 - ux * dist * side + uy * lateral,
                            );
                            let s = snap.sector_at(p.0, p.1);
                            let sec = snap.sectors[s];
                            if !snap.inside_sector(p.0, p.1, s)
                                || snap.nearest_line_distance(p.0, p.1) < 24.0
                                || sec.ceil - sec.floor < 72.0
                                || snap.sight_blocked(p, mid, Some(l))
                            {
                                continue;
                            }
                            let ang = (mid.1 - p.1).atan2(mid.0 - p.0).to_degrees();
                            eprintln!(
                                "      spot ({}, {}) angle {ang:.1} dist {dist} sector {s} light {}",
                                p.0, p.1, sec.light
                            );
                            found = true;
                            break 'search;
                        }
                    }
                }
                if !found {
                    eprintln!("      no spot found");
                }
                let _ = i;
            }
        }
    }
}
