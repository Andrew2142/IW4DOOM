// SPDX-License-Identifier: GPL-3.0-or-later
//! `doommw2.toml` schema and loader (decision D18, sheet 02 section 5.7).
//!
//! [`DmwConfig`] is the full typed schema; later sheets only read it. Every field has a
//! default (`docs/specs/00-overview-and-contracts.md` section 9). Unknown keys log one
//! warning each and are ignored, a file that does not parse falls back to defaults, and
//! out-of-range values are clamped with a warning. Nothing here is ever fatal.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use log::{info, warn};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::consts::{DoomAmmo, VIEWMODEL_FOV_DEFAULT};
use crate::doom::doomkeys::{
    KEY_BACKSPACE, KEY_DEL, KEY_DOWNARROW, KEY_END, KEY_ENTER, KEY_F1, KEY_F10, KEY_F2, KEY_F3,
    KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8, KEY_F9, KEY_HOME, KEY_INS, KEY_LEFTARROW, KEY_PGDN,
    KEY_PGUP, KEY_RALT, KEY_RCTRL, KEY_RIGHTARROW, KEY_RSHIFT, KEY_TAB, KEY_UPARROW,
};

const TARGET: &str = "dmw::config";

/// File name searched for in the working directory and the user config directory.
pub const CONFIG_FILE_NAME: &str = "doommw2.toml";

/// Entire `doommw2.toml`. Missing sections and keys take their defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DmwConfig {
    /// `[general]`.
    pub general: GeneralCfg,
    /// `[weapons]`: Doom weapon to MW2 weapon mapping.
    pub weapons: WeaponsCfg,
    /// `[ammo]`: maxima and pickup scaling per pool.
    pub ammo: AmmoCfg,
    /// `[balance]`: damage and speed scaling.
    pub balance: BalanceCfg,
    /// `[render]`.
    pub render: RenderCfg,
    /// `[input]`.
    pub input: InputCfg,
    /// `[audio]`.
    pub audio: AudioCfg,
    /// `[hud]`.
    pub hud: HudCfg,
}

/// `[general]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralCfg {
    /// Master switch for MW2 mode (decision D15 still applies on top).
    pub enabled: bool,
    /// Allow falling back to `packs/fixture` when no real pack is present.
    pub allow_fixture_pack: bool,
}

impl Default for GeneralCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_fixture_pack: false,
        }
    }
}

/// `[weapons]`: each value is an MW2 weapon name (for example `"m4_mp"`) or `"vanilla"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeaponsCfg {
    /// Doom slot 1, `wp_fist`.
    pub fist: String,
    /// Doom slot 1, `wp_chainsaw`.
    pub chainsaw: String,
    /// Doom slot 2, `wp_pistol`.
    pub pistol: String,
    /// Doom slot 3, `wp_shotgun`.
    pub shotgun: String,
    /// Doom slot 3, `wp_supershotgun`.
    pub supershotgun: String,
    /// Doom slot 4, `wp_chaingun`.
    pub chaingun: String,
    /// Doom slot 5, `wp_missile`.
    pub missile: String,
    /// Doom slot 6, `wp_plasma`.
    pub plasma: String,
    /// Doom slot 7, `wp_bfg`.
    pub bfg: String,
}

impl Default for WeaponsCfg {
    fn default() -> Self {
        let s = |name: &str| name.to_owned();
        Self {
            fist: s("vanilla"),
            chainsaw: s("vanilla"),
            pistol: s("usp_mp"),
            shotgun: s("spas12_mp"),
            supershotgun: s("ranger_mp"),
            chaingun: s("m4_mp"),
            missile: s("rpg_mp"),
            plasma: s("mg4_mp"),
            bfg: s("barrett_mp"),
        }
    }
}

/// `[ammo]`. Arrays are indexed by [`DoomAmmo`] (`am_clip` 0, `am_shell` 1, `am_cell` 2,
/// `am_misl` 3) in memory and written as `{ clip, shell, cell, misl }` tables in TOML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AmmoCfg {
    /// Maximum reserve per pool without a backpack.
    #[serde(with = "ammo_max_table")]
    pub max: [i32; 4],
    /// Pickup multiplier per pool.
    #[serde(with = "ammo_scale_table")]
    pub pickup_scale: [f32; 4],
}

impl Default for AmmoCfg {
    fn default() -> Self {
        Self {
            max: [360, 64, 300, 12],
            pickup_scale: [3.0, 1.0, 0.5, 1.0],
        }
    }
}

impl AmmoCfg {
    /// Maximum reserve for a pool, or `None` for an index that is not a real pool.
    pub fn max_for(&self, ammo: DoomAmmo) -> Option<i32> {
        usize::try_from(ammo)
            .ok()
            .and_then(|i| self.max.get(i).copied())
    }
}

