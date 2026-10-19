//! The player options that map onto renderer settings (vanilla's options screens change
//! them live; the CLI can force some for one run). Split out of `frame.rs` (CI keeps
//! that file under 600 lines).

/// The config's cloud option as the renderer's (vanilla `Options.cloudStatus` →
/// `OptionsRenderState.cloudStatus`).
pub(crate) fn cloud_status(
    status: haste_setup::options::CloudStatus,
) -> haste_render::clouds::CloudStatus {
    match status {
        haste_setup::options::CloudStatus::Off => haste_render::clouds::CloudStatus::Off,
        haste_setup::options::CloudStatus::Fast => haste_render::clouds::CloudStatus::Fast,
        haste_setup::options::CloudStatus::Fancy => haste_render::clouds::CloudStatus::Fancy,
    }
}

/// The config's pre-mesh option as the renderer's (a Haste-only setting, no vanilla
/// counterpart to map).
pub(crate) fn premesh_mode(mode: haste_setup::options::PremeshMode) -> haste_render::PremeshMode {
    match mode {
        haste_setup::options::PremeshMode::Off => haste_render::PremeshMode::Off,
        haste_setup::options::PremeshMode::Cone => haste_render::PremeshMode::Cone,
        haste_setup::options::PremeshMode::Predict => haste_render::PremeshMode::Predict,
    }
}

/// `--premesh off|cone|predict` as the renderer's mode.
pub(crate) fn parse_premesh(value: &str) -> anyhow::Result<haste_render::PremeshMode> {
    haste_setup::options::PremeshMode::parse(value)
        .map(premesh_mode)
        .ok_or_else(|| {
            anyhow::anyhow!("--premesh must be `off`, `cone` or `predict`, got `{value}`")
        })
}

/// `--premesh` as the renderer's mode, when given (`None`: the saved option decides).
pub(crate) fn forced_premesh(
    args: &crate::Args,
) -> anyhow::Result<Option<haste_render::PremeshMode>> {
    args.premesh.as_deref().map(parse_premesh).transpose()
}

/// `--work-budget fixed|adaptive` as the renderer's mode.
pub(crate) fn parse_work_budget(value: &str) -> anyhow::Result<haste_render::WorkBudgetMode> {
    match value {
        "fixed" => Ok(haste_render::WorkBudgetMode::Fixed),
        "adaptive" => Ok(haste_render::WorkBudgetMode::Adaptive),
        other => anyhow::bail!("--work-budget must be `fixed` or `adaptive`, got `{other}`"),
    }
}

/// `--work-budget` as the renderer's mode, when given (`None`: `fixed`, the historical
/// behaviour — the lead benchmarks both modes from one binary).
pub(crate) fn forced_work_budget(
    args: &crate::Args,
) -> anyhow::Result<Option<haste_render::WorkBudgetMode>> {
    args.work_budget
        .as_deref()
        .map(parse_work_budget)
        .transpose()
}

/// A `WorldFeed` running `--work-budget`'s mode (the feed's uploads are the budgeted
/// work, so the feed carries the flag).
pub(crate) fn work_feed(args: &crate::Args) -> anyhow::Result<super::WorldFeed> {
    Ok(super::WorldFeed::with_work_budget(
        forced_work_budget(args)?.unwrap_or_default(),
    ))
}

/// The work budget's per-frame observation (call once per frame, before the renderer is
/// borrowed): the interval since the previous frame plus the previous frame's render and
/// feed times — the two make the CPU cost the uploads compete with. The frame-time
/// target comes from vsync's refresh interval (or the user's max FPS); while
/// unthrottled the rolling interval is the target.
pub(crate) fn work_budget_observation(
    frame_at: &mut std::time::Instant,
    render_ms: f32,
    feed_ms: f32,
    t0: std::time::Instant,
) -> (f32, f32, f32) {
    let interval_ms = (t0 - *frame_at).as_secs_f32() * 1e3;
    *frame_at = t0;
    (interval_ms, render_ms, feed_ms)
}
