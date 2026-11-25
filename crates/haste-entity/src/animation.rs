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
use haste_core::model::{BodyOp, BodyTransform, CubeModel, ModelPart, PoseOverrides};

pub(crate) mod arm_pose;
mod crouch;
mod swim;

pub use arm_pose::{poses, set_item_uses, ArmPose, ItemUse, ItemUses, UseAnimation};
use haste_core::parts;
use swim::setup_swim;

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

/// Replace (or insert) a part's additive rotation, for the models that run after the humanoid
/// walk swing and override it (`SkeletonModel.setupAnim`).
fn set_rotation(o: &mut PoseOverrides, id: haste_core::parts::PartId, rot: Vec3) {
    o.set_rotation(id, rot);
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
    Villager,
    Illager,
    IronGolem,
    None,
}

fn part_exists(p: &ModelPart, id: parts::PartId) -> bool {
    p.id == id || p.children.iter().any(|c| part_exists(c, id))
}

/// Classifies a model from its parts (a short, allocation-free tree walk: the dumped layers have
/// a handful of parts).
fn model_kind(model: &CubeModel) -> ModelKind {
    if part_exists(&model.root, parts::RIGHT_MIDDLE_FRONT_LEG) {
        ModelKind::Spider
    } else if part_exists(&model.root, parts::RIGHT_WING) {
        ModelKind::Chicken
    } else if part_exists(&model.root, parts::RIGHT_HIND_LEG) {
        // QuadrupedModel and CreeperModel share the same leg/head setupAnim.
        ModelKind::Quadruped
    } else if part_exists(&model.root, parts::ARMS) && !part_exists(&model.root, parts::RIGHT_ARM) {
        // VillagerModel: a single "arms" part (crossed arms) and no separate arms.
        ModelKind::Villager
    } else if part_exists(&model.root, parts::RIGHT_ARM) {
        // HumanoidModel subclasses plus the arm models with their own setupAnim, told apart by
        // the texture the registry set.
        let texture = model.texture.path();
        if texture.contains("humanoid")
            || texture.contains("enderman")
            || texture.contains("piglin")
        {
            ModelKind::Humanoid
        } else if texture.contains("zombie") {
            ModelKind::Zombie
        } else if texture.contains("skeleton") {
            ModelKind::Skeleton
        } else if texture.contains("illager") {
            ModelKind::Illager
        } else if texture.contains("iron_golem") {
            ModelKind::IronGolem
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
pub fn setup_anim(
    e: &Entity,
    model: &CubeModel,
    has_head: bool,
    partial: f32,
    is_passenger: bool,
) -> PoseOverrides {
    let mut o = PoseOverrides::default();
    let body_yaw = e.body_yaw_at(partial);
    let mut head_rel = wrap_degrees(e.head_yaw_at(partial) - body_yaw);
    // `LivingEntityRenderer.extractRenderState`: an upside-down entity (Dinnerbone/Grumm) has its
    // look pitch and head yaw negated before any model setup runs.
    let upside = is_upside_down(e);
    let mut pitch = e.pitch_at(partial);
    if upside {
        pitch = -pitch;
        head_rel = -head_rel;
    }
    let pitch = pitch.to_radians();
    let walk_pos = e.walk.position(partial);
    let walk_speed = e.walk.speed(partial);
    let swing = e.swing.animation(partial);
    let age = e.tick_count as f32 + partial;
    // The head's yaw relative to the body in radians: the arm poses read it
    // (`HumanoidModel.setupAnim`'s `this.head.yRot`).
    let head_rel_rad = head_rel.to_radians();

    match model_kind(model) {
        ModelKind::Humanoid => {
            humanoid(
                &mut o,
                e,
                partial,
                walk_pos,
                walk_speed,
                swing,
                age,
                is_passenger,
                false,
                arm_pose::poses(e, arm_pose::item_uses()),
                head_rel_rad,
            );
            push_head(&mut o, e, head_rel, partial, true);
        }
        ModelKind::Zombie => {
            humanoid(
                &mut o,
                e,
                partial,
                walk_pos,
                walk_speed,
                swing,
                age,
                is_passenger,
                true,
                // `ZombieModel.setupAttackAnimation`'s `animateZombieArms` overrides
                // whatever the pose set, so the zombie branch never poses the arms.
                (ArmPose::Empty, ArmPose::Empty),
                head_rel_rad,
            );
            push_head(&mut o, e, head_rel, partial, true);
        }
        ModelKind::Skeleton => {
            humanoid(
                &mut o,
                e,
                partial,
                walk_pos,
                walk_speed,
                swing,
                age,
                is_passenger,
                false,
                // `AbstractSkeletonRenderer.getArmPose` (the bow hold) is not ported yet;
                // an empty hand hangs (vanilla's mob rule), a bow hand is not raised.
                (ArmPose::Empty, ArmPose::Empty),
                head_rel_rad,
            );
            push_head(&mut o, e, head_rel, partial, true);
            // `SkeletonModel.setupAnim` (26.3 client): an aggressive skeleton not holding a bow
            // raises both arms straight ahead, overriding the humanoid walk swing. The
            // `!isHoldingBow` half needs the main-hand item, which this crate does not resolve
            // yet; without it the aggressive arms are used (the common case).
            if e.metadata.mob.aggressive {
                skeleton_aggressive_arms(&mut o, swing, age);
            }
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
        ModelKind::Villager => villager(&mut o, e, pitch, head_rel, walk_pos, walk_speed, age),
        ModelKind::Illager => illager(
            &mut o,
            e,
            pitch,
            head_rel,
            walk_pos,
            walk_speed,
            swing,
            age,
            is_passenger,
            model.texture.path(),
        ),
        ModelKind::IronGolem => {
            iron_golem(&mut o, e, pitch, head_rel, partial, walk_pos, walk_speed)
        }
        ModelKind::None => {
            if has_head {
                push_head(&mut o, e, head_rel, partial, false);
            }
        }
    }
    o
}

/// `LivingEntityRenderer.isEntityUpsideDown` / `AvatarRenderer.isEntityUpsideDown`: a mob named
/// `Dinnerbone` or `Grumm` (custom name) is drawn upside down; a player uses its profile name,
/// and only while its cape part is shown.
fn is_upside_down(e: &Entity) -> bool {
    if e.is_player_type {
        crate::avatar::ModelParts::of(&e.metadata).shown(crate::avatar::PlayerModelPart::Cape)
            && e.nametag.as_deref().is_some_and(is_upside_down_name)
    } else {
        e.metadata
            .custom_name_plain()
            .is_some_and(is_upside_down_name)
    }
}

/// `LivingEntityRenderer.isUpsideDownName`.
fn is_upside_down_name(name: &str) -> bool {
    name == "Dinnerbone" || name == "Grumm"
}

/// `LivingEntityRenderer.extractRenderState`: `deathTime = entity.deathTime + partialTicks`
/// (`entity.deathTime` is 0 until the first death tick, then counts up; here it is derived from
/// the 20-tick death counter).
fn death_time(e: &Entity, partial: f32) -> f32 {
    if !e.dead {
        return 0.0;
    }
    let ticks = crate::store::DEATH_TICKS.saturating_sub(e.hurt_ticks) as f32;
    if ticks > 0.0 {
        ticks + partial
    } else {
        0.0
    }
}

/// `AvatarRenderState.fallFlyingScale`: `clamp(fallFlyingTimeInTicks^2 / 100, 0, 1)`.
fn fall_flying_scale(ticks: u32) -> f32 {
    let t = ticks as f32;
    (t * t / 100.0).clamp(0.0, 1.0)
}

/// `LivingEntityRenderer.setupRotations` plus `AvatarRenderer.setupRotations` (players only):
/// the whole-model rotations the renderer applies between the body yaw and the model scale
/// (death tilt, sleeping, upside-down, and the player flying/swimming pitch).
pub fn body_transform(e: &Entity, partial: f32, iron_golem: bool) -> BodyTransform {
    let mut b = BodyTransform::default();
    let age = e.tick_count as f32 + partial;
    // `extractRenderState`: the look pitch interpolated over the tick, negated when upside down.
    let x_rot = e.pitch_at(partial) * (if is_upside_down(e) { -1.0 } else { 1.0 });

    let death = death_time(e, partial);
    if death > 0.0 {
        // `fall = (deathTime - 1) / 20 * 1.6`, `sqrt`, clamp 1, rotate Z by `fall * 90`.
        let mut fall = (death - 1.0) / 20.0 * 1.6;
        fall = fall.sqrt().min(1.0);
        b.push(BodyOp::RotateZ((fall * 90.0).to_radians()));
    } else if e.metadata.living.auto_spin_attack {
        b.push(BodyOp::RotateX((-90.0 - x_rot).to_radians()));
        b.push(BodyOp::RotateY((age * -75.0).to_radians()));
    } else if e.pose == Pose::Sleeping {
        // The bed orientation is not tracked: vanilla falls back to `bodyRot` when it is null and
        // skips the bed head offset, which is what this branch does.
        b.suppress_yaw = true;
        b.push(BodyOp::RotateY(e.body_yaw_at(partial).to_radians()));
        b.push(BodyOp::RotateZ(90.0f32.to_radians()));
        b.push(BodyOp::RotateY(270.0f32.to_radians()));
    } else if is_upside_down(e) {
        // `translate(0, (boundingBoxHeight + 0.1) / scale, 0)`, then rotate Z 180.
        b.push(BodyOp::Translate(Vec3::new(0.0, e.dimensions.1 + 0.1, 0.0)));
        b.push(BodyOp::RotateZ(PI));
    }

    // `IronGolemRenderer.setupRotations`: while walking, the whole golem rocks side to side on a
    // 13-tick triangle wave.
    if iron_golem {
        let speed = e.walk.speed(partial);
        // Vanilla: `if (!(state.walkAnimationSpeed < 0.01))`.
        if speed.is_nan() || speed >= 0.01 {
            let wp = e.walk.position(partial) + 6.0;
            let triangle = ((wp % 13.0 - 6.5).abs() - 3.25) / 3.25;
            b.push(BodyOp::RotateZ((6.5 * triangle).to_radians()));
        }
    }

    // `AvatarRenderer.setupRotations` (players only), applied after the base rotations above.
    if e.is_player_type {
        if e.metadata.shared.elytra_flying {
            let scale = fall_flying_scale(e.fall_flying_ticks);
            if !e.metadata.living.auto_spin_attack {
                b.push(BodyOp::RotateX((scale * (-90.0 - x_rot)).to_radians()));
            }
        } else {
            let swim = lerp(partial, e.swim_amount_o, e.swim_amount);
            if swim > 0.0 {
                let target = if e.pose == Pose::Swimming {
                    -90.0 - x_rot
                } else {
                    -90.0
                };
                b.push(BodyOp::RotateX((swim * target).to_radians()));
                if e.pose == Pose::Swimming {
                    b.push(BodyOp::Translate(Vec3::new(0.0, -1.0, 0.3)));
                }
            }
        }
    }
    b
}

/// The look pitch in radians, interpolated over the tick (`entity.getXRot(partialTicks)`,
/// negated for an upside-down entity).
fn look_pitch(e: &Entity, partial: f32) -> f32 {
    let p = e.pitch_at(partial);
    (if is_upside_down(e) { -p } else { p }).to_radians()
}

/// The head aim: `state.xRot` (look pitch, radians) and `state.yRot` (head yaw relative to the
/// body). `humanoid` adds the fall-flying/swimming head tip (`HumanoidModel.setupAnim`).
fn push_head(o: &mut PoseOverrides, e: &Entity, head_rel: f32, partial: f32, humanoid: bool) {
    let mut head = Vec3::new(look_pitch(e, partial), head_rel.to_radians(), 0.0);
    if humanoid {
        head.x = head_x_rot(e, head.x, partial);
    }
    o.set_rotation(parts::HEAD, head);
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
/// `arm` is the (right, left) arm pose the held items and use state chose
/// (`AvatarRenderer.getArmPose`); `head_rel` the head's yaw relative to the body in radians.
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
    arm: (ArmPose, ArmPose),
    head_rel: f32,
) {
    let pitch = look_pitch(e, partial);
    // Walk swing (`cos(animationPos * 0.6662) * 2 * speed * 0.5` for arms, `* 1.4` for legs).
    let mut ra = Vec3::new((walk_pos * 0.6662 + PI).cos() * walk_speed, 0.0, 0.0);
    let mut la = Vec3::new((walk_pos * 0.6662).cos() * walk_speed, 0.0, 0.0);
    let mut rl = Vec3::new((walk_pos * 0.6662).cos() * 1.4 * walk_speed, 0.005, 0.005);
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
    // `HumanoidModel.poseRightArm`/`poseLeftArm` (the zombie's `animateZombieArms` below
    // overrides the arms, like vanilla's `ZombieModel.setupAttackAnimation`).
    if !zombie {
        arm_pose::apply(e, arm, &mut ra, &mut la, (head_rel, pitch));
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
        // `AnimationUtils.animateZombieArms`: arms forward; `armDrop = -PI / (aggressive ? 1.5 :
        // 2.25)` (the adult raiseArms branch; `isAggressive` is `Mob.DATA_MOB_FLAGS_ID` bit 4).
        let arm_drop = -PI / if e.metadata.mob.aggressive { 1.5 } else { 2.25 };
        let attack_y = (swing * PI).sin();
        let attack_x = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).sin();
        let x = arm_drop + attack_y * 1.2 - attack_x * 0.4;
        let y = 0.1 - attack_y * 0.6;
        ra = Vec3::new(x, -y, 0.0);
        la = Vec3::new(x, y, 0.0);
    }
    if e.metadata.shared.crouching {
        crouch::pose(o, &mut body, &mut ra, &mut la);
    }
    ra += bob(age, 1.0);
    la += bob(age, -1.0);
    // `HumanoidModel.setupSwimAnimation` runs last, overriding the arms and legs while swimming.
    let swim = lerp(partial, e.swim_amount_o, e.swim_amount);
    if swim > 0.0 {
        setup_swim(&mut ra, &mut la, &mut rl, &mut ll, walk_pos, swim, e);
    }
    o.set_rotation(parts::RIGHT_ARM, ra);
    o.set_rotation(parts::LEFT_ARM, la);
    o.set_rotation(parts::RIGHT_LEG, rl);
    o.set_rotation(parts::LEFT_LEG, ll);
    if body != Vec3::ZERO {
        o.set_rotation(parts::BODY, body);
    }
}

/// pushed by the swing, replacing the humanoid walk swing. `swing` is `state.swingAnimation`.
fn skeleton_aggressive_arms(o: &mut PoseOverrides, swing: f32, age: f32) {
    let swing2 = (swing * PI).sin();
    let swing_x = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).sin();
    let x = -FRAC_PI_2 - swing2 * 1.2 + swing_x * 0.4;
    let mut ra = Vec3::new(x, -0.1 + swing2 * 0.6, 0.0);
    let mut la = Vec3::new(x, 0.1 - swing2 * 0.6, 0.0);
    ra += bob(age, 1.0);
    la += bob(age, -1.0);
    set_rotation(o, parts::RIGHT_ARM, ra);
    set_rotation(o, parts::LEFT_ARM, la);
}

/// `Mth.triangleWave`.
fn triangle_wave(index: f32, period: f32) -> f32 {
    ((index % period - period * 0.5).abs() - period * 0.25) / (period * 0.25)
}

/// `VillagerModel.setupAnim`: head aim, the unhappy head shake, and the legs (note the walk
/// amplitude is halved, unlike the humanoid). The crossed arms are the model's base pose.
fn villager(
    o: &mut PoseOverrides,
    e: &Entity,
    pitch: f32,
    head_rel: f32,
    walk_pos: f32,
    walk_speed: f32,
    age: f32,
) {
    let mut head = Vec3::new(pitch, head_rel.to_radians(), 0.0);
    if e.metadata.villager_unhappy {
        head.z = 0.3 * (0.45 * age).sin();
        head.x = 0.4;
    }
    o.set_rotation(parts::HEAD, head);
    o.set_rotation(
        parts::RIGHT_LEG,
        Vec3::new((walk_pos * 0.6662).cos() * 1.4 * walk_speed * 0.5, 0.0, 0.0),
    );
    o.set_rotation(
        parts::LEFT_LEG,
        Vec3::new(
            (walk_pos * 0.6662 + PI).cos() * 1.4 * walk_speed * 0.5,
            0.0,
            0.0,
        ),
    );
}

/// `AbstractIllager.IllagerArmPose`, as far as the client can tell without item resolution:
/// attacking illagers, and the crossed/neutral arms otherwise.
#[derive(Clone, Copy, PartialEq, Eq)]
enum IllagerArmPose {
    Crossed,
    Neutral,
    Attacking,
}

/// `AbstractIllager.getArmPose` for the types whose pose depends only on `isAggressive`: a
/// vindicator attacks when aggressive and crosses its arms otherwise; a pillager goes neutral
/// when calm (its crossbow poses need the main-hand item); an evoker/illusioner crosses its arms
/// (their spell/bow poses need more synced state).
fn illager_arm_pose(e: &Entity, texture: &str) -> IllagerArmPose {
    if e.metadata.mob.aggressive {
        if texture.contains("vindicator") || texture.contains("pillager") {
            IllagerArmPose::Attacking
        } else {
            IllagerArmPose::Crossed
        }
    } else if texture.contains("pillager") {
        IllagerArmPose::Neutral
    } else {
        IllagerArmPose::Crossed
    }
}

/// `IllagerModel.setupAnim`: head aim, the walk swing (arms and legs, amplitude halved like the
/// villager), the riding pose, and the arm pose (`IllagerArmPose`). The crossed "arms" part and
/// the separate arms are shown/hidden per pose.
#[allow(clippy::too_many_arguments)]
fn illager(
    o: &mut PoseOverrides,
    e: &Entity,
    pitch: f32,
    head_rel: f32,
    walk_pos: f32,
    walk_speed: f32,
    swing: f32,
    age: f32,
    is_passenger: bool,
    texture: &str,
) {
    o.set_rotation(parts::HEAD, Vec3::new(pitch, head_rel.to_radians(), 0.0));
    let mut ra = Vec3::ZERO;
    let mut la = Vec3::ZERO;
    let mut rl = Vec3::ZERO;
    let mut ll = Vec3::ZERO;
    if is_passenger {
        ra.x = -PI / 5.0;
        la.x = -PI / 5.0;
        rl = Vec3::new(-1.4137167, PI / 10.0, 0.07853982);
        ll = Vec3::new(-1.4137167, -PI / 10.0, -0.07853982);
    } else {
        ra.x = (walk_pos * 0.6662 + PI).cos() * 2.0 * walk_speed * 0.5;
        la.x = (walk_pos * 0.6662).cos() * 2.0 * walk_speed * 0.5;
        rl.x = (walk_pos * 0.6662).cos() * 1.4 * walk_speed * 0.5;
        ll.x = (walk_pos * 0.6662 + PI).cos() * 1.4 * walk_speed * 0.5;
    }
    let pose = illager_arm_pose(e, texture);
    if pose == IllagerArmPose::Attacking {
        // `AnimationUtils.animateZombieArms(..., aggressive = true)` for an empty main hand,
        // then `bobArms` (the weapon branch needs the main-hand item, which is not resolved).
        let arm_drop = -PI / 1.5;
        let attack_y = (swing * PI).sin();
        let attack_x = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).sin();
        let x = arm_drop + attack_y * 1.2 - attack_x * 0.4;
        let y = 0.1 - attack_y * 0.6;
        ra = Vec3::new(x, -y, 0.0) + bob(age, 1.0);
        la = Vec3::new(x, y, 0.0) + bob(age, -1.0);
    }
    o.set_rotation(parts::RIGHT_ARM, ra);
    o.set_rotation(parts::LEFT_ARM, la);
    o.set_rotation(parts::RIGHT_LEG, rl);
    o.set_rotation(parts::LEFT_LEG, ll);
    // `crossedArms` toggles the "arms" part against the two separate arms.
    if pose == IllagerArmPose::Crossed {
        o.hide(parts::RIGHT_ARM);
        o.hide(parts::LEFT_ARM);
    } else {
        o.hide(parts::ARMS);
    }
}

/// `IronGolemModel.setupAnim`: the arms swing on a triangle wave (or the 10-tick attack
/// animation), the legs on the same wave, and the head aims.
fn iron_golem(
    o: &mut PoseOverrides,
    e: &Entity,
    pitch: f32,
    head_rel: f32,
    partial: f32,
    walk_pos: f32,
    walk_speed: f32,
) {
    // `IronGolemRenderer.extractRenderState`: attackTicksRemaining = tick > 0 ? tick - partial : 0.
    let attack = if e.attack_ticks > 0 {
        e.attack_ticks as f32 - partial
    } else {
        0.0
    };
    let (ra, la) = if attack > 0.0 {
        let t = triangle_wave(attack, 10.0);
        (-2.0 + 1.5 * t, -2.0 + 1.5 * t)
    } else {
        let t = triangle_wave(walk_pos, 13.0);
        ((-0.2 + 1.5 * t) * walk_speed, (-0.2 - 1.5 * t) * walk_speed)
    };
    o.set_rotation(parts::RIGHT_ARM, Vec3::new(ra, 0.0, 0.0));
    o.set_rotation(parts::LEFT_ARM, Vec3::new(la, 0.0, 0.0));
    o.set_rotation(parts::HEAD, Vec3::new(pitch, head_rel.to_radians(), 0.0));
    let t = triangle_wave(walk_pos, 13.0) * walk_speed;
    o.set_rotation(parts::RIGHT_LEG, Vec3::new(-1.5 * t, 0.0, 0.0));
    o.set_rotation(parts::LEFT_LEG, Vec3::new(1.5 * t, 0.0, 0.0));
}

/// `CreeperRenderer.scale`: the swell-driven wobble and the cubed swell, as a per-axis scale
/// (x/z on `s`, y on `hs`). `swelling = lerp(partial, oldSwell, swell) / (maxSwell - 2)`.
pub(crate) fn creeper_scale(e: &Entity, partial: f32) -> Vec3 {
    let swelling = lerp(partial, e.old_swell as f32, e.swell as f32)
        / (crate::store::CREEPER_MAX_SWELL - 2) as f32;
    let mut g = swelling;
    let wobble = 1.0 + (g * 100.0).sin() * g * 0.01;
    g = g.clamp(0.0, 1.0);
    g *= g;
    g *= g;
    Vec3::new(
        (1.0 + g * 0.4) * wobble,
        (1.0 + g * 0.1) / wobble,
        (1.0 + g * 0.4) * wobble,
    )
}

/// `QuadrupedModel.setupAnim` (also `CreeperModel.setupAnim`).
fn quadruped(o: &mut PoseOverrides, walk_pos: f32, walk_speed: f32) {
    let c = (walk_pos * 0.6662).cos() * 1.4 * walk_speed;
    let c2 = (walk_pos * 0.6662 + PI).cos() * 1.4 * walk_speed;
    o.set_rotation(parts::RIGHT_HIND_LEG, Vec3::new(c, 0.0, 0.0));
    o.set_rotation(parts::LEFT_HIND_LEG, Vec3::new(c2, 0.0, 0.0));
    o.set_rotation(parts::RIGHT_FRONT_LEG, Vec3::new(c2, 0.0, 0.0));
    o.set_rotation(parts::LEFT_FRONT_LEG, Vec3::new(c, 0.0, 0.0));
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
    o.set_rotation(parts::RIGHT_HIND_LEG, leg(swing_hind, step_hind));
    o.set_rotation(parts::LEFT_HIND_LEG, leg(-swing_hind, -step_hind));
    o.set_rotation(
        parts::RIGHT_MIDDLE_HIND_LEG,
        leg(swing_mid_hind, step_mid_hind),
    );
    o.set_rotation(
        parts::LEFT_MIDDLE_HIND_LEG,
        leg(-swing_mid_hind, -step_mid_hind),
    );
    o.set_rotation(
        parts::RIGHT_MIDDLE_FRONT_LEG,
        leg(swing_mid_front, step_mid_front),
    );
    o.set_rotation(
        parts::LEFT_MIDDLE_FRONT_LEG,
        leg(-swing_mid_front, -step_mid_front),
    );
    o.set_rotation(parts::RIGHT_FRONT_LEG, leg(swing_front, step_front));
    o.set_rotation(parts::LEFT_FRONT_LEG, leg(-swing_front, -step_front));
}

/// `ChickenModel.setupAnim` (legs walk, wings flap).
fn chicken(o: &mut PoseOverrides, e: &Entity, partial: f32, walk_pos: f32, walk_speed: f32) {
    let flap = lerp(partial, e.o_flap, e.flap);
    let flap_speed = lerp(partial, e.o_flap_speed, e.flap_speed);
    let flap_angle = (flap.sin() + 1.0) * flap_speed;
    o.set_rotation(
        parts::RIGHT_LEG,
        Vec3::new((walk_pos * 0.6662).cos() * 1.4 * walk_speed, 0.0, 0.0),
    );
    o.set_rotation(
        parts::LEFT_LEG,
        Vec3::new((walk_pos * 0.6662 + PI).cos() * 1.4 * walk_speed, 0.0, 0.0),
    );
    o.set_rotation(parts::RIGHT_WING, Vec3::new(0.0, 0.0, flap_angle));
    o.set_rotation(parts::LEFT_WING, Vec3::new(0.0, 0.0, -flap_angle));
}
