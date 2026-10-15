// SPDX-License-Identifier: GPL-3.0-or-later
//! Checks every pinned location of `support::locations` against the loaded Freedoom
//! (sheet 13 section 5.3). With `DMW_DISCOVER_LOCATIONS=1` it prints candidates instead, which
//! is how the constants were pinned.

mod support;

use std::panic::{catch_unwind, AssertUnwindSafe};

use support::locations::{self, Kind, Location};
use support::Harness;

#[test]
fn locations_headless() {
    let mut h = Harness::start_default();
    if std::env::var("DMW_DISCOVER_LOCATIONS").is_ok_and(|v| v == "1") {
        locations::discover(&mut h);
        return;
    }

    // Every pinned spot satisfies its requirement and renders a real picture.
    for loc in locations::ALL {
        loc.go(&mut h);
        let frame = h.frame();
        assert!(
            frame.distinct_indices() > 8,
            "{}: the rendered frame is nearly uniform ({} distinct colours)",
            loc.name,
            frame.distinct_indices()
        );
    }

    // The assertions fail loudly when a spot stops matching its requirement.
    locations::OPEN_ROOM.go(&mut h);
    let wrong_fingerprint = Location {
        fingerprint: (1, 1),
        ..locations::OPEN_ROOM
    };
    let wrong_kind = Location {
        kind: Kind::Dark,
        ..locations::OPEN_ROOM
    };
    let facing_wall = Location {
        angle_deg: 270.0,
        ..locations::OPEN_ROOM
    };
    // The expected panics would otherwise print three backtraces-worth of noise.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let results: Vec<_> = [
        ("fingerprint", wrong_fingerprint),
        ("sector light", wrong_kind),
        ("clear distance", facing_wall),
    ]
    .into_iter()
    .map(|(what, bad)| {
        (
            what,
            catch_unwind(AssertUnwindSafe(|| bad.assert_valid(&h))),
        )
    })
    .collect();
    std::panic::set_hook(default_hook);
    for (what, result) in results {
        assert!(result.is_err(), "a wrong {what} must fail assert_valid");
    }
}