/// TOML shape of `ammo.max`.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct AmmoMaxTable {
    clip: i32,
    shell: i32,
    cell: i32,
    misl: i32,
}

impl Default for AmmoMaxTable {
    fn default() -> Self {
        let [clip, shell, cell, misl] = AmmoCfg::default().max;
        Self {
            clip,
            shell,
            cell,
            misl,
        }
    }
}

mod ammo_max_table {
    use super::*;

    pub fn serialize<S: Serializer>(v: &[i32; 4], s: S) -> Result<S::Ok, S::Error> {
        AmmoMaxTable {
            clip: v[0],
            shell: v[1],
            cell: v[2],
            misl: v[3],
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[i32; 4], D::Error> {
        let t = AmmoMaxTable::deserialize(d)?;
        Ok([t.clip, t.shell, t.cell, t.misl])
    }
}

/// TOML shape of `ammo.pickup_scale`.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct AmmoScaleTable {
    clip: f32,
    shell: f32,
    cell: f32,
    misl: f32,
}

impl Default for AmmoScaleTable {
    fn default() -> Self {
        let [clip, shell, cell, misl] = AmmoCfg::default().pickup_scale;
        Self {
            clip,
            shell,
            cell,
            misl,
        }
    }
}

mod ammo_scale_table {
    use super::*;

    pub fn serialize<S: Serializer>(v: &[f32; 4], s: S) -> Result<S::Ok, S::Error> {
        AmmoScaleTable {
            clip: v[0],
            shell: v[1],
            cell: v[2],
            misl: v[3],
        }
        .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[f32; 4], D::Error> {
        let t = AmmoScaleTable::deserialize(d)?;
        Ok([t.clip, t.shell, t.cell, t.misl])
    }
}

/// `[balance]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BalanceCfg {
    /// Multiplier on MW2 damage against monsters.
    pub monster_damage_scale: f32,
    /// Multiplier on MW2 explosion damage.
    pub explosive_damage_scale: f32,
    /// Hit height selects a hit location (headshots).
    pub headshot_enabled: bool,
    /// Multiplier on MW2 melee damage against monsters.
    pub melee_damage_scale: f32,
    /// Call `P_NoiseAlert` on every shot, as Doom does.
    pub alert_monsters_on_fire: bool,
    /// MW2 `player_sprintSpeedScale` applied on top of Doom run speed.
    pub sprint_speed_scale: f32,
    /// Apply MW2 per-weapon move speed and ADS move speed scales to Doom movement.
    pub weapon_move_scales: bool,
}

impl Default for BalanceCfg {
    fn default() -> Self {
        Self {
            monster_damage_scale: 0.35,
            explosive_damage_scale: 0.6,
            headshot_enabled: true,
            melee_damage_scale: 1.0,
            alert_monsters_on_fire: true,
            sprint_speed_scale: 1.5,
            weapon_move_scales: true,
        }
    }
}

/// `[render]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderCfg {
    /// Present the frame at 4:3 (letterboxed) instead of stretching.
    pub aspect_correct: bool,
    /// Viewmodel horizontal FOV in MW2 `cg_fov` space.
    pub viewmodel_fov: f32,
    /// Texture quantization dither.
    pub dither: DitherCfg,
    /// Per-vertex directional shading term on the viewmodel.
    pub directional_shading: bool,
    /// Muzzle flash style.
    pub muzzle_flash: FlashCfg,
}

impl Default for RenderCfg {
    fn default() -> Self {
        Self {
            aspect_correct: true,
            viewmodel_fov: VIEWMODEL_FOV_DEFAULT,
            dither: DitherCfg::Bayer4,
            directional_shading: true,
            muzzle_flash: FlashCfg::Doom,
        }
    }
}

/// `[render] dither`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DitherCfg {
    /// Nearest palette colour only.
    None,
    /// 4x4 Bayer ordered dither.
    Bayer4,
}

/// `[render] muzzle_flash`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlashCfg {
    /// Doom's flash patch drawn at `tag_flash`.
    Doom,
    /// No muzzle flash.
    None,
}

