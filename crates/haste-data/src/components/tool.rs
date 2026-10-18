//! The `minecraft:tool` item component (vanilla `ToolComponent`).

use haste_core::Identifier;

/// The `blocks` predicate of a `minecraft:tool` rule, in the report's own shape.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolBlocks {
    /// `minecraft:stone` — one block.
    Block(Identifier),
    /// `#minecraft:mineable/pickaxe` — a block tag.
    Tag(Identifier),
    /// A JSON array of block names.
    List(Vec<Identifier>),
}

impl ToolBlocks {
    fn parse(v: &serde_json::Value) -> Option<ToolBlocks> {
        match v {
            serde_json::Value::String(s) => Some(match s.strip_prefix('#') {
                Some(tag) => ToolBlocks::Tag(Identifier::parse(tag).ok()?),
                None => ToolBlocks::Block(Identifier::parse(s).ok()?),
            }),
            serde_json::Value::Array(a) => Some(ToolBlocks::List(
                a.iter()
                    .filter_map(|e| e.as_str().and_then(|s| Identifier::parse(s).ok()))
                    .collect(),
            )),
            _ => None,
        }
    }
}

/// One rule of a `minecraft:tool` component.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRule {
    pub blocks: ToolBlocks,
    pub speed: Option<f32>,
    pub correct_for_drops: Option<bool>,
}

/// The `minecraft:tool` component of one item (vanilla `ToolComponent`), parsed from the report;
/// physics turns it into its mining `Tool`.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolComponent {
    pub rules: Vec<ToolRule>,
    pub default_mining_speed: f32,
    pub damage_per_block: i32,
    /// Swords and some weapons cannot break blocks in creative.
    pub can_destroy_blocks_in_creative: bool,
}

impl ToolComponent {
    /// Parses the JSON value of a `minecraft:tool` component; `None` when malformed. One
    /// unparseable rule fails the whole component, as vanilla's codec does (a decode error means
    /// the component is absent and `getOrDefault` falls back to no tool).
    pub fn from_json(v: &serde_json::Value) -> Option<ToolComponent> {
        let raw: RawTool = serde_json::from_value(v.clone()).ok()?;
        let rules = raw
            .rules
            .iter()
            .map(|r| {
                Some(ToolRule {
                    blocks: ToolBlocks::parse(r.get("blocks")?)?,
                    speed: r.get("speed").and_then(|s| s.as_f64()).map(|s| s as f32),
                    correct_for_drops: r.get("correct_for_drops").and_then(|c| c.as_bool()),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(ToolComponent {
            rules,
            default_mining_speed: raw.default_mining_speed,
            damage_per_block: raw.damage_per_block,
            can_destroy_blocks_in_creative: raw.can_destroy_blocks_in_creative,
        })
    }
}

#[derive(serde::Deserialize)]
struct RawTool {
    #[serde(default)]
    rules: Vec<serde_json::Value>,
    #[serde(default = "one")]
    default_mining_speed: f32,
    #[serde(default = "one_i")]
    damage_per_block: i32,
    #[serde(default = "yes")]
    can_destroy_blocks_in_creative: bool,
}

fn one() -> f32 {
    1.0
}

fn one_i() -> i32 {
    1
}

fn yes() -> bool {
    true
}
