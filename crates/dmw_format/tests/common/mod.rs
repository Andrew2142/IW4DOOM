// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared helpers for the integration tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use dmw_format::{DmwWeaponId, Pack, ViewmodelFile};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh directory under the system temp dir, removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dmw_format_{}_{tag}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every file below `root` as (relative path with `/`, bytes), sorted by path.
pub fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, root, out);
            } else {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

/// The in-memory fixture's viewmodel for `name`, as the file form.
pub fn fixture_file(name: &str) -> ViewmodelFile {
    let pack = Pack::fixture();
    let id = pack.weapon_id(name).unwrap();
    let vm = pack.load_viewmodel(id).unwrap();
    ViewmodelFile {
        skeleton: vm.skeleton.clone(),
        mesh: vm.mesh.clone(),
        clips: vm.clips.clone(),
    }
}

pub fn all_ids(pack: &Pack) -> Vec<DmwWeaponId> {
    pack.weapon_ids().collect()
}