/// `[input]`.
///
/// The ten DOOMMW2 bindings (00 section 8.3) live here instead of `default.cfg`, which this
/// engine never reads or writes (sheet 05 section 11 item 1). `on_engine_init` copies them into
/// the `m_controls` statics, which stay the runtime values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputCfg {
    /// Use raw, unaccelerated mouse motion.
    pub raw_mouse: bool,
    /// Grab the cursor as soon as the window opens.
    pub grab_on_start: bool,
    /// Move forward (Doom `key_up`).
    pub key_up: KeyBind,
    /// Move back (Doom `key_down`).
    pub key_down: KeyBind,
    /// Strafe left (Doom `key_strafeleft`).
    pub key_strafeleft: KeyBind,
    /// Strafe right (Doom `key_straferight`).
    pub key_straferight: KeyBind,
    /// Open doors and press switches (Doom `key_use`).
    pub key_use: KeyBind,
    /// Reload key.
    pub key_dmw_reload: KeyBind,
    /// ADS key (unbound by default; ADS is on the right mouse button).
    pub key_dmw_ads: KeyBind,
    /// ADS mouse button (0 left, 1 right, 2 middle, 3 mouse4, 4 mouse5; -1 unbound).
    pub mouseb_dmw_ads: i32,
    /// Sprint / hold-breath key.
    pub key_dmw_sprint: KeyBind,
    /// Melee key.
    pub key_dmw_melee: KeyBind,
    /// Melee mouse button.
    pub mouseb_dmw_melee: i32,
    /// Last-weapon key.
    pub key_dmw_swap: KeyBind,
    /// Mouse Y controls pitch (`false` = vanilla forward/back).
    pub mouse_dmw_look: bool,
    /// Invert pitch.
    pub mouse_dmw_invert: bool,
    /// Pitch sensitivity, same scale as Doom's mouse sensitivity.
    pub mouse_dmw_pitch_sens: i32,
}

impl Default for InputCfg {
    fn default() -> Self {
        Self {
            raw_mouse: true,
            grab_on_start: true,
            key_up: KeyBind(i32::from(b'w')),
            key_down: KeyBind(i32::from(b's')),
            key_strafeleft: KeyBind(i32::from(b'a')),
            key_straferight: KeyBind(i32::from(b'd')),
            key_use: KeyBind(i32::from(b'f')),
            key_dmw_reload: KeyBind(i32::from(b'r')),
            key_dmw_ads: KeyBind::UNBOUND,
            mouseb_dmw_ads: 1,
            key_dmw_sprint: KeyBind(i32::from(KEY_RSHIFT)),
            key_dmw_melee: KeyBind(i32::from(b'v')),
            mouseb_dmw_melee: 3,
            key_dmw_swap: KeyBind(i32::from(b'q')),
            mouse_dmw_look: true,
            mouse_dmw_invert: false,
            mouse_dmw_pitch_sens: 5,
        }
    }
}

/// Highest mouse button index Doom tracks (`MAX_MOUSE_BUTTONS - 1`).
pub const MAX_MOUSE_BUTTON: i32 = 7;

/// A Doom key code (`doomkeys.h` encoding), `-1` when unbound.
///
/// In TOML a binding is either a string or an integer key code. Strings are a single
/// printable character (`"r"`), `"none"`, or one of the names in [`KEY_NAMES`] (`"rshift"`,
/// `"f5"`, `"uparrow"`, ...). It is written back as a name or character when one exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyBind(pub i32);

impl KeyBind {
    /// No key.
    pub const UNBOUND: KeyBind = KeyBind(-1);

    /// Parses the string form; `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<KeyBind> {
        let lower = name.trim().to_ascii_lowercase();
        if lower.is_empty() || lower == "none" || lower == "unbound" {
            return Some(KeyBind::UNBOUND);
        }
        if let Some(&(_, code)) = KEY_NAMES.iter().find(|(n, _)| *n == lower) {
            return Some(KeyBind(i32::from(code)));
        }
        let mut chars = lower.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_graphic() => Some(KeyBind(c as i32)),
            _ => None,
        }
    }

    /// The preferred string form, or `None` when the code has no name (written as a number).
    pub fn name(self) -> Option<String> {
        if self.0 < 0 {
            return Some("none".to_owned());
        }
        if let Some(&(n, _)) = KEY_NAMES.iter().find(|(_, c)| i32::from(*c) == self.0) {
            return Some(n.to_owned());
        }
        u8::try_from(self.0)
            .ok()
            .filter(|b| b.is_ascii_graphic() && !b.is_ascii_uppercase())
            .map(|b| char::from(b).to_string())
    }
}

