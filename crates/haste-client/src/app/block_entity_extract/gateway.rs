/// The end gateway's beam (vanilla `TheEndGatewayRenderer.extractRenderState`): spawning
/// under age 200 in magenta to the build limit; the cooldown phase arrives as a block event
/// (not in the NBT), so it is not drawn yet. `None` hides the beam.
pub(super) fn gateway_beam_at(
    age: i64,
    partial: f32,
    max_y: i32,
) -> Option<haste_render::GatewayBeam> {
    if age >= 200 {
        return None;
    }
    // Vanilla `getSpawnPercent`: `clamp((age + partial) / 200, 0, 1)`, then `sin(scale * PI)`;
    // the beam distance is the build limit (`level.getMaxY()`), the height its floor.
    let scale = ((age as f32 + partial) / 200.0).clamp(0.0, 1.0);
    let scale = (scale * std::f32::consts::PI).sin();
    let height = (scale as f64 * max_y as f64).floor() as i32;
    Some(haste_render::GatewayBeam {
        scale,
        height,
        // Vanilla `DyeColor.MAGENTA.getTextureDiffuseColor()` while spawning.
        color: haste_render::blockentity::DYE_DIFFUSE_COLOR[2],
    })
}

/// The end gateway's `Age` from its NBT (vanilla `EndGatewayBlockEntity.loadAdditional`,
/// default 0).
pub(super) fn parse_gateway_age(nbt: &[u8]) -> i64 {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper)
        .ok()
        .and_then(|doc| doc.root_tag.get_long("Age"))
        .unwrap_or(0)
}
