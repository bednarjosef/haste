//! Per-instance `setupAnim`: limb swing, head aim, attack swing, pose adjustments.
//!
//! This follows vanilla's model code (26.3 client, `net/minecraft/client/model`): the walk swing
//! is `HumanoidModel.setupAnim` / `QuadrupedModel.setupAnim` (cos(animationPos * 0.6662) times
//! the animation speed), the head is `state.xRot`/`state.yRot` (the look pitch and the head yaw
//! relative to the body, `LivingEntityRenderer.extractRenderState`), the attack is
//! `HumanoidModel.setupAttackAnimation` (and `AnimationUtils.animateZombieArms` for zombies), and
//! the crouch/riding/swim branches are the same method's. The per-model functions are chosen from
//! the dumped model's part names (and texture) so the setupAnim matches the layer the recorder
//! dumped, not a hand-written type table.
//!
//! The renderer applies `EntityInstance.yaw` (the body yaw) to the whole model and these additive
//! rotations on top of each part's base rotation; a child overlay (hat, sleeve) inherits its
//! parent's rotation, exactly like vanilla's part tree.
use haste_core::entity::Pose;
use haste_core::math::Vec3;
use haste_core::model::{CubeModel, ModelPart, PoseOverrides};

use crate::store::{wrap_degrees, Entity};

// `Mth.PI`/`Mth.TWO_PI` equivalents.
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// `Mth.rotLerpRad`.
fn rot_lerp_rad(delta: f32, from: f32, to: f32) -> f32 {
    let mut diff = to - from;
    while diff < -PI {
        diff += TAU;
    }
    while diff >= PI {
        diff -= TAU;
    }
    from + delta * diff
}

/// `Mth.lerp`.
fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
}

/// `Ease.outQuart`.
fn out_quart(x: f32) -> f32 {
    let t = 1.0 - x;
    1.0 - (t * t) * (t * t)
}

/// `AnimationUtils.bobModelPart` added to a part's rotation.
fn bob(age: f32, scale: f32) -> Vec3 {
    Vec3::new(
        scale * ((age * 0.067).sin() * 0.05),
        0.0,
        scale * ((age * 0.09).cos() * 0.05 + 0.05),
    )
}

/// Which `setupAnim` a dumped model uses, recognised from its part names (the model layer is the
/// oracle, so this needs no entity-type table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelKind {
    Humanoid,
    Zombie,
    Skeleton,
    Quadruped,
    Spider,
    Chicken,
    None,
}

fn part_exists(p: &ModelPart, name: &str) -> bool {
    p.name == name || p.children.iter().any(|c| part_exists(c, name))
}

/// Classifies a model from its parts (a short, allocation-free tree walk: the dumped layers have
/// a handful of parts).
fn model_kind(model: &CubeModel) -> ModelKind {
    if part_exists(&model.root, "right_middle_front_leg") {
        ModelKind::Spider
    } else if part_exists(&model.root, "right_wing") {
        ModelKind::Chicken
    } else if part_exists(&model.root, "right_hind_leg") {
        // QuadrupedModel and CreeperModel share the same leg/head setupAnim.
        ModelKind::Quadruped
    } else if part_exists(&model.root, "right_arm") {
        // HumanoidModel subclasses: the player/humanoid texture, zombies and skeletons (the
        // texture the registry set tells them apart). Other arm models (villager, illager) use
        // their own setupAnim and are left on the head aim only.
        let texture = model.texture.path();
        if texture.contains("humanoid") || texture.contains("enderman") || texture.contains("piglin")
        {
            ModelKind::Humanoid
        } else if texture.contains("zombie") {
            ModelKind::Zombie
        } else if texture.contains("skeleton") {
            ModelKind::Skeleton
        } else {
            ModelKind::None
        }
    } else {
        ModelKind::None
    }
}

