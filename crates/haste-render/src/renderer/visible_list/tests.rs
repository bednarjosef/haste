use super::*;

#[test]
fn sections_are_grouped_by_arena_in_visit_order() {
    // (arena, visit index) pairs.
    let src = [(2, 0), (0, 1), (2, 2), (1, 3), (0, 4)];
    let mut out = vec![(7, 99)];
    let mut offsets = Vec::new();
    group_by_arena(&mut out, &src, |d| d.0, &mut offsets);
    assert_eq!(out, [(7, 99), (0, 1), (0, 4), (1, 3), (2, 0), (2, 2)]);
    group_by_arena(&mut out, &[], |d: &(u32, u32)| d.0, &mut offsets);
    assert_eq!(out.len(), 6);
}

fn layer(layer: RenderLayer, dir: usize, first: u32, count: u32) -> SlabLayer {
    SlabLayer {
        layer,
        first,
        count,
        dir: dir as u8,
        kind: SpanKind::Indexed,
    }
}

/// The draws the old per-group loop made for `layers`: each group facing the camera,
/// joined to the previous draw when its range continues it.
fn per_group_draws(layers: &[SlabLayer], front: u16) -> Vec<(RenderLayer, u32, u32)> {
    let mut out: Vec<(RenderLayer, u32, u32)> = Vec::new();
    for l in layers {
        if front & (1 << l.dir) == 0 {
            continue;
        }
        match out.iter_mut().rev().find(|d| d.0 == l.layer) {
            Some(prev) if prev.1 + prev.2 == l.first => prev.2 += l.count,
            _ => out.push((l.layer, l.first, l.count)),
        }
    }
    out
}

fn run_draws(layers: &[SlabLayer], front: u16) -> Vec<(RenderLayer, u32, u32)> {
    let mut groups = Vec::new();
    push_groups(&mut groups, layers);
    let mut out: Vec<(RenderLayer, u32, u32)> = Vec::new();
    for g in &groups {
        g.for_each_run(front_positions(front), |first, count| {
            match out.iter_mut().rev().find(|d| d.0 == g.layer) {
                Some(prev) if prev.1 + prev.2 == first => prev.2 += count,
                _ => out.push((g.layer, first, count)),
            }
        });
    }
    out
}

/// A section's layer list as the arena writes it: per render layer, the nonempty
/// groups of `present` (a bit per group) in storage order, back to back.
fn section(layers: &[(RenderLayer, u32)]) -> Vec<SlabLayer> {
    let mut out = Vec::new();
    let mut next = 0;
    for &(l, present) in layers {
        for d in GROUP_ORDER {
            if present & (1 << d) != 0 {
                let count = 6 * (1 + d as u32 % 3);
                out.push(layer(l, d, next, count));
                next += count;
            }
        }
        // The next layer's indices start after a gap, as in the pool.
        next += 6;
    }
    out
}

#[test]
fn group_runs_match_the_per_group_loop() {
    let all = (1u32 << GROUPS) - 1;
    let masks = [
        all,
        0b1,
        0b100_0000,
        0b101_0101_0101,
        0b010_1010_1010,
        0b111_0000_1111,
    ];
    for &present in &masks {
        for second in [0, 0b11, all] {
            let mut layers = vec![(RenderLayer::Solid, present)];
            if second != 0 {
                layers.push((RenderLayer::Translucent, second));
            }
            let layers = section(&layers);
            for front in 0..1u16 << GROUPS {
                assert_eq!(
                    run_draws(&layers, front),
                    per_group_draws(&layers, front),
                    "present {present:#b} second {second:#b} front {front:#b}"
                );
            }
        }
    }
}

#[test]
fn ranges_that_do_not_continue_start_a_new_run() {
    // Out of storage order, a gap in the pool and a non-indexed span each start a new
    // record; the draws still match the per-group loop.
    let mut quad = layer(RenderLayer::Solid, 4, 100, 6);
    quad.kind = SpanKind::General;
    let layers = [
        layer(RenderLayer::Solid, 2, 0, 6),
        layer(RenderLayer::Solid, 1, 6, 6),
        layer(RenderLayer::Solid, 4, 20, 6),
        quad,
    ];
    let mut groups = Vec::new();
    push_groups(&mut groups, &layers);
    assert_eq!(groups.len(), 4);
    for front in 0..1u16 << GROUPS {
        assert_eq!(
            run_draws(&layers[..3], front),
            per_group_draws(&layers[..3], front)
        );
    }
}
