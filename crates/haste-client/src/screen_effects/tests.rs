use super::*;

/// A GUI frame size to draw into (GUI pixels at scale 4 of 1920x1080).
const W: u32 = 480;
const H: u32 = 270;

fn view(e: ViewEffects) -> ViewEffects {
    e
}

fn frame(partial_tick: f32, effects: &ViewEffects) -> Frame<'_> {
    Frame {
        width: W,
        height: H,
        first_person: true,
        effects,
        partial_tick,
        look: (0.0, 0.0),
        scope_scale: 1.125,
    }
}

/// The frame's overlay commands without the vignette, which every frame draws: the tests of
/// the other overlays count their own commands.
fn without_vignette(f: &Frame) -> Vec<GuiCommand> {
    let mut out = Vec::new();
    f.append_to(&mut out);
    out.retain(|c| {
        !matches!(c, GuiCommand::Texture { texture, .. } if texture.path() == "textures/misc/vignette.png")
    });
    out
}

/// A nausea instance as the server sends it (`effect give nausea 30`: amplifier 0,
/// duration 600 ticks, the blend flag clear, which means `skipBlending`).
fn nausea(blend: bool, duration: i32) -> Effect {
    Effect::new(
        0,
        duration,
        blend,
        NAUSEA_BLEND_IN,
        NAUSEA_BLEND_OUT,
        NAUSEA_BLEND_OUT_ADVANCE,
    )
}

// ---- the nausea blend, per tick ---------------------------------------------

#[test]
fn nausea_skips_blending_when_the_packet_says_so() {
    // `ClientPacketListener.handleUpdateMobEffect` calls `skipBlending` when the packet
    // has no blend flag: the factor starts at its target.
    let e = nausea(false, 600);
    assert_eq!(e.blend_pair(), (1.0, 1.0));
}

#[test]
fn nausea_blends_in_one_150th_per_tick() {
    let mut e = nausea(true, 600);
    e.tick();
    assert!(
        (e.blend_pair().1 - 1.0 / 150.0).abs() < 1e-6,
        "{}",
        e.blend_pair().1
    );
    // The previous tick's factor is the frame's start (`BlendState.factorPreviousFrame`).
    assert_eq!(e.blend_pair().0, 0.0);
    assert!((e.blend_factor(0.5) - 1.0 / 300.0).abs() < 1e-6);
    // 150 ticks of blending in reaches the full intensity.
    for _ in 0..149 {
        e.tick();
    }
    assert!(
        (e.blend_pair().1 - 1.0).abs() < 1e-6,
        "{}",
        e.blend_pair().1
    );
}

#[test]
fn nausea_blends_out_at_one_20th_once_it_is_ending() {
    // The blend-in runs at 1/150 per tick while the effect is more than the 60-tick advance
    // from ending. From 210 ticks, 149 ticks blend the factor to 149/150 with the duration
    // still at 61 (`endsWithin(60)` false).
    let mut e = nausea(true, 60 + 150);
    for _ in 0..149 {
        e.tick();
    }
    assert!(
        (e.blend_pair().1 - 149.0 / 150.0).abs() < 1e-6,
        "{}",
        e.blend_pair().1
    );
    // The 150th tick brings the duration to 60, so the effect starts ending and the blend
    // state eases out by 1/20 per tick, not the blend-in's 1/150.
    e.tick();
    assert!(e.ends_within(60), "duration {}", e.duration);
    let before = e.blend_pair().1;
    e.tick();
    assert!(
        (e.blend_pair().1 - (before - 1.0 / 20.0)).abs() < 1e-6,
        "{} -> {}",
        before,
        e.blend_pair().1
    );
}

#[test]
fn an_infinite_duration_never_blends_out() {
    let mut e = nausea(true, -1);
    for _ in 0..400 {
        e.tick();
    }
    assert!(!e.ends_within(60));
    assert!((e.blend_pair().1 - 1.0).abs() < 1e-6);
}

// ---- the portal and spinning effects ----------------------------------------

