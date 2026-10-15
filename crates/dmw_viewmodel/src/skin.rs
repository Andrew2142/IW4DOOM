// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU linear-blend skinning (sheet 11 section 5.4).

use glam::{Mat4, Vec3};

use crate::prepared::PreparedViewmodel;

#[inline]
fn bone(skin: &[Mat4], b: u16) -> &Mat4 {
    skin.get(usize::from(b)).unwrap_or(&Mat4::IDENTITY)
}

/// Skins every vertex of `model` with `skin` into `out_p` / `out_n` (resized to the vertex
/// count; no allocation once they have reached that size).
///
/// `p = Σ w·(M·p)`, `n = normalize(Σ w·(mat3(M)·n))`. Single-influence vertices take one matrix
/// multiply. Bone indices beyond `skin.len()` use the identity.
pub(crate) fn skin_vertices(
    model: &PreparedViewmodel,
    skin: &[Mat4],
    out_p: &mut Vec<Vec3>,
    out_n: &mut Vec<Vec3>,
) {
    let n = model.positions.len();
    out_p.resize(n, Vec3::ZERO);
    out_n.resize(n, Vec3::ZERO);
    let iter = model
        .positions
        .iter()
        .zip(&model.normals)
        .zip(&model.skin)
        .zip(out_p.iter_mut().zip(out_n.iter_mut()));
    for (((p, nrm), s), (op, on)) in iter {
        let p = Vec3::from(*p);
        let nrm = Vec3::from(*nrm);
        if s.count == 1 {
            let m = bone(skin, s.bones[0]);
            *op = m.transform_point3(p);
            *on = m.transform_vector3(nrm).normalize_or_zero();
        } else {
            let mut ap = Vec3::ZERO;
            let mut an = Vec3::ZERO;
            for k in 0..usize::from(s.count) {
                let m = bone(skin, s.bones[k]);
                let w = s.weights[k];
                ap += w * m.transform_point3(p);
                an += w * m.transform_vector3(nrm);
            }
            *op = ap;
            *on = an.normalize_or_zero();
        }
    }
}