/// Names accepted for non-printable keys (lower case), with their Doom key codes. The first
/// name of a code is the one written back.
pub const KEY_NAMES: [(&str, u8); 30] = [
    ("rshift", KEY_RSHIFT),
    ("shift", KEY_RSHIFT),
    ("rctrl", KEY_RCTRL),
    ("ctrl", KEY_RCTRL),
    ("ralt", KEY_RALT),
    ("alt", KEY_RALT),
    ("space", b' '),
    ("tab", KEY_TAB),
    ("enter", KEY_ENTER),
    ("backspace", KEY_BACKSPACE),
    ("uparrow", KEY_UPARROW),
    ("downarrow", KEY_DOWNARROW),
    ("leftarrow", KEY_LEFTARROW),
    ("rightarrow", KEY_RIGHTARROW),
    ("home", KEY_HOME),
    ("end", KEY_END),
    ("pgup", KEY_PGUP),
    ("pgdn", KEY_PGDN),
    ("ins", KEY_INS),
    ("del", KEY_DEL),
    ("f1", KEY_F1),
    ("f2", KEY_F2),
    ("f3", KEY_F3),
    ("f4", KEY_F4),
    ("f5", KEY_F5),
    ("f6", KEY_F6),
    ("f7", KEY_F7),
    ("f8", KEY_F8),
    ("f9", KEY_F9),
    ("f10", KEY_F10),
];

impl Serialize for KeyBind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.name() {
            Some(n) => s.serialize_str(&n),
            None => s.serialize_i64(i64::from(self.0)),
        }
    }
}

impl<'de> Deserialize<'de> for KeyBind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = KeyBind;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a key name such as \"r\" or \"rshift\", \"none\", or a key code")
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<KeyBind, E> {
                match v {
                    -1 => Ok(KeyBind::UNBOUND),
                    0..=255 => Ok(KeyBind(v as i32)),
                    _ => Err(E::custom(format!("key code {v} out of range -1..=255"))),
                }
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<KeyBind, E> {
                self.visit_i64(i64::try_from(v).unwrap_or(i64::MAX))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<KeyBind, E> {
                KeyBind::from_name(v).ok_or_else(|| E::custom(format!("unknown key name \"{v}\"")))
            }
        }
        d.deserialize_any(Visitor)
    }
}

/// `[audio]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioCfg {
    /// Volume multiplier for MW2 weapon sounds.
    pub mw2_volume: f32,
    /// Extra mixer channels reserved for MW2 sounds.
    pub channels: usize,
}

impl Default for AudioCfg {
    fn default() -> Self {
        Self {
            mw2_volume: 1.0,
            channels: 8,
        }
    }
}

/// `[hud]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HudCfg {
    /// Draw the spread crosshair.
    pub crosshair: bool,
    /// Draw the hit marker.
    pub hitmarker: bool,
    /// Show the magazine count in the AMMO widget.
    pub ammo_counter: bool,
    /// Show the MW2 weapon name when switching.
    pub weapon_names: bool,
}

impl Default for HudCfg {
    fn default() -> Self {
        Self {
            crosshair: true,
            hitmarker: true,
            ammo_counter: true,
            weapon_names: true,
        }
    }
}

impl DmwConfig {
    /// Finds and parses the config file.
    ///
    /// With `explicit` (`-mw2config <file>`) only that file is tried. Otherwise the search is
    /// `./doommw2.toml`, then `$XDG_CONFIG_HOME/doommw2/doommw2.toml`, falling back to
    /// `~/.config/doommw2/doommw2.toml`. Returns the config and the file that was found
    /// (even when it failed to parse and defaults were used).
    pub fn load(explicit: Option<&Path>) -> (DmwConfig, Option<PathBuf>) {
        let candidates = candidate_paths(
            explicit,
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        );
        Self::load_from(&candidates)
    }