/// The `setupAnim` for one entity: additive part rotations by name (the renderer adds them to the
/// model's base pose). `has_head` is whether the model has a "head" part; a model with no
/// recognised setupAnim still gets the head aim if it has one. Allocates only the overrides the
/// caller's `EntityInstance` already owns.
pub(crate) fn setup_anim(
    e: &Entity,
    model: &CubeModel,
    has_head: bool,
    partial: f32,
    is_passenger: bool,
) -> PoseOverrides {
    let mut o = PoseOverrides::default();
    let body_yaw = e.body_yaw_at(partial);
    let head_rel = wrap_degrees(e.head_yaw_at(partial) - body_yaw);
    let walk_pos = e.walk.position(partial);
    let walk_speed = e.walk.speed(partial);
    let swing = e.swing.animation(partial);
    let age = e.tick_count as f32 + partial;

    match model_kind(model) {
        ModelKind::Humanoid => {
            humanoid(&mut o, e, partial, walk_pos, walk_speed, swing, age, is_passenger, false);
            push_head(&mut o, e, head_rel, partial, true);
        }
        ModelKind::Zombie => {
            humanoid(&mut o, e, partial, walk_pos, walk_speed, swing, age, is_passenger, true);
            push_head(&mut o, e, head_rel, partial, true);
        }
        ModelKind::Skeleton => {
            humanoid(&mut o, e, partial, walk_pos, walk_speed, swing, age, is_passenger, false);
            push_head(&mut o, e, head_rel, partial, true);
            // `SkeletonModel.setupAnim`'s aggressive bow-ready arms need `isAggressive`, which is
            // `Mob.getTarget() != null` on the server and is not packet-synced, so the arms stay
            // at the humanoid walk swing.
        }
        ModelKind::Quadruped => {
            quadruped(&mut o, walk_pos, walk_speed);
            push_head(&mut o, e, head_rel, partial, false);
        }
        ModelKind::Spider => {
            spider(&mut o, walk_pos, walk_speed);
            push_head(&mut o, e, head_rel, partial, false);
        }
        ModelKind::Chicken => chicken(&mut o, e, partial, walk_pos, walk_speed),
        ModelKind::None => {
            if has_head {
                push_head(&mut o, e, head_rel, partial, false);
            }
        }
    }
    o
}

/// The head aim: `state.xRot` (look pitch, degrees) and `state.yRot` (head yaw relative to the
/// body). `humanoid` adds the fall-flying/swimming head tip (`HumanoidModel.setupAnim`).
fn push_head(o: &mut PoseOverrides, e: &Entity, head_rel: f32, partial: f32, humanoid: bool) {
    let mut head = Vec3::new(e.pitch.to_radians(), head_rel.to_radians(), 0.0);
    if humanoid {
        head.x = head_x_rot(e, head.x, partial);
    }
    o.rotations.push(("head".into(), head));
}

/// `HumanoidModel.setupAnim`'s head pitch: fall flying and swimming tip the head down.
fn head_x_rot(e: &Entity, mut x: f32, partial: f32) -> f32 {
    if e.pose == Pose::FallFlying {
        x = -FRAC_PI_2 / 2.0;
    }
    let swim = lerp(partial, e.swim_amount_o, e.swim_amount);
    if swim > 0.0 && e.pose != Pose::FallFlying {
        x = rot_lerp_rad(swim, x, -FRAC_PI_2 / 2.0);
    }
    x
}