#[test]
fn standing_in_a_portal_rises_the_intensity_by_1_80_per_tick() {
    let mut p = PortalEffects::default();
    for tick in 1..=80 {
        // Standing in the portal tick after tick, as the game thread's world probe says.
        p.set_inside_portal(true);
        p.tick(true, 0.0);
        assert!(
            (p.portal() - 0.0125 * tick as f32).abs() < 1e-6,
            "tick {tick}: {}",
            p.portal()
        );
    }
    // 80 ticks of 1/80 is 1.0: the full portal intensity.
    assert!((p.portal() - 1.0).abs() < 1e-6);
}

#[test]
fn leaving_a_portal_falls_the_intensity_by_1_20_per_tick() {
    let mut p = PortalEffects::default();
    for _ in 0..80 {
        p.set_inside_portal(true);
        p.tick(true, 0.0);
    }
    p.set_inside_portal(false);
    p.tick(true, 0.0);
    assert!((p.portal() - (1.0 - 0.05)).abs() < 1e-6, "{}", p.portal());
    // The fall never goes below zero, and holds there without spinning on.
    for _ in 0..20 {
        p.tick(true, 0.0);
    }
    assert_eq!(p.portal(), 0.0);
    assert_eq!(p.spinning_speed(), 0.0);
}

#[test]
fn the_spinning_speed_is_the_weighted_mean_of_the_two_intensities() {
    // Vanilla's `tickSpinningEffect`: the speed is
    // (portal * 20 + nausea * 7) / (portal + nausea), and the angle advances by it.
    let at = |portal: f32, nausea: f32| -> (f32, f32) {
        let mut p = PortalEffects {
            portal,
            prev_portal: portal,
            inside_this_tick: false,
            spinning_time: 0.0,
            spinning_speed: 0.0,
        };
        p.tick(true, nausea);
        (p.spinning_speed(), p.spinning_time())
    };
    // Portal only spins at 20, nausea only at 7, and their mean is the weighted average of
    // the intensities *after* this tick's portal step (vanilla `tickSpinningEffect` runs
    // after `handlePortalTransitionEffect`, so 0.5 falls to 0.45 first).
    assert!((at(0.5, 0.0).0 - 20.0).abs() < 1e-6);
    assert!((at(0.0, 1.0).0 - 7.0).abs() < 1e-6);
    let expected = (0.45 * 20.0 + 0.5 * 7.0) / (0.45 + 0.5);
    let (speed, time) = at(0.5, 0.5);
    assert!((speed - expected).abs() < 1e-6, "{speed}");
    assert!(
        (time - expected).abs() < 1e-6,
        "the angle advances by the speed"
    );
    // No portal and no nausea: no spin at all.
    assert_eq!(at(0.0, 0.0), (0.0, 0.0));
}

#[test]
fn no_portal_and_no_nausea_spins_not_at_all() {
    let mut p = PortalEffects::default();
    p.tick(true, 0.0);
    assert_eq!(p.spinning_speed(), 0.0);
    assert_eq!(p.portal(), 0.0);
    assert_eq!(p.spinning_time(), 0.0);
}

// ---- the screen warp ---------------------------------------------------------

#[test]
fn the_warp_squeezes_x_by_vanillas_skew() {
    // `GameRenderer.renderLevel`: skew = (5 / (I² + 5) - 0.04 I)².
    let w = SpinWarp {
        angle_deg: 0.0,
        intensity: 1.0,
    };
    let skew = 5.0 / 6.0 - 0.04;
    assert!((w.skew() - skew * skew).abs() < 1e-6);
    // At full intensity the screen is pulled wide in x: 1/skew ≈ 1.59.
    assert!((1.0 / w.skew() - 1.59).abs() < 0.01);
    // A weak intensity barely squeezes (the 5/5 dominates).
    let weak = SpinWarp {
        angle_deg: 0.0,
        intensity: 0.01,
    };
    assert!(weak.skew() > 0.99);
}