    /// Like [`DmwConfig::load`] with an explicit candidate list, tried in order.
    pub fn load_from(candidates: &[PathBuf]) -> (DmwConfig, Option<PathBuf>) {
        for path in candidates {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    info!(target: TARGET, "config: {}", path.display());
                    let cfg = Self::from_toml_str(&text, &path.display().to_string());
                    return (cfg, Some(path.clone()));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn!(target: TARGET, "cannot read {}: {e}", path.display()),
            }
        }
        info!(target: TARGET, "config: defaults (no {CONFIG_FILE_NAME} found)");
        (DmwConfig::default(), None)
    }

    /// Parses `text` (read from `origin`, used only in messages), warns about unknown keys,
    /// clamps out-of-range values. A syntax or type error logs `file:line:col` and returns
    /// the defaults.
    pub fn from_toml_str(text: &str, origin: &str) -> DmwConfig {
        let user_table = match text.parse::<toml::Table>() {
            Ok(t) => t,
            Err(e) => return parse_failure(text, origin, &e),
        };
        let mut cfg = match toml::from_str::<DmwConfig>(text) {
            Ok(c) => c,
            Err(e) => return parse_failure(text, origin, &e),
        };
        for key in unknown_keys(&user_table) {
            warn!(target: TARGET, "unknown key {key}");
        }
        cfg.validate();
        cfg
    }

    /// Clamps out-of-range values, one warning per clamp.
    ///
    /// `viewmodel_fov` to 40..=100, scales to 0.0..=10.0, `channels` to 1..=16. A non-finite
    /// float is replaced by its default.
    pub fn validate(&mut self) {
        let defaults = DmwConfig::default();
        clamp_f32(
            "render.viewmodel_fov",
            &mut self.render.viewmodel_fov,
            40.0,
            100.0,
            defaults.render.viewmodel_fov,
        );
        let b = &mut self.balance;
        let bd = &defaults.balance;
        clamp_scale(
            "balance.monster_damage_scale",
            &mut b.monster_damage_scale,
            bd.monster_damage_scale,
        );
        clamp_scale(
            "balance.explosive_damage_scale",
            &mut b.explosive_damage_scale,
            bd.explosive_damage_scale,
        );
        clamp_scale(
            "balance.melee_damage_scale",
            &mut b.melee_damage_scale,
            bd.melee_damage_scale,
        );
        clamp_scale(
            "balance.sprint_speed_scale",
            &mut b.sprint_speed_scale,
            bd.sprint_speed_scale,
        );
        clamp_scale(
            "audio.mw2_volume",
            &mut self.audio.mw2_volume,
            defaults.audio.mw2_volume,
        );
        const POOLS: [&str; 4] = [
            "ammo.pickup_scale.clip",
            "ammo.pickup_scale.shell",
            "ammo.pickup_scale.cell",
            "ammo.pickup_scale.misl",
        ];
        for (i, name) in POOLS.iter().enumerate() {
            clamp_scale(
                name,
                &mut self.ammo.pickup_scale[i],
                defaults.ammo.pickup_scale[i],
            );
        }
        let inp = &mut self.input;
        for (name, button) in [
            ("input.mouseb_dmw_ads", &mut inp.mouseb_dmw_ads),
            ("input.mouseb_dmw_melee", &mut inp.mouseb_dmw_melee),
        ] {
            let clamped = (*button).clamp(-1, MAX_MOUSE_BUTTON);
            if clamped != *button {
                warn!(target: TARGET, "{name} {button} out of range -1..={MAX_MOUSE_BUTTON}, clamped to {clamped}");
                *button = clamped;
            }
        }
        let sens = inp.mouse_dmw_pitch_sens;
        let clamped = sens.clamp(0, 50);
        if clamped != sens {
            warn!(target: TARGET, "input.mouse_dmw_pitch_sens {sens} out of range 0..=50, clamped to {clamped}");
            inp.mouse_dmw_pitch_sens = clamped;
        }
        let channels = self.audio.channels;
        let clamped = channels.clamp(1, 16);
        if clamped != channels {
            warn!(target: TARGET, "audio.channels {channels} out of range 1..=16, clamped to {clamped}");
            self.audio.channels = clamped;
        }
    }
}