/// `HumanoidModel.setupAnim` (with `ZombieModel`'s `animateZombieArms` when `zombie`).
#[allow(clippy::too_many_arguments)]
fn humanoid(
    o: &mut PoseOverrides,
    e: &Entity,
    partial: f32,
    walk_pos: f32,
    walk_speed: f32,
    swing: f32,
    age: f32,
    is_passenger: bool,
    zombie: bool,
) {
    let pitch = e.pitch.to_radians();
    // Walk swing (`cos(animationPos * 0.6662) * 2 * speed * 0.5` for arms, `* 1.4` for legs).
    let mut ra = Vec3::new((walk_pos * 0.6662 + PI).cos() * walk_speed, 0.0, 0.0);
    let mut la = Vec3::new((walk_pos * 0.6662).cos() * walk_speed, 0.0, 0.0);
    let mut rl = Vec3::new(
        (walk_pos * 0.6662).cos() * 1.4 * walk_speed,
        0.005,
        0.005,
    );
    let mut ll = Vec3::new(
        (walk_pos * 0.6662 + PI).cos() * 1.4 * walk_speed,
        -0.005,
        -0.005,
    );
    let mut body = Vec3::ZERO;
    if is_passenger {
        ra.x += -PI / 5.0;
        la.x += -PI / 5.0;
        rl = Vec3::new(-1.4137167, PI / 10.0, 0.07853982);
        ll = Vec3::new(-1.4137167, -PI / 10.0, -0.07853982);
    }
    // `HumanoidModel.setupAttackAnimation` (main arm right, WHACK animation).
    if swing > 0.0 {
        let body_y_rot = (swing.sqrt() * TAU).sin() * 0.2;
        body.y = body_y_rot;
        ra.y += body_y_rot;
        la.y += body_y_rot;
        la.x += body_y_rot;
        let aa = (out_quart(swing) * PI).sin();
        let bb = (swing * PI).sin() * -(pitch - 0.7) * 0.75;
        ra.x -= aa * 1.2 + bb;
        ra.y += body_y_rot * 2.0;
        ra.z += (swing * PI).sin() * -0.4;
    }
    if zombie {
        // `AnimationUtils.animateZombieArms`: arms forward, the drop shallower when aggressive
        // (`isAggressive` is server-only state, so the adult non-aggressive drop is used).
        let arm_drop = -PI / 2.25;
        let attack_y = (swing * PI).sin();
        let attack_x = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).sin();
        let x = arm_drop + attack_y * 1.2 - attack_x * 0.4;
        let y = 0.1 - attack_y * 0.6;
        ra = Vec3::new(x, -y, 0.0);
        la = Vec3::new(x, y, 0.0);
    }
    if e.metadata.shared.crouching {
        body.x = 0.5;
        ra.x += 0.4;
        la.x += 0.4;
    }
    ra += bob(age, 1.0);
    la += bob(age, -1.0);
    // `HumanoidModel.setupSwimAnimation` runs last, overriding the arms and legs while swimming.
    let swim = lerp(partial, e.swim_amount_o, e.swim_amount);
    if swim > 0.0 {
        setup_swim(&mut ra, &mut la, &mut rl, &mut ll, walk_pos, swim, e);
    }
    o.rotations.push(("right_arm".into(), ra));
    o.rotations.push(("left_arm".into(), la));
    o.rotations.push(("right_leg".into(), rl));
    o.rotations.push(("left_leg".into(), ll));
    if body != Vec3::ZERO {
        o.rotations.push(("body".into(), body));
    }
}

/// `HumanoidModel.setupSwimAnimation`: the crawl stroke. `swim` is the smoothed swim amount; the
/// arm of the swinging hand keeps its pose (`state.currentSwing`).
fn setup_swim(
    ra: &mut Vec3,
    la: &mut Vec3,
    rl: &mut Vec3,
    ll: &mut Vec3,
    walk_pos: f32,
    swim: f32,
    e: &Entity,
) {
    let quadratic = |x: f32| -65.0 * x + x * x;
    let swim_pos = walk_pos % 26.0;
    // Main hand is right: hand 0 = right, 1 = left. No swing means both arms swim.
    let (right_amount, left_amount) = if e.swing.is_swinging() {
        if e.swing.hand() == 0 {
            (0.0, swim)
        } else {
            (swim, 0.0)
        }
    } else {
        (swim, swim)
    };
    if swim_pos < 14.0 {
        la.x = rot_lerp_rad(left_amount, la.x, 0.0);
        ra.x = lerp(right_amount, ra.x, 0.0);
        la.y = rot_lerp_rad(left_amount, la.y, PI);
        ra.y = lerp(right_amount, ra.y, PI);
        la.z = rot_lerp_rad(
            left_amount,
            la.z,
            PI + 1.8707964 * quadratic(swim_pos) / quadratic(14.0),
        );
        ra.z = lerp(
            right_amount,
            ra.z,
            PI - 1.8707964 * quadratic(swim_pos) / quadratic(14.0),
        );
    } else if swim_pos < 22.0 {
        let p = (swim_pos - 14.0) / 8.0;
        la.x = rot_lerp_rad(left_amount, la.x, FRAC_PI_2 * p);
        ra.x = lerp(right_amount, ra.x, FRAC_PI_2 * p);
        la.y = rot_lerp_rad(left_amount, la.y, PI);
        ra.y = lerp(right_amount, ra.y, PI);
        la.z = rot_lerp_rad(left_amount, la.z, 5.012389 - 1.8707964 * p);
        ra.z = lerp(right_amount, ra.z, 1.2707963 + 1.8707964 * p);
    } else if swim_pos < 26.0 {
        let p = (swim_pos - 22.0) / 4.0;
        la.x = rot_lerp_rad(left_amount, la.x, FRAC_PI_2 - FRAC_PI_2 * p);
        ra.x = lerp(right_amount, ra.x, FRAC_PI_2 - FRAC_PI_2 * p);
        la.y = rot_lerp_rad(left_amount, la.y, PI);
        ra.y = lerp(right_amount, ra.y, PI);
        la.z = rot_lerp_rad(left_amount, la.z, PI);
        ra.z = lerp(right_amount, ra.z, PI);
    }
    ll.x = lerp(swim, ll.x, 0.3 * (walk_pos * 0.33333334 + PI).cos());
    rl.x = lerp(swim, rl.x, 0.3 * (walk_pos * 0.33333334).cos());
}