#[test]
fn the_warp_needs_portal_or_nausea() {
    let calm = view(ViewEffects::default());
    assert!(calm.spin_warp(0.5).is_none());
    let spinning = view(ViewEffects {
        nausea: (1.0, 1.0),
        spinning_time: 12.0,
        spinning_speed: 7.0,
        ..Default::default()
    });
    let w = spinning.spin_warp(0.5).expect("nausea spins");
    assert!((w.intensity - 1.0).abs() < 1e-6);
    // Half a tick of spinning at 7 per tick: the angle has advanced by 3.5.
    assert!((w.angle_deg - 15.5).abs() < 1e-6);
    // The nausea intensity itself is the pair lerped by the partial tick.
    let half = view(ViewEffects {
        nausea: (0.0, 1.0),
        ..Default::default()
    });
    assert!((half.spin_warp(0.5).unwrap().intensity - 0.5).abs() < 1e-6);
}

#[test]
fn the_portal_wins_over_nausea_in_the_warp() {
    let both = view(ViewEffects {
        portal: (0.0, 0.25),
        nausea: (0.0, 1.0),
        spinning_time: 0.0,
        spinning_speed: 20.0,
        ..Default::default()
    });
    // At the end of the tick `max(portal, nausea)` is the nausea's 1.0 here.
    let w = both.spin_warp(1.0).expect("spinning");
    assert!((w.intensity - 1.0).abs() < 1e-6);
}

// ---- the freeze overlay ------------------------------------------------------

#[test]
fn freeze_ticks_are_a_fraction_of_140() {
    assert_eq!(percent_frozen(0), 0.0);
    assert!((percent_frozen(70) - 0.5).abs() < 1e-6);
    // Fully frozen (140) and past it both read 1.
    assert_eq!(percent_frozen(140), 1.0);
    assert_eq!(percent_frozen(999), 1.0);
}

#[test]
fn the_freeze_ticks_rise_in_powder_snow_and_fall_outside_it() {
    // Powder snow's `FREEZE` adds one per tick, capped at `getTicksRequiredToFreeze`.
    let mut ticks = 0;
    for _ in 0..FREEZE_TICKS {
        ticks = tick_frozen(ticks, true, false);
    }
    assert_eq!(ticks, FREEZE_TICKS);
    // It never goes above the cap.
    assert_eq!(tick_frozen(FREEZE_TICKS, true, false), FREEZE_TICKS);
    // Outside, the server's rule eases them out by two per tick, never below zero.
    assert_eq!(tick_frozen(10, false, false), 8);
    assert_eq!(tick_frozen(1, false, false), 0);
    assert_eq!(tick_frozen(0, false, false), 0);
    // Fire's `CLEAR_FREEZE` zeroes them at once.
    assert_eq!(tick_frozen(120, false, true), 0);
}

// ---- the water overlay -------------------------------------------------------

/// The water overlay's tile quads: `(x, y, w, h, u, v, uw, vh)`.
type WaterTile = (i32, i32, i32, i32, f32, f32, f32, f32);