/// Config search order for [`DmwConfig::load`], as a pure function of its inputs.
pub fn candidate_paths(
    explicit: Option<&Path>,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Vec<PathBuf> {
    if let Some(path) = explicit {
        return vec![path.to_path_buf()];
    }
    let mut out = vec![PathBuf::from(CONFIG_FILE_NAME)];
    let xdg = xdg_config_home.filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = xdg.or_else(|| {
        home.filter(|v| !v.is_empty())
            .map(|h| PathBuf::from(h).join(".config"))
    });
    if let Some(base) = base {
        out.push(base.join("doommw2").join(CONFIG_FILE_NAME));
    }
    out
}

/// Dotted paths of keys in `user` that the schema does not know, in sorted order.
pub fn unknown_keys(user: &toml::Table) -> Vec<String> {
    let known = match toml::Table::try_from(DmwConfig::default()) {
        Ok(t) => t,
        Err(e) => {
            warn!(target: TARGET, "cannot build schema table: {e}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    collect_unknown(user, &known, "", &mut out);
    out
}

fn collect_unknown(user: &toml::Table, known: &toml::Table, prefix: &str, out: &mut Vec<String>) {
    for (key, value) in user {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match known.get(key) {
            None => out.push(path),
            Some(toml::Value::Table(known_child)) => {
                if let toml::Value::Table(user_child) = value {
                    collect_unknown(user_child, known_child, &path, out);
                }
            }
            Some(_) => {}
        }
    }
}

fn parse_failure(text: &str, origin: &str, e: &toml::de::Error) -> DmwConfig {
    let location = e
        .span()
        .map(|span| {
            let before = &text[..span.start.min(text.len())];
            let line = before.matches('\n').count() + 1;
            let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
            format!(":{line}:{col}")
        })
        .unwrap_or_default();
    warn!(target: TARGET, "{origin}{location}: {} - using defaults", e.message());
    DmwConfig::default()
}

fn clamp_f32(name: &str, value: &mut f32, lo: f32, hi: f32, default: f32) {
    if !value.is_finite() {
        warn!(target: TARGET, "{name} is not finite, using default {default}");
        *value = default;
        return;
    }
    let clamped = value.clamp(lo, hi);
    if clamped.to_bits() != value.to_bits() {
        warn!(target: TARGET, "{name} {value} out of range {lo}..={hi}, clamped to {clamped}");
        *value = clamped;
    }
}

fn clamp_scale(name: &str, value: &mut f32, default: f32) {
    clamp_f32(name, value, 0.0, 10.0, default);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Logger that records `dmw::config` warnings so tests can assert on them.
    struct Capture;
    static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static CAPTURE: Capture = Capture;

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.target() == TARGET && record.level() <= log::Level::Warn {
                if let Ok(mut r) = RECORDS.lock() {
                    r.push(record.args().to_string());
                }
            }
        }
        fn flush(&self) {}
    }

    fn capture_init() {
        // Another test in this process may have installed it already; both cases are fine.
        let _ = log::set_logger(&CAPTURE);
        log::set_max_level(log::LevelFilter::Info);
    }

    fn warnings_containing(needle: &str) -> usize {
        RECORDS
            .lock()
            .map(|r| r.iter().filter(|m| m.contains(needle)).count())
            .unwrap_or(0)
    }

    const EXAMPLE: &str = include_str!("../../../doommw2.example.toml");

    #[test]
    fn example_file_equals_default() {
        let parsed: DmwConfig = toml::from_str(EXAMPLE).expect("example parses");
        assert_eq!(parsed, DmwConfig::default());
        let table: toml::Table = EXAMPLE.parse().expect("example is a table");
        assert_eq!(unknown_keys(&table), Vec::<String>::new());
    }

    #[test]
    fn default_serializes_and_parses_back() {
        let text = toml::to_string(&DmwConfig::default()).expect("serialize");
        let back: DmwConfig = toml::from_str(&text).expect("reparse");
        assert_eq!(back, DmwConfig::default());
    }

    #[test]
    fn ammo_is_written_as_pool_tables() {
        let cfg: DmwConfig =
            toml::from_str("[ammo.max]\nclip = 100\n[ammo.pickup_scale]\nmisl = 2.5\n")
                .expect("parses");
        assert_eq!(cfg.ammo.max, [100, 64, 300, 12]);
        assert_eq!(cfg.ammo.pickup_scale, [3.0, 1.0, 0.5, 2.5]);
        assert_eq!(cfg.ammo.max_for(0), Some(100));
        assert_eq!(cfg.ammo.max_for(5), None);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let cfg = DmwConfig::from_toml_str("[weapons]\npistol = \"deserteagle_mp\"\n", "t");
        assert_eq!(cfg.weapons.pistol, "deserteagle_mp");
        assert_eq!(cfg.weapons.shotgun, "spas12_mp");
        assert_eq!(cfg.render, RenderCfg::default());
    }

    #[test]
    fn enums_use_lowercase_names() {
        let cfg: DmwConfig =
            toml::from_str("[render]\ndither = \"none\"\nmuzzle_flash = \"none\"\n").expect("ok");
        assert_eq!(cfg.render.dither, DitherCfg::None);
        assert_eq!(cfg.render.muzzle_flash, FlashCfg::None);
    }

    #[test]
    fn unknown_keys_are_reported_with_dotted_paths() {
        let table: toml::Table = "bogus = 1\n[balance]\nnope = true\n[ammo.max]\nrockets = 3\n"
            .parse()
            .expect("toml");
        assert_eq!(
            unknown_keys(&table),
            ["ammo.max.rockets", "balance.nope", "bogus"]
        );
    }

    #[test]
    fn unknown_key_warns_and_still_parses() {
        capture_init();
        let cfg = DmwConfig::from_toml_str(
            "[render]\nviewmodel_fov = 70.0\nzzz_unknown_render_key = 1\n",
            "t",
        );
        assert_eq!(cfg.render.viewmodel_fov.to_bits(), 70.0f32.to_bits());
        assert_eq!(
            warnings_containing("unknown key render.zzz_unknown_render_key"),
            1
        );
    }

    #[test]
    fn syntax_error_warns_with_location_and_uses_defaults() {
        capture_init();
        let cfg = DmwConfig::from_toml_str("[render]\naspect_correct = = false\n", "syntax.toml");
        assert_eq!(cfg, DmwConfig::default());
        assert_eq!(warnings_containing("syntax.toml:2:"), 1);
    }

    #[test]
    fn type_error_uses_defaults() {
        capture_init();
        let cfg = DmwConfig::from_toml_str("[audio]\nchannels = \"many\"\n", "type.toml");
        assert_eq!(cfg, DmwConfig::default());
        assert_eq!(warnings_containing("type.toml:2:"), 1);
    }

    #[test]
    fn viewmodel_fov_is_clamped() {
        capture_init();
        let lo = DmwConfig::from_toml_str("[render]\nviewmodel_fov = 10.0\n", "t");
        assert_eq!(lo.render.viewmodel_fov.to_bits(), 40.0f32.to_bits());
        let hi = DmwConfig::from_toml_str("[render]\nviewmodel_fov = 170.0\n", "t");
        assert_eq!(hi.render.viewmodel_fov.to_bits(), 100.0f32.to_bits());
        assert!(warnings_containing("render.viewmodel_fov") >= 2);
    }

    #[test]
    fn scales_are_clamped_to_zero_through_ten() {
        capture_init();
        let cfg = DmwConfig::from_toml_str(
            "[balance]\nmonster_damage_scale = -1.0\nexplosive_damage_scale = 99.0\n\
             melee_damage_scale = 11.0\nsprint_speed_scale = -0.5\n\
             [audio]\nmw2_volume = 50.0\n[ammo.pickup_scale]\nclip = 12.0\nmisl = -3.0\n",
            "t",
        );
        assert_eq!(cfg.balance.monster_damage_scale.to_bits(), 0.0f32.to_bits());
        assert_eq!(
            cfg.balance.explosive_damage_scale.to_bits(),
            10.0f32.to_bits()
        );
        assert_eq!(cfg.balance.melee_damage_scale.to_bits(), 10.0f32.to_bits());
        assert_eq!(cfg.balance.sprint_speed_scale.to_bits(), 0.0f32.to_bits());
        assert_eq!(cfg.audio.mw2_volume.to_bits(), 10.0f32.to_bits());
        assert_eq!(cfg.ammo.pickup_scale[0].to_bits(), 10.0f32.to_bits());
        assert_eq!(cfg.ammo.pickup_scale[3].to_bits(), 0.0f32.to_bits());
        assert!(warnings_containing("balance.monster_damage_scale") >= 1);
        assert!(warnings_containing("ammo.pickup_scale.misl") >= 1);
    }

    #[test]
    fn non_finite_float_falls_back_to_default() {
        capture_init();
        let cfg = DmwConfig::from_toml_str("[render]\nviewmodel_fov = nan\n", "t");
        assert_eq!(
            cfg.render.viewmodel_fov.to_bits(),
            VIEWMODEL_FOV_DEFAULT.to_bits()
        );
    }

    #[test]
    fn channels_are_clamped_to_one_through_sixteen() {
        capture_init();
        let lo = DmwConfig::from_toml_str("[audio]\nchannels = 0\n", "t");
        assert_eq!(lo.audio.channels, 1);
        let hi = DmwConfig::from_toml_str("[audio]\nchannels = 64\n", "t");
        assert_eq!(hi.audio.channels, 16);
        assert!(warnings_containing("audio.channels") >= 2);
    }

    #[test]
    fn key_bindings_accept_names_characters_and_codes() {
        let cfg: DmwConfig = toml::from_str(
            "[input]\nkey_dmw_reload = \"T\"\nkey_dmw_ads = \"rctrl\"\n\
             key_dmw_sprint = 32\nkey_dmw_melee = \"none\"\nkey_dmw_swap = -1\n",
        )
        .expect("parses");
        assert_eq!(cfg.input.key_dmw_reload, KeyBind(i32::from(b't')));
        let moved: DmwConfig = toml::from_str(
            "[input]\nkey_up = \"uparrow\"\nkey_down = \"downarrow\"\nkey_strafeleft = \"q\"\n\
             key_straferight = \"e\"\nkey_use = \"space\"\n",
        )
        .expect("parses");
        assert_eq!(moved.input.key_up, KeyBind(i32::from(KEY_UPARROW)));
        assert_eq!(moved.input.key_down, KeyBind(i32::from(KEY_DOWNARROW)));
        assert_eq!(moved.input.key_strafeleft, KeyBind(i32::from(b'q')));
        assert_eq!(moved.input.key_straferight, KeyBind(i32::from(b'e')));
        assert_eq!(moved.input.key_use, KeyBind(i32::from(b' ')));
        let round: DmwConfig =
            toml::from_str(&toml::to_string(&moved).expect("serialises")).expect("round trips");
        assert_eq!(round, moved);
        assert_eq!(cfg.input.key_dmw_ads, KeyBind(i32::from(KEY_RCTRL)));
        assert_eq!(cfg.input.key_dmw_sprint, KeyBind(32));
        assert_eq!(cfg.input.key_dmw_melee, KeyBind::UNBOUND);
        assert_eq!(cfg.input.key_dmw_swap, KeyBind::UNBOUND);
        assert!(toml::from_str::<DmwConfig>("[input]\nkey_dmw_reload = \"bogus\"\n").is_err());
        assert!(toml::from_str::<DmwConfig>("[input]\nkey_dmw_reload = 300\n").is_err());
    }

    #[test]
    fn key_binding_defaults_match_the_contract() {
        // 00 section 8.3.
        let i = InputCfg::default();
        // Contract change request (sheet 05 section 11): WASD movement and F use.
        assert_eq!(i.key_up.0, i32::from(b'w'));
        assert_eq!(i.key_down.0, i32::from(b's'));
        assert_eq!(i.key_strafeleft.0, i32::from(b'a'));
        assert_eq!(i.key_straferight.0, i32::from(b'd'));
        assert_eq!(i.key_use.0, i32::from(b'f'));
        assert_eq!(i.key_dmw_reload.0, 114);
        assert_eq!(i.key_dmw_ads.0, -1);
        assert_eq!(i.mouseb_dmw_ads, 1);
        assert_eq!(i.key_dmw_sprint.0, i32::from(KEY_RSHIFT));
        assert_eq!(i.key_dmw_melee.0, 118);
        assert_eq!(i.mouseb_dmw_melee, 3);
        assert_eq!(i.key_dmw_swap.0, 113);
        assert!(i.mouse_dmw_look);
        assert!(!i.mouse_dmw_invert);
        assert_eq!(i.mouse_dmw_pitch_sens, 5);
        for code in [
            -1,
            0,
            32,
            65,
            114,
            i32::from(KEY_RSHIFT),
            i32::from(KEY_F5),
            200,
        ] {
            let k = KeyBind(code);
            let back = match k.name() {
                Some(n) => KeyBind::from_name(&n),
                None => Some(k),
            };
            assert_eq!(back, Some(k), "code {code}");
        }
    }

    #[test]
    fn mouse_buttons_and_pitch_sens_are_clamped() {
        capture_init();
        let cfg = DmwConfig::from_toml_str(
            "[input]\nmouseb_dmw_ads = 12\nmouseb_dmw_melee = -5\nmouse_dmw_pitch_sens = 99\n",
            "t",
        );
        assert_eq!(cfg.input.mouseb_dmw_ads, MAX_MOUSE_BUTTON);
        assert_eq!(cfg.input.mouseb_dmw_melee, -1);
        assert_eq!(cfg.input.mouse_dmw_pitch_sens, 50);
    }

    #[test]
    fn candidate_order_matches_the_spec() {
        let explicit = candidate_paths(Some(Path::new("/x/y.toml")), Some("/xdg".into()), None);
        assert_eq!(explicit, [PathBuf::from("/x/y.toml")]);
        let xdg = candidate_paths(None, Some("/xdg".into()), Some("/home/u".into()));
        assert_eq!(
            xdg,
            [
                PathBuf::from("doommw2.toml"),
                PathBuf::from("/xdg/doommw2/doommw2.toml")
            ]
        );
        let home = candidate_paths(None, Some("".into()), Some("/home/u".into()));
        assert_eq!(
            home,
            [
                PathBuf::from("doommw2.toml"),
                PathBuf::from("/home/u/.config/doommw2/doommw2.toml")
            ]
        );
        assert_eq!(
            candidate_paths(None, None, None),
            [PathBuf::from("doommw2.toml")]
        );
    }

    #[test]
    fn load_from_uses_the_first_existing_file() {
        let dir = std::env::temp_dir().join(format!("doommw2-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let found = dir.join("found.toml");
        std::fs::write(&found, "[hud]\ncrosshair = false\n").expect("write");
        let missing = dir.join("missing.toml");

        let (cfg, used) = DmwConfig::load_from(&[missing.clone(), found.clone()]);
        assert!(!cfg.hud.crosshair);
        assert_eq!(used, Some(found.clone()));

        let (cfg, used) = DmwConfig::load_from(&[missing]);
        assert_eq!(cfg, DmwConfig::default());
        assert_eq!(used, None);

        std::fs::remove_file(&found).expect("cleanup file");
        std::fs::remove_dir(&dir).expect("cleanup dir");
    }
}
