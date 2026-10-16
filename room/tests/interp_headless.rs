//! Uncapped frame rate (sheet 16): drawing a blended frame never changes the simulation.
//!
//! The real engine on Freedoom 1 E1M1 with interpolation allowed. The player walks forward and
//! a frame is drawn after every tic: every map object, sector height and the player's `viewz`
//! must be bit-identical before and after `D_Display`, and the walking player must have been
//! drawn blended. One `#[test]`, one engine.

mod support;

use room::doom::c_ffi::sector_t;
use room::doom::d_main::D_Display;
use room::doom::doomkeys::KEY_UPARROW;
use support::Harness;

type World = (Vec<(i32, f32, f32, i32)>, Vec<(i32, i32)>, i32, u32);

fn world(h: &Harness) -> World {
    // SAFETY: engine thread, level loaded; `sectors` holds `numsectors` entries.
    let secs = unsafe {
        let n = usize::try_from(room::doom::p_setup::numsectors).unwrap_or(0);
        std::slice::from_raw_parts(room::doom::p_setup::sectors as *const sector_t, n)
            .iter()
            .map(|s| (s.floorheight, s.ceilingheight))
            .collect()
    };
    (h.mobjs(), secs, h.player().viewz, h.mobj().angle)
}

#[test]
fn blended_frames_leave_the_world_untouched() {
    let mut h = Harness::start_default();
    room::dmw::interp::set_allowed(true);
    assert!(room::dmw::interp::active());

    let mut blended_frames = 0;
    h.key(KEY_UPARROW, true);
    for tic in 0..40 {
        h.run_tics(1);
        let before = world(&h);
        D_Display();
        let after = world(&h);
        assert_eq!(before, after, "tic {tic}: drawing a frame changed the world");
        if room::dmw::interp::last_blended().0 > 0 {
            blended_frames += 1;
        }
    }
    h.key(KEY_UPARROW, false);
    assert!(blended_frames > 30, "only {blended_frames} of 40 frames were blended");

    room::dmw::interp::set_allowed(false);
    h.run_tics(1);
    D_Display();
    assert_eq!(room::dmw::interp::last_blended(), (0, 0), "off means no blending");
}