/// `QuadrupedModel.setupAnim` (also `CreeperModel.setupAnim`).
fn quadruped(o: &mut PoseOverrides, walk_pos: f32, walk_speed: f32) {
    let c = (walk_pos * 0.6662).cos() * 1.4 * walk_speed;
    let c2 = (walk_pos * 0.6662 + PI).cos() * 1.4 * walk_speed;
    o.rotations.push(("right_hind_leg".into(), Vec3::new(c, 0.0, 0.0)));
    o.rotations
        .push(("left_hind_leg".into(), Vec3::new(c2, 0.0, 0.0)));
    o.rotations
        .push(("right_front_leg".into(), Vec3::new(c2, 0.0, 0.0)));
    o.rotations
        .push(("left_front_leg".into(), Vec3::new(c, 0.0, 0.0)));
}

/// `SpiderModel.setupAnim`.
fn spider(o: &mut PoseOverrides, walk_pos: f32, walk_speed: f32) {
    let ap = walk_pos * 0.6662;
    let speed = walk_speed;
    let swing_hind = -((ap * 2.0).cos() * 0.4) * speed;
    let swing_mid_hind = -((ap * 2.0 + PI).cos() * 0.4) * speed;
    let swing_mid_front = -((ap * 2.0 + FRAC_PI_2).cos() * 0.4) * speed;
    let swing_front = -((ap * 2.0 + PI * 3.0 / 2.0).cos() * 0.4) * speed;
    let step_hind = (ap.sin() * 0.4).abs() * speed;
    let step_mid_hind = ((ap + PI).sin() * 0.4).abs() * speed;
    let step_mid_front = ((ap + FRAC_PI_2).sin() * 0.4).abs() * speed;
    let step_front = ((ap + PI * 3.0 / 2.0).sin() * 0.4).abs() * speed;
    let leg = |swing: f32, step: f32| Vec3::new(0.0, swing, step);
    o.rotations
        .push(("right_hind_leg".into(), leg(swing_hind, step_hind)));
    o.rotations
        .push(("left_hind_leg".into(), leg(-swing_hind, -step_hind)));
    o.rotations
        .push(("right_middle_hind_leg".into(), leg(swing_mid_hind, step_mid_hind)));
    o.rotations
        .push(("left_middle_hind_leg".into(), leg(-swing_mid_hind, -step_mid_hind)));
    o.rotations.push((
        "right_middle_front_leg".into(),
        leg(swing_mid_front, step_mid_front),
    ));
    o.rotations.push((
        "left_middle_front_leg".into(),
        leg(-swing_mid_front, -step_mid_front),
    ));
    o.rotations
        .push(("right_front_leg".into(), leg(swing_front, step_front)));
    o.rotations
        .push(("left_front_leg".into(), leg(-swing_front, -step_front)));
}

/// `ChickenModel.setupAnim` (legs walk, wings flap).
fn chicken(o: &mut PoseOverrides, e: &Entity, partial: f32, walk_pos: f32, walk_speed: f32) {
    let flap = lerp(partial, e.o_flap, e.flap);
    let flap_speed = lerp(partial, e.o_flap_speed, e.flap_speed);
    let flap_angle = (flap.sin() + 1.0) * flap_speed;
    o.rotations.push((
        "right_leg".into(),
        Vec3::new((walk_pos * 0.6662).cos() * 1.4 * walk_speed, 0.0, 0.0),
    ));
    o.rotations.push((
        "left_leg".into(),
        Vec3::new(
            (walk_pos * 0.6662 + PI).cos() * 1.4 * walk_speed,
            0.0,
            0.0,
        ),
    ));
    o.rotations
        .push(("right_wing".into(), Vec3::new(0.0, 0.0, flap_angle)));
    o.rotations
        .push(("left_wing".into(), Vec3::new(0.0, 0.0, -flap_angle)));
}
