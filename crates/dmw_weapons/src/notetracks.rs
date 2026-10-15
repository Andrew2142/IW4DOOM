// SPDX-License-Identifier: GPL-3.0-or-later
//! Notetrack → `SoundCue` (sheet 10 §5.7).

use std::collections::BTreeMap;

use crate::clip_scheduler::ClipNotification;
use crate::events::SoundCue;

/// Notetrack name (lower case) → sound alias, built once per weapon from
/// `WeaponSounds::notetracks`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotetrackSounds(BTreeMap<String, String>);

impl NotetrackSounds {
    /// Lower-cases the names so lookups are case-insensitive. Empty aliases are dropped.
    pub fn new(map: &BTreeMap<String, String>) -> Self {
        Self(
            map.iter()
                .filter(|(_, alias)| !alias.is_empty())
                .map(|(name, alias)| (name.to_ascii_lowercase(), alias.clone()))
                .collect(),
        )
    }

    /// The alias mapped to `name`, compared case-insensitively.
    pub fn alias(&self, name: &str) -> Option<&str> {
        self.0.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    /// Cues for the notifications of one advance, which arrive in clip-time order. Unmapped
    /// notetracks are skipped.
    pub fn to_cues(&self, notifications: &[ClipNotification]) -> Vec<SoundCue> {
        notifications
            .iter()
            .filter_map(|n| match self.alias(&n.name) {
                Some(alias) => Some(SoundCue {
                    alias: alias.to_owned(),
                    local: true,
                }),
                None => {
                    log::trace!(target: "dmw::anim", "notetrack {} has no sound", n.name);
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_and_unmapped_ignored() {
        let map = BTreeMap::from([("Mag_Out".to_owned(), "snd_out".to_owned())]);
        let s = NotetrackSounds::new(&map);
        let n = |name: &str| ClipNotification {
            playback: 1,
            node: 9,
            cycle: 0,
            marker: 0,
            name: name.into(),
        };
        let cues = s.to_cues(&[n("MAG_OUT"), n("bolt"), n("mag_out")]);
        assert_eq!(cues.len(), 2);
        assert!(cues.iter().all(|c| c.alias == "snd_out" && c.local));
    }
}
