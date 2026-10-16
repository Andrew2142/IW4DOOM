// SPDX-License-Identifier: GPL-3.0-or-later
//! Frames-per-second counter: counts the frames that reach the view-overlay hook in
//! `D_Display` and draws the rate in the top-right corner of the view window.
//!
//! Off by default (`[hud] show_fps`); the window layer toggles it with the backquote key. It
//! draws nothing while off, so demos and vanilla parity are unaffected.

use std::cell::Cell;
use std::time::{Duration, Instant};

use super::with_state;

/// How often the shown number is refreshed.
const WINDOW: Duration = Duration::from_millis(500);

thread_local! {
    /// `Some` once the player toggled the counter; `None` follows `[hud] show_fps`.
    static OVERRIDE: Cell<Option<bool>> = const { Cell::new(None) };
    static COUNTER: Cell<Counter> = const { Cell::new(Counter::new()) };
}

/// Frame count over the current measuring window and the last finished rate.
#[derive(Clone, Copy, Debug)]
pub struct Counter {
    start: Option<Instant>,
    frames: u32,
    shown: Option<f32>,
}

impl Counter {
    pub const fn new() -> Self {
        Self {
            start: None,
            frames: 0,
            shown: None,
        }
    }

    /// Counts one frame at `now`; returns the rate to show (`None` until the first window ends).
    pub fn frame(&mut self, now: Instant) -> Option<f32> {
        // The first frame only opens the window; frames after it are counted against it.
        let Some(start) = self.start else {
            self.start = Some(now);
            return self.shown;
        };
        self.frames += 1;
        let elapsed = now.duration_since(start);
        if elapsed >= WINDOW {
            self.shown = Some(self.frames as f32 / elapsed.as_secs_f32());
            self.start = Some(now);
            self.frames = 0;
        }
        self.shown
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the counter is showing.
pub fn enabled() -> bool {
    OVERRIDE
        .with(Cell::get)
        .unwrap_or_else(|| with_state(|s| s.config.hud.show_fps).unwrap_or(false))
}

/// Flips the counter on or off (the backquote key).
pub fn toggle() {
    let on = !enabled();
    OVERRIDE.with(|o| o.set(Some(on)));
    COUNTER.with(|c| c.set(Counter::new()));
    log::info!(target: "dmw::hud", "fps counter {}", if on { "on" } else { "off" });
}

/// Called once per displayed frame from the view-overlay hook.
pub(crate) fn on_frame() {
    if !enabled() {
        return;
    }
    let shown = COUNTER.with(|c| {
        let mut counter = c.get();
        let shown = counter.frame(Instant::now());
        c.set(counter);
        shown
    });
    if let Some(fps) = shown {
        super::hud::draw_corner_text(&format!("{:.0} FPS", fps));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_appears_after_the_first_window() {
        let t0 = Instant::now();
        let mut c = Counter::new();
        // 35 frames over one second, one every 1/35 s.
        let mut last = None;
        for i in 0..=36u32 {
            last = c.frame(t0 + Duration::from_secs_f32(i as f32 / 35.0));
            if i < 17 {
                assert_eq!(last, None, "frame {i}");
            }
        }
        let fps = last.expect("a rate after one second");
        assert!((fps - 35.0).abs() < 1.5, "{fps}");
    }
}
