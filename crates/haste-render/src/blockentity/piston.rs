// The moving piston (vanilla `PistonHeadRenderer`): the render data as vanilla's block entity
// keeps it, the interpolated progress, the state-string adjustments before baking and the
// moved block's quads resolved once and cached.

use std::sync::Arc;

use haste_core::{Direction, Identifier};

use crate::entity::TickPair;

/// A moving piston's render data, as vanilla's block entity keeps it: the moved block's state
/// string (from the NBT, full `block[facing=north,...]` form), the progress at the ends of the
/// last tick (vanilla ticks it 0.5 per tick to 1.0), and the direction flags. The renderer
/// resolves the state's quads once and caches them.
#[derive(Clone, Debug, PartialEq)]
pub struct PistonData {
    /// The moved block state as vanilla's NBT `blockState` string (adjusted by the renderer
    /// for the piston head's `short` and the base's `extended`, like `PistonHeadRenderer`
    /// does before baking).
    pub block_state: String,
    /// `progressO`/`progress`, the extension at the ends of the last tick.
    pub progress: TickPair,
    /// The piston's own `facing` (NBT `facing`, the direction the head moves to).
    pub facing: Direction,
    /// NBT `extending`.
    pub extending: bool,
    /// NBT `source` (this block entity is the piston base itself).
    pub source: bool,
}

/// Vanilla `PistonMovingBlockEntity.getProgress`: `lerp(partial, progressO, progress)` clamped
/// at partial 1.
pub fn piston_progress(p: &PistonData, partial: f32) -> f32 {
    p.progress.at(partial.min(1.0))
}

/// Vanilla `PistonMovingBlockEntity.getExtendedProgress`: `progress - 1` extending,
/// `1 - progress` retracting.
pub fn piston_extended_progress(p: &PistonData, progress: f32) -> f32 {
    if p.extending {
        progress - 1.0
    } else {
        1.0 - progress
    }
}

/// The moved block's state string after vanilla `PistonHeadRenderer.extractRenderState`'s
/// adjustments: a piston head picks `short` by the progress (<= 0.5), and the source piston's
/// retracting base draws `extended=true` with a head above it.
pub fn piston_drawn_states(p: &PistonData, progress: f32) -> (String, Option<String>) {
    let (block, props) = parse_state_string(&p.block_state);
    let mut props = props;
    let is_head = block.path() == "piston_head";
    let is_piston = matches!(block.path(), "piston" | "sticky_piston");
    if is_head && progress <= 4.0 {
        set_prop(&mut props, "short", &(progress <= 0.5).to_string());
        return (format_state(&block, &props), None);
    }
    if p.source && !p.extending && is_piston {
        // The head that retracts with the base (vanilla builds a piston_head state with the
        // piston's own type and facing, `short` by the progress).
        let sticky = block.path() == "sticky_piston";
        let facing = props
            .iter()
            .find(|(k, _)| k == "facing")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "north".into());
        let head = format_state(
            &Identifier::minecraft("piston_head"),
            &[
                ("facing".to_string(), facing.clone()),
                ("short".to_string(), (progress >= 0.5).to_string()),
                (
                    "type".to_string(),
                    if sticky { "sticky" } else { "normal" }.to_string(),
                ),
            ],
        );
        set_prop(&mut props, "extended", "true");
        return (format_state(&block, &props), Some(head));
    }
    (p.block_state.clone(), None)
}
/// Parses a vanilla block state string `block[facing=north,...]` into its id and properties.
pub(crate) fn parse_state_string(s: &str) -> (Identifier, Vec<(String, String)>) {
    let (name, props) = match s.split_once('[') {
        Some(n) => (n.0, n.1.trim_end_matches(']')),
        None => (s, ""),
    };
    let id = Identifier::parse(name).unwrap_or_else(|_| Identifier::minecraft("stone"));
    let props = props
        .split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    (id, props)
}

/// Formats a block id and properties back into vanilla's state string form.
fn format_state(block: &Identifier, props: &[(String, String)]) -> String {
    if props.is_empty() {
        return block.to_string();
    }
    let joined: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{}[{}]", block, joined.join(","))
}

fn set_prop(props: &mut Vec<(String, String)>, name: &str, value: &str) {
    if let Some(p) = props.iter_mut().find(|(k, _)| k == name) {
        p.1 = value.to_string();
    } else {
        props.push((name.to_string(), value.to_string()));
    }
}
/// One moved-block quad with its variant rotation: `(quad, x, y)` from the blockstate ref.
pub(crate) type PistonQuad = (haste_assets::BakedQuad, i32, i32);

/// The moved block's quads for a state string, resolved once and cached by the caller (the
/// states a piston ever moves are few; vanilla's model resolver caches the same way). Variant
/// x/y rotation is left to the emit matrix; each quad's texture resolves to its entity-atlas
/// rect per frame (`texture_rect` caches those itself).
pub(crate) fn piston_quads(
    assets: &mut haste_assets::Assets,
    state: &str,
) -> Option<Arc<Vec<PistonQuad>>> {
    let (block, props) = parse_state_string(state);
    let quads = (|| -> Option<Vec<PistonQuad>> {
        let def = assets.blockstate(&block).ok()?;
        let refs = def.model_refs_for(&props);
        let mut quads = Vec::new();
        for r in refs {
            let id = Identifier::parse(&r.model).ok()?;
            let q = assets.model_quads(&id).ok()?;
            for quad in q {
                quads.push((quad, r.x, r.y));
            }
        }
        Some(quads)
    })();
    quads.map(Arc::new)
}
