// `Camera.alignWithEntity`'s minecart branch: the camera sits at the cart's walk plus the
// seat offset plus the camera's own smoothed eye height, whatever the straight line between
// the tick's feet positions says.

use glam::DVec3;
use haste_core::EntityId;

use super::{riding_eye, CartCamera};

#[test]
fn the_riding_eye_follows_the_walk_with_the_smoothed_eye_height() {
    // Our feet moved one block east over the tick while the eye height grew from 1.5 to 1.62
    // (the camera's `eyeHeightOld`/`eyeHeight`).
    let c = CartCamera {
        cart: EntityId(5),
        cart_tick: 40,
        prev_feet: DVec3::new(0.0, 64.0, 0.0),
        feet: DVec3::new(1.0, 64.0, 0.0),
        offset: DVec3::new(0.0, -0.3, 0.0),
    };
    let prev_eye = c.prev_feet + DVec3::new(0.0, 1.5, 0.0);
    let eye = c.feet + DVec3::new(0.0, 1.62, 0.0);
    // Half way through the frame the cart's walk is round a corner, off the straight line.
    let walk = DVec3::new(0.7, 64.2, 0.1);
    let linear = prev_eye.lerp(eye, 0.5);
    let got = riding_eye(linear, walk, &c, 0.5);
    let want = walk + c.offset + DVec3::new(0.0, 1.56, 0.0);
    assert!((got - want).length() < 1e-9, "{got:?} is not {want:?}");
    // At the tick's end with the walk at our cart's position, it is the plain eye.
    let cart_pos = c.feet - c.offset;
    let got = riding_eye(eye, cart_pos, &c, 1.0);
    assert!((got - eye).length() < 1e-9, "{got:?} is not {eye:?}");
}
