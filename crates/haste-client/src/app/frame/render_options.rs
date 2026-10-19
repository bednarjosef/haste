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