fn water_tiles(f: &Frame) -> Vec<WaterTile> {
    let mut out = Vec::new();
    f.append_to(&mut out);
    out.iter()
        .filter_map(|c| match c {
            GuiCommand::Texture {
                texture,
                x,
                y,
                w,
                h,
                u,
                v,
                uw,
                vh,
                ..
            } if texture.path() == "textures/misc/underwater.png" => {
                Some((*x, *y, *w, *h, *u, *v, *uw, *vh))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_water_overlay_is_alpha_tenth_with_the_eyes_light() {
    // `ARGB.colorFromFloat(0.1, brightness, brightness, brightness)` with the eye at
    // light 7: brightness = (7/15) / (4 - 3 * 7/15).
    let e = view(ViewEffects {
        eye_in_water: true,
        eye_light: 7,
        ..Default::default()
    });
    let brightness = light_brightness(7, 0.0);
    assert!((brightness - 0.17948).abs() < 1e-4, "{brightness}");
    let mut out = vec![GuiCommand::Text {
        x: 0,
        y: 0,
        text: "hud".into(),
        color: 0,
        shadow: false,
    }];
    frame(0.5, &e).append_to(&mut out);
    // Every tile carries the alpha-0.1 tint of the eye's light.
    let channel = (0.17948 * 255.0) as u32;
    let tint = 25 << 24 | channel << 16 | channel << 8 | channel;
    let mut seen = 0;
    for c in &out {
        if let GuiCommand::Texture {
            texture, tint: t, ..
        } = c
        {
            if texture.path() == "textures/misc/underwater.png" {
                assert_eq!(*t, tint);
                seen += 1;
            }
        }
    }
    assert!(seen >= 1, "the water tiles");
}

#[test]
fn the_water_overlay_tiles_the_texture_across_the_screen() {
    // Vanilla draws one quad over the whole screen with a uv span of 4 and a REPEAT
    // sampler; the tiles must cover the screen with each tile a period of the texture.
    let e = view(ViewEffects {
        eye_in_water: true,
        eye_light: 15,
        ..Default::default()
    });
    let tiles = water_tiles(&frame(0.5, &e));
    assert!(tiles.len() > 1, "the texture tiles, not one stretched quad");
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (i32::MAX, 0, i32::MAX, 0);
    for &(x, y, w, h, _u, _v, uw, vh) in &tiles {
        assert!(w > 0 && h > 0, "a tile has area");
        assert!(
            (-1.0 - 1e-6..=1e-6).contains(&uw),
            "u spans a period or less"
        );
        assert!(
            (-1e-6..=1.0 + 1e-6).contains(&vh),
            "v spans a period or less"
        );
        min_x = min_x.min(x);
        max_x = max_x.max(x + w);
        min_y = min_y.min(y);
        max_y = max_y.max(y + h);
    }
    assert_eq!((min_x, min_y), (0, 0));
    assert_eq!((max_x, max_y), (W as i32, H as i32));
}

#[test]
fn the_water_overlay_scrolls_by_the_look() {
    // The uv at a screen pixel is vanilla's `u = uOffset + 2 - (px - w/2) / tile` (in
    // periods), with `uOffset = -yRot / 64`; the leftmost tile samples it at `px = 0`.
    let e = view(ViewEffects {
        eye_in_water: true,
        eye_light: 15,
        ..Default::default()
    });
    let f = Frame {
        width: W,
        height: H,
        first_person: true,
        effects: &e,
        partial_tick: 0.5,
        look: (128.0, -64.0),
        scope_scale: 1.125,
    };
    let tiles = water_tiles(&f);
    let tile = H as f32 / (hud_fov(true).to_radians() * 0.5).tan() * 0.5;
    let raw = -128.0 / 64.0 + 2.0 - (0.0 - W as f32 * 0.5) / tile;
    let expected = raw - (raw.ceil() - 1.0);
    let left = tiles
        .iter()
        .find(|t| t.0 == 0)
        .expect("a tile at the left edge");
    assert!((left.4 - expected).abs() < 1e-4, "{} vs {expected}", left.4);
    // A different yaw scrolls the texture: the left edge samples a different uv.
    let mut turned = f;
    turned.look = (96.0, -64.0);
    let turned_left = water_tiles(&turned);
    let t_left = turned_left.iter().find(|t| t.0 == 0).unwrap();
    assert!(
        (t_left.4 - left.4).abs() > 1e-3,
        "the look scrolls the water"
    );
}

#[test]
fn the_water_overlay_is_first_person_only() {
    let e = view(ViewEffects {
        eye_in_water: true,
        ..Default::default()
    });
    let f = Frame {
        first_person: false,
        ..frame(0.5, &e)
    };
    let out = without_vignette(&f);
    assert!(out.is_empty(), "third person draws no water overlay");
}

// ---- the fire overlay --------------------------------------------------------

#[test]
fn the_fire_overlay_is_two_quads_over_the_bottom_of_the_screen() {
    let e = view(ViewEffects {
        on_fire: true,
        ..Default::default()
    });
    let out = without_vignette(&frame(0.5, &e));
    assert_eq!(out.len(), 2, "the fire overlay is two quads");
    let scale = (H as f32) / (hud_fov(false).to_radians() * 0.5).tan();
    // The quads sit at the bottom: their top edge is 0.2 view units above the centre and
    // one view unit tall (`ScreenEffectRenderer.submitFire`'s quad is 1x1 at z = -0.5,
    // translated to y = -0.3).
    let GuiCommand::Texture {
        texture,
        x: x0,
        y: y0,
        w: w0,
        h: h0,
        u,
        v,
        uw,
        vh,
        ..
    } = &out[0]
    else {
        panic!("expected a fire quad")
    };
    // One animation frame, mirrored in x, upright in y (`buildSpriteQuad`).
    assert_eq!(texture.path(), "block/fire_1");
    assert_eq!((*u, *v, *uw, *vh), (1.0, 0.0, -1.0, 1.0));
    assert!(((H as f32 / 2.0 - 0.2 * scale) - *y0 as f32).abs() <= 1.0);
    assert!((*h0 as f32 - scale).abs() <= 1.0);
    // 1 view unit is the height / tan(fov/2): a 480x270 window at 70 degrees has its quad
    // top at 135 - 0.2 * 386 = 58 pixels, 386 tall.
    assert_eq!((*y0, *h0), (58, 386));
    // Both quads are 1 view unit wide before the rotation's squeeze, and the pair is
    // symmetric about the centre: the left quad's right edge is as far from the centre
    // as the right quad's left edge.
    let GuiCommand::Texture { x: x1, w: w1, .. } = &out[1] else {
        panic!("expected the second fire quad")
    };
    assert_eq!(*w0, *w1);
    // Symmetric about the centre (W / 2): the left quad's right edge and the right quad's
    // left edge are equidistant from it, so their sum is the width.
    assert_eq!(
        (*x0 + *w0) + *x1,
        W as i32,
        "the pair is symmetric about the centre: {x0}..{} and {x1}..{}",
        x0 + w0,
        x1 + w1
    );
}

#[test]
fn the_fire_overlay_uses_the_underwater_hud_fov() {
    // Underwater the HUD field of view drops to 70 * 0.857, which makes the fire quads
    // larger on screen.
    assert!((hud_fov(true) - 60.0).abs() < 1e-3);
    assert!((hud_fov(false) - 70.0).abs() < 1e-3);
    let dry = quad_height(&view(ViewEffects {
        on_fire: true,
        ..Default::default()
    }));
    let wet = quad_height(&view(ViewEffects {
        on_fire: true,
        eye_in_water: true,
        ..Default::default()
    }));
    assert!(wet > dry);
}

/// The height (GUI pixels) of the first fire quad in a frame.
fn quad_height(e: &ViewEffects) -> i32 {
    let mut out = Vec::new();
    frame(0.5, e).append_to(&mut out);
    out.iter()
        .find_map(|c| match c {
            GuiCommand::Texture { texture, h, .. } if texture.path() == "block/fire_1" => Some(*h),
            _ => None,
        })
        .expect("a fire quad")
}

// ---- the block the camera is inside ------------------------------------------

#[test]
fn the_block_overlay_draws_the_blocks_particle_texture() {
    let e = view(ViewEffects {
        block_overlay: Some(Identifier::minecraft("block/dirt")),
        ..Default::default()
    });
    let mut out = Vec::new();
    frame(0.5, &e).append_to(&mut out);
    let GuiCommand::Texture { texture, tint, .. } = &out[0] else {
        panic!("expected the block overlay")
    };
    assert_eq!(texture.path(), "block/dirt");
    // `ScreenEffectRenderer.submitBlockSprite`'s `-15132391`: a 10% white.
    assert_eq!(*tint, 0xFF19_1919);
}

// ---- the worn item's overlay -------------------------------------------------

// ---- the spectator -----------------------------------------------------------

#[test]
fn a_spectator_draws_neither_water_nor_fire_overlay() {
    // `ScreenEffectRenderer.submit`: `firstPerson && !isSpectator` gates the water and
    // fire overlays — a spectator inside a solid block sees the caves, not a screenful
    // of stone, and no water or fire over the world either.
    let e = view(ViewEffects {
        eye_in_water: true,
        on_fire: true,
        spectator: true,
        ..Default::default()
    });
    let out = without_vignette(&frame(0.5, &e));
    assert!(out.is_empty(), "no level-pass overlays for a spectator");
}

/// The probe over a world of solid stone: the same world, once as a walking player (the
/// overlay is found) and once as a spectator (`LevelExtractor.getViewBlockingState`
/// answers nothing while `noPhysics`). Needs the data generator reports in the cache.
#[test]
#[ignore = "needs cache/datagen/26.3/reports (GameData::load)"]
fn the_probe_skips_the_view_blocking_block_for_a_spectator() {
    let data = crate::net::load_game_data().unwrap();
    let dim = haste_protocol::haste_core::game::DimensionInfo::overworld();
    let mut world = haste_world::World::new_with_biomes(dim.clone(), data.clone(), 67);
    let stone = haste_core::BlockStateId(1);
    let c = haste_world::chunk::synthetic_chunk_data(
        haste_core::ChunkPos::new(0, 0),
        dim.min_section(),
        dim.section_count(),
        |_, _, _, _| stone,
    );
    world.load_chunk(&c).unwrap();
    // `causes_suffocation` on stone, so the stone around the eye is view-blocking.
    let index = data
        .blocks
        .block_index(data.blocks.block_of(stone))
        .unwrap();
    let tags = haste_physics::mining::BlockTags::new(vec![(
        Identifier::minecraft("causes_suffocation"),
        vec![index.0 as i32],
    )]);
    let probe = |spectator: bool| {
        Probe {
            world: &world,
            shapes: &data.shapes,
            blocks: &data.blocks,
            tags: Some(&tags),
            eye: glam::DVec3::new(0.5, 0.6, 0.5),
            feet: glam::DVec3::new(0.5, 0.0, 0.5),
            bb_width: 0.6,
            bb_height: 1.8,
            scale: 1.0,
            spectator,
        }
        .sample()
    };
    use haste_core::view::BlockView as _;
    assert_eq!(
        world.block_state(haste_core::BlockPos::new(0, 0, 0)),
        stone,
        "the eye block is stone"
    );
    assert!(
        probe(false).block_overlay.is_some(),
        "a walking player in stone sees the overlay"
    );
    let s = probe(true);
    assert!(s.block_overlay.is_none(), "a spectator sees through it");
    assert!(!s.in_fire && !s.in_powder_snow && !s.in_portal);
}

#[test]
fn only_the_carved_pumpkin_hides_the_screen() {
    assert_eq!(
        camera_overlay(&Identifier::minecraft("carved_pumpkin")),
        Some(Identifier::minecraft("textures/misc/pumpkinblur.png"))
    );
    // Every other head item (and every unknown id) draws nothing.
    assert_eq!(camera_overlay(&Identifier::minecraft("player_head")), None);
    assert_eq!(
        camera_overlay(&Identifier::minecraft("turtle_helmet")),
        None
    );
}

#[test]
fn the_pumpkin_overlay_covers_the_screen_untinted() {
    let e = view(ViewEffects {
        camera_overlay: camera_overlay(&Identifier::minecraft("carved_pumpkin")),
        ..Default::default()
    });
    let out = without_vignette(&frame(0.5, &e));
    let GuiCommand::Texture { texture, tint, .. } = &out[0] else {
        panic!("expected the pumpkin overlay")
    };
    assert_eq!(texture.path(), "textures/misc/pumpkinblur.png");
    assert_eq!(*tint, 0xFFFF_FFFF);
}

// ---- the spyglass scope -------------------------------------------------------

#[test]
fn the_spyglass_scope_is_a_centred_square_with_black_around_it() {
    let e = view(ViewEffects {
        scoping: true,
        ..Default::default()
    });
    let out = without_vignette(&frame(0.5, &e));
    assert_eq!(out.len(), 5, "the scope texture plus four black fills");
    // `min(w, h) * 1.125`, centred.
    let GuiCommand::Texture { x, y, w, h, .. } = &out[0] else {
        panic!("expected the scope texture")
    };
    let size = ((W.min(H)) as f32 * 1.125).floor() as i32;
    assert_eq!((*w, *h), (size, size));
    assert_eq!((*x, *y), ((W as i32 - size) / 2, (H as i32 - size) / 2));
    for (i, fill) in out[1..].iter().enumerate() {
        let GuiCommand::Fill { color, .. } = fill else {
            panic!("expected a black fill {i}")
        };
        assert_eq!(*color, 0xFF00_0000);
    }
}

#[test]
fn the_scope_grows_over_frames_while_scoping() {
    // Vanilla eases the scale towards 1.125 by half the frame's ticks and resets it to
    // 0.5 when we are not scoping.
    let mut scale = 0.5f32;
    let step = lerp(0.5 * 0.139, scale, 1.125);
    assert!(step > scale);
    assert!(step < 1.125);
    scale = step;
    assert!((scale - (0.5 + 0.625 * 0.0695)).abs() < 1e-3);
    // Many frames of scoping reach the maximum.
    for _ in 0..1000 {
        scale = lerp(0.5 * 0.139, scale, 1.125);
    }
    assert!((scale - 1.125).abs() < 1e-3);
}

// ---- the portal and confusion overlays ---------------------------------------

#[test]
fn the_portal_overlay_alpha_is_squared_twice_and_lifted() {
    let at = |intensity: f32| {
        let e = view(ViewEffects {
            portal: (0.0, intensity),
            ..Default::default()
        });
        // Partial 1.0 reads the pair's second component, the intensity this tick.
        let out = without_vignette(&frame(1.0, &e));
        let GuiCommand::Texture { tint, .. } = &out[0] else {
            panic!("expected the portal overlay")
        };
        *tint >> 24
    };
    // alpha < 1: alpha *= alpha; alpha *= alpha; alpha * 0.8 + 0.2.
    // At 0.5: 0.25 -> 0.0625 -> 0.25.
    assert_eq!(at(0.5), (0.25 * 255.0) as u32);
    // At 1.0 the curve is skipped entirely.
    assert_eq!(at(1.0), 255);
    // Even a sliver of portal draws at alpha 0.2.
    assert_eq!(at(0.01), (0.2 * 255.0) as u32);
}

#[test]
fn the_confusion_overlay_zooms_and_tints_at_the_default_scale_never() {
    // Screen-effect scale 1.0 (vanilla's default) draws no confusion overlay at all; the
    // nausea shows as the projection warp instead.
    let e = view(ViewEffects {
        nausea: (1.0, 1.0),
        ..Default::default()
    });
    let out = without_vignette(&frame(0.5, &e));
    assert!(out.is_empty());

    // The overlay itself (below 100%): zoomed about the centre by `lerp(strength, 2, 1)`
    // and added to the screen in a green tint.
    let strength = 0.5;
    let size = lerp(strength, 2.0, 1.0);
    assert!((size - 1.5).abs() < 1e-6);
    let tint = argb(1.0, 0.2 * strength, 0.4 * strength, 0.2 * strength);
    assert_eq!(tint, argb(1.0, 0.1, 0.2, 0.1));
    // The zoom samples the texture's middle `1 / size` part: at strength 0 the quad is
    // twice the screen, so the uv window is half the texture, from 0.25 to 0.75.
    let size = lerp(0.0, 2.0, 1.0);
    let span = 1.0 / size;
    assert!((span - 0.5).abs() < 1e-6);
    assert!(((1.0 - span) / 2.0 - 0.25).abs() < 1e-6);
}

// ---- the frame's order -------------------------------------------------------

#[test]
fn every_overlay_draws_under_the_huds_own_commands() {
    let e = view(ViewEffects {
        block_overlay: Some(Identifier::minecraft("block/sand")),
        eye_in_water: true,
        on_fire: true,
        camera_overlay: camera_overlay(&Identifier::minecraft("carved_pumpkin")),
        frozen_ticks: 70,
        scoping: true,
        portal: (1.0, 1.0),
        ..Default::default()
    });
    let hud = GuiCommand::Text {
        x: 1,
        y: 2,
        text: "hotbar".into(),
        color: 0,
        shadow: false,
    };
    let mut out = vec![hud];
    frame(0.5, &e).append_to(&mut out);
    // Vanilla's order, first to last: the level pass's block, water and fire, then the
    // HUD's camera overlays: the vignette, the spyglass scope and its black fills, the freeze
    // outline, then the portal overlay.
    let names: Vec<&str> = out
        .iter()
        .map(|c| match c {
            GuiCommand::Texture { texture, .. } => texture.path(),
            GuiCommand::Fill { .. } => "black fill",
            GuiCommand::Text { .. } => "hud",
            other => other_variant(other),
        })
        .collect();
    // The water is drawn as several consecutive tile quads.
    let water = names
        .iter()
        .filter(|n| **n == "textures/misc/underwater.png")
        .count();
    let mut expected = vec!["block/sand"];
    expected.extend(std::iter::repeat_n("textures/misc/underwater.png", water));
    expected.extend([
        "block/fire_1",
        "block/fire_1",
        "textures/misc/vignette.png",
        "textures/misc/spyglass_scope.png",
        "black fill",
        "black fill",
        "black fill",
        "black fill",
        "textures/misc/powder_snow_outline.png",
        "block/nether_portal",
        "hud",
    ]);
    assert_eq!(names, expected);
}

/// Scoping hides the worn item's overlay, as vanilla's `extractCameraOverlays` picks the
/// spyglass branch instead of the item loop.
#[test]
fn scoping_draws_the_scope_instead_of_the_head_item() {
    let e = view(ViewEffects {
        scoping: true,
        camera_overlay: camera_overlay(&Identifier::minecraft("carved_pumpkin")),
        ..Default::default()
    });
    let mut out = Vec::new();
    frame(0.5, &e).append_to(&mut out);
    assert!(
        !out.iter().any(|c| matches!(
            c,
            GuiCommand::Texture { texture, .. } if texture.path().contains("pumpkinblur")
        )),
        "scoping must not draw the pumpkin overlay"
    );
    assert!(out.iter().any(|c| matches!(
        c,
        GuiCommand::Texture { texture, .. } if texture.path().contains("spyglass_scope")
    )));
}

fn other_variant(c: &GuiCommand) -> &'static str {
    match c {
        GuiCommand::Text { .. } => "hud",
        _ => "other",
    }
}

#[test]
fn a_calm_frame_appends_only_the_white_vignette() {
    let e = view(ViewEffects::default());
    let mut out = vec![GuiCommand::Text {
        x: 0,
        y: 0,
        text: "crosshair".into(),
        color: 0,
        shadow: false,
    }];
    frame(0.5, &e).append_to(&mut out);
    assert_eq!(out.len(), 2);
    assert_eq!(vignette_tint(&out[0]), (0xFF_FF_FF_FF, GuiBlend::Multiply));
}

/// The tint and blend of a full-screen vignette command.
fn vignette_tint(c: &GuiCommand) -> (u32, GuiBlend) {
    match c {
        GuiCommand::Texture { tint, blend, .. } => (*tint, *blend),
        other => panic!("expected the vignette texture, got {other:?}"),
    }
}

#[test]
fn the_border_warning_turns_the_vignette_red_toward_the_edge() {
    // `Hud.extractVignette` at brightness 1: red falls by the strength, green and blue stay lit.
    let half = view(ViewEffects {
        border_warning: 0.5,
        ..ViewEffects::default()
    });
    let mut out = Vec::new();
    frame(0.5, &half).append_to(&mut out);
    assert_eq!(vignette_tint(&out[0]), (0xFF_7F_FF_FF, GuiBlend::Multiply));

    let edge = view(ViewEffects {
        border_warning: 1.0,
        ..ViewEffects::default()
    });
    let mut out = Vec::new();
    frame(0.5, &edge).append_to(&mut out);
    assert_eq!(vignette_tint(&out[0]), (0xFF_00_FF_FF, GuiBlend::Multiply));
}
