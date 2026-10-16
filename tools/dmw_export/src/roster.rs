// SPDX-License-Identifier: GPL-3.0-or-later
//! Weapon name resolution and close-match suggestions (sheet 04 §5.2).

use asset_game::WeaponRegistry;

/// 00 §9.1 non-vanilla names plus the four extra weapons available for remapping.
pub const DEFAULT_ROSTER: [&str; 11] = [
    "usp_mp",
    "spas12_mp",
    "ranger_mp",
    "m4_mp",
    "rpg_mp",
    "mg4_mp",
    "barrett_mp",
    "cheytac_mp",
    "aa12_mp",
    "kriss_mp",
    "deserteagle_mp",
];

/// [`DEFAULT_ROSTER`] as owned names.
pub fn default_roster() -> Vec<String> {
    DEFAULT_ROSTER.iter().map(|n| (*n).to_owned()).collect()
}

/// Levenshtein distance over bytes (weapon names are ASCII).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Up to `limit` names closest to `wanted` by edit distance (ties broken alphabetically), compared
/// case-insensitively.
pub fn suggest<'a>(wanted: &str, names: impl IntoIterator<Item = &'a str>, limit: usize) -> Vec<String> {
    let wanted = wanted.to_ascii_lowercase();
    let mut scored: Vec<(usize, String)> = names
        .into_iter()
        .filter(|n| !n.is_empty())
        .map(|n| (edit_distance(&wanted, &n.to_ascii_lowercase()), n.to_owned()))
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(limit).map(|(_, n)| n).collect()
}

/// Every published weapon name (row 0, the empty "none" row, is skipped).
pub fn published_names(registry: &WeaponRegistry) -> Vec<String> {
    registry
        .published_weapons()
        .map(|b| registry.name_of(b.wire_id()).to_owned())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Resolves a requested name to a registry row, or an error message listing close matches.
pub fn resolve(registry: &WeaponRegistry, name: &str) -> Result<u32, String> {
    match registry.resolve_index(name) {
        Ok(Some(index)) if index != 0 => Ok(index),
        _ => {
            let names = published_names(registry);
            let close = suggest(name, names.iter().map(String::as_str), 5);
            Err(format!(
                "unknown weapon '{name}'; closest published names: {}",
                close.join(", ")
            ))
        }
    }
}
