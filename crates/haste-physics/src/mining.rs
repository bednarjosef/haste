//! Mining speed: how much of a block one tick of breaking removes.
//!
//! Formula (minecraft.wiki "Breaking", speed calculation): the held tool's speed for the block
//! (its `minecraft:tool` component: first matching rule with a speed, else the default mining
//! speed); if that is above 1 the `mining_efficiency` attribute (Efficiency) is added; Haste and
//! Conduit Power multiply by 1 + 0.2 x level; Mining Fatigue by 0.3^min(level, 4); the
//! `block_break_speed` attribute multiplies; with the eyes under water the
//! `submerged_mining_speed` attribute multiplies (0.2 by default, Aqua Affinity raises it); off
//! the ground the speed is divided by 5. Damage per tick = speed / hardness / 30 when the block
//! can be harvested (it does not require the correct tool, or the tool's first matching
//! `correct_for_drops` rule says true), / 100 otherwise. Hardness -1 never breaks, 0 always
//! breaks at once; damage of 1 or more breaks the block on the first tick.
//!
//! Tool rules come from the data generator's item component reports
//! (`reports/minecraft/components/item/<item>.json`); block tags from the server (`update_tags`).
use std::collections::{HashMap, HashSet};
use std::path::Path;

use haste_core::Identifier;
use serde::Deserialize;

/// Which blocks a tool rule covers.
#[derive(Clone, Debug, PartialEq)]
pub enum BlockSet {
    Block(Identifier),
    Tag(Identifier),
    List(Vec<Identifier>),
}

impl BlockSet {
    fn parse(v: &serde_json::Value) -> Option<BlockSet> {
        match v {
            serde_json::Value::String(s) => Some(match s.strip_prefix('#') {
                Some(tag) => BlockSet::Tag(Identifier::parse(tag).ok()?),
                None => BlockSet::Block(Identifier::parse(s).ok()?),
            }),
            serde_json::Value::Array(a) => Some(BlockSet::List(
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
    pub blocks: BlockSet,
    pub speed: Option<f32>,
    pub correct_for_drops: Option<bool>,
}

/// A `minecraft:tool` component.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub rules: Vec<ToolRule>,
    pub default_mining_speed: f32,
    pub damage_per_block: i32,
    /// Swords and some weapons cannot break blocks in creative.
    pub can_destroy_blocks_in_creative: bool,
}

#[derive(Deserialize)]
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

impl Tool {
    /// Parses the JSON value of a `minecraft:tool` component.
    pub fn from_json(v: &serde_json::Value) -> anyhow::Result<Tool> {
        let raw: RawTool = serde_json::from_value(v.clone())?;
        let rules = raw
            .rules
            .iter()
            .filter_map(|r| {
                Some(ToolRule {
                    blocks: BlockSet::parse(r.get("blocks")?)?,
                    speed: r.get("speed").and_then(|s| s.as_f64()).map(|s| s as f32),
                    correct_for_drops: r.get("correct_for_drops").and_then(|c| c.as_bool()),
                })
            })
            .collect();
        Ok(Tool {
            rules,
            default_mining_speed: raw.default_mining_speed,
            damage_per_block: raw.damage_per_block,
            can_destroy_blocks_in_creative: raw.can_destroy_blocks_in_creative,
        })
    }

    /// Speed for a block and whether it is correct for drops (`None`: no rule says), given a
    /// predicate telling whether the block is in a [`BlockSet`].
    pub fn speed_for(&self, contains: impl Fn(&BlockSet) -> bool) -> (f32, Option<bool>) {
        let mut speed = None;
        let mut correct = None;
        for r in &self.rules {
            let wants = (speed.is_none() && r.speed.is_some())
                || (correct.is_none() && r.correct_for_drops.is_some());
            if wants && contains(&r.blocks) {
                speed = speed.or(r.speed);
                correct = correct.or(r.correct_for_drops);
            }
        }
        (speed.unwrap_or(self.default_mining_speed), correct)
    }
}

/// Default tool component per item, from the data generator's component reports.
#[derive(Default)]
pub struct ToolTable {
    by_item: HashMap<Identifier, Tool>,
    /// Items whose default components include `minecraft:glider` (vanilla
    /// `LivingEntity.canGlideUsing`'s `DataComponents.GLIDER`); the elytra and any mod item that
    /// carries the component.
    gliders: HashSet<Identifier>,
}

impl ToolTable {
    /// Reads `<reports>/minecraft/components/item/*.json`.
    pub fn load(reports: &Path) -> anyhow::Result<ToolTable> {
        let dir = reports.join("minecraft/components/item");
        let mut by_item = HashMap::new();
        let mut gliders = HashSet::new();
        for e in std::fs::read_dir(&dir).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))? {
            let path = e?.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
            let components = v.get("components");
            if let Some(t) = components.and_then(|c| c.get("minecraft:tool")) {
                by_item.insert(Identifier::minecraft(stem), Tool::from_json(t)?);
            }
            if components.is_some_and(|c| c.get("minecraft:glider").is_some()) {
                gliders.insert(Identifier::minecraft(stem));
            }
        }
        Ok(ToolTable { by_item, gliders })
    }

    /// From the default cache location (`haste_core::paths::datagen_reports`).
    pub fn load_default() -> anyhow::Result<ToolTable> {
        Self::load(&haste_core::paths::datagen_reports())
    }

    pub fn insert(&mut self, item: Identifier, tool: Tool) {
        self.by_item.insert(item, tool);
    }

    /// Records `item` as a glider (`minecraft:glider` in its default components).
    pub fn insert_glider(&mut self, item: Identifier) {
        self.gliders.insert(item);
    }

    /// Whether `item`'s default components carry the glider component.
    pub fn is_glider(&self, item: &Identifier) -> bool {
        self.gliders.contains(item)
    }

    /// The glider item names (for resolving them to registry indices).
    pub fn gliders(&self) -> &HashSet<Identifier> {
        &self.gliders
    }

    pub fn get(&self, item: &Identifier) -> Option<&Tool> {
        self.by_item.get(item)
    }

    pub fn len(&self) -> usize {
        self.by_item.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_item.is_empty()
    }
}

/// Block tags as the server synced them (`update_tags`, registry `minecraft:block`): tag name ->
/// block registry indices.
#[derive(Default, Clone, Debug)]
pub struct BlockTags {
    tags: HashMap<Identifier, HashSet<u32>>,
}

impl BlockTags {
    pub fn new(tags: impl IntoIterator<Item = (Identifier, Vec<i32>)>) -> BlockTags {
        BlockTags {
            tags: tags
                .into_iter()
                .map(|(n, e)| (n, e.into_iter().map(|i| i as u32).collect()))
                .collect(),
        }
    }

    pub fn contains(&self, tag: &Identifier, block_index: u32) -> bool {
        self.tags.get(tag).is_some_and(|s| s.contains(&block_index))
    }

    /// The block registry indices in `tag`, for callers that test many blocks against the same
    /// tag (resolve once, then look up without building an [`Identifier`] per block).
    pub fn members(&self, tag: &Identifier) -> Option<&HashSet<u32>> {
        self.tags.get(tag)
    }

    /// Whether the block (name and registry index) is in `set`.
    pub fn matches(&self, set: &BlockSet, block: &Identifier, block_index: u32) -> bool {
        match set {
            BlockSet::Block(b) => b == block,
            BlockSet::Tag(t) => self.contains(t, block_index),
            BlockSet::List(l) => l.contains(block),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }
}

/// Everything the breaking speed depends on, for one block and one tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiningParams {
    /// Block hardness (`destroy_time`): -1 unbreakable.
    pub hardness: f32,
    /// The block drops only with the correct tool.
    pub requires_correct_tool: bool,
    /// Tool speed for the block (1 without a tool or a matching rule).
    pub tool_speed: f32,
    /// The tool's first matching `correct_for_drops` rule.
    pub tool_correct: Option<bool>,
    /// `minecraft:mining_efficiency` (Efficiency: level^2 + 1).
    pub mining_efficiency: f64,
    /// Haste or Conduit Power level (amplifier + 1; the larger of the two), 0 if none.
    pub haste_level: u32,
    /// Mining Fatigue level (amplifier + 1), 0 if none.
    pub fatigue_level: u32,
    /// `minecraft:block_break_speed` (1).
    pub block_break_speed: f64,
    /// `minecraft:submerged_mining_speed` (0.2).
    pub submerged_mining_speed: f64,
    pub eyes_in_water: bool,
    pub on_ground: bool,
}

impl Default for MiningParams {
    fn default() -> Self {
        Self {
            hardness: 1.0,
            requires_correct_tool: false,
            tool_speed: 1.0,
            tool_correct: None,
            mining_efficiency: 0.0,
            haste_level: 0,
            fatigue_level: 0,
            block_break_speed: 1.0,
            submerged_mining_speed: 0.2,
            eyes_in_water: false,
            on_ground: true,
        }
    }
}

impl MiningParams {
    /// The block drops its items when broken with this tool.
    pub fn can_harvest(&self) -> bool {
        !self.requires_correct_tool || self.tool_correct == Some(true)
    }

    /// The breaking speed before dividing by hardness (wiki "Breaking", speed multiplier), in f32
    /// like the game's player destroy speed.
    pub fn speed(&self) -> f32 {
        let mut s = self.tool_speed;
        if s > 1.0 {
            s += self.mining_efficiency as f32;
        }
        if self.haste_level > 0 {
            s *= 1.0 + self.haste_level as f32 * 0.2;
        }
        if self.fatigue_level > 0 {
            s *= match self.fatigue_level {
                1 => 0.3,
                2 => 0.09,
                3 => 0.0027,
                _ => 8.1e-4,
            };
        }
        s *= self.block_break_speed as f32;
        if self.eyes_in_water {
            s *= self.submerged_mining_speed as f32;
        }
        if !self.on_ground {
            s /= 5.0;
        }
        s
    }

    /// Fraction of the block one tick of breaking removes (0 for unbreakable blocks).
    pub fn progress_per_tick(&self) -> f32 {
        if self.hardness < 0.0 || self.hardness.is_nan() {
            return 0.0;
        }
        if self.hardness == 0.0 {
            return 1.0;
        }
        let div = if self.can_harvest() { 30.0 } else { 100.0 };
        self.speed() / self.hardness / div
    }

    /// Ticks until the block breaks (1 = instant); `None` if it never does.
    pub fn ticks_to_break(&self) -> Option<u32> {
        let d = self.progress_per_tick();
        if d <= 0.0 {
            None
        } else if d >= 1.0 {
            Some(1)
        } else {
            Some((1.0 / d).ceil() as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pickaxe(speed: f32, incorrect_tag: &str) -> Tool {
        Tool::from_json(&serde_json::json!({"rules": [
            {"blocks": format!("#minecraft:{incorrect_tag}"), "correct_for_drops": false},
            {"blocks": "#minecraft:mineable/pickaxe", "correct_for_drops": true, "speed": speed}
        ]}))
        .unwrap()
    }

    fn tags() -> BlockTags {
        // stone = 1, obsidian = 2, dirt = 3
        BlockTags::new([
            (Identifier::minecraft("mineable/pickaxe"), vec![1, 2]),
            (Identifier::minecraft("incorrect_for_wooden_tool"), vec![2]),
            (Identifier::minecraft("incorrect_for_iron_tool"), vec![2]),
            (Identifier::minecraft("incorrect_for_diamond_tool"), vec![]),
        ])
    }

    fn params(
        tool: Option<&Tool>,
        block: &str,
        idx: u32,
        hardness: f32,
        req: bool,
    ) -> MiningParams {
        let t = tags();
        let id = Identifier::minecraft(block);
        let (tool_speed, tool_correct) = match tool {
            Some(tool) => tool.speed_for(|s| t.matches(s, &id, idx)),
            None => (1.0, None),
        };
        MiningParams {
            hardness,
            requires_correct_tool: req,
            tool_speed,
            tool_correct,
            ..Default::default()
        }
    }

    // Expected times: minecraft.wiki "Breaking" table (seconds = ticks / 20).
    #[test]
    fn stone_by_hand_and_pickaxes() {
        // Stone by hand: 7.5 s (not harvestable: /100).
        assert_eq!(
            params(None, "stone", 1, 1.5, true).ticks_to_break(),
            Some(150)
        );
        // Wooden pickaxe (speed 2): 1.15 s.
        let wood = pickaxe(2.0, "incorrect_for_wooden_tool");
        assert_eq!(
            params(Some(&wood), "stone", 1, 1.5, true).ticks_to_break(),
            Some(23)
        );
        // Diamond pickaxe (8): 0.3 s.
        let diamond = pickaxe(8.0, "incorrect_for_diamond_tool");
        assert_eq!(
            params(Some(&diamond), "stone", 1, 1.5, true).ticks_to_break(),
            Some(6)
        );
    }

    #[test]
    fn obsidian_wrong_tier_is_slow_and_not_harvestable() {
        let wood = pickaxe(2.0, "incorrect_for_wooden_tool");
        let p = params(Some(&wood), "obsidian", 2, 50.0, true);
        assert!(!p.can_harvest());
        assert_eq!(p.tool_speed, 2.0);
        // 2 / 50 / 100 per tick: 2500 ticks = 125 s (wiki: wooden pickaxe on obsidian 125 s).
        assert_eq!(p.ticks_to_break(), Some(2500));
        let diamond = pickaxe(8.0, "incorrect_for_diamond_tool");
        // 9.4 s with a diamond pickaxe.
        assert_eq!(
            params(Some(&diamond), "obsidian", 2, 50.0, true).ticks_to_break(),
            Some(188)
        );
        // An iron pickaxe is the wrong tier for obsidian: not correct for drops.
        let iron = pickaxe(6.0, "incorrect_for_iron_tool");
        let p = params(Some(&iron), "obsidian", 2, 50.0, true);
        assert!(!p.can_harvest());
        assert_eq!((p.tool_speed, p.tool_correct), (6.0, Some(false)));
    }

    #[test]
    fn dirt_by_hand_and_unbreakable_and_instant() {
        // Dirt by hand: 0.75 s.
        assert_eq!(
            params(None, "dirt", 3, 0.5, false).ticks_to_break(),
            Some(15)
        );
        assert_eq!(
            params(None, "bedrock", 9, -1.0, false).ticks_to_break(),
            None
        );
        assert_eq!(
            params(None, "short_grass", 9, 0.0, false).ticks_to_break(),
            Some(1)
        );
    }

    /// The item-data check's expected break ticks (vanilla `BlockBehaviour.getDestroyProgress`
    /// and `MultiPlayerGameMode.continueDestroyBlock`): progress accumulates in f32, one
    /// addition per tick starting on the press tick (`Minecraft.handleKeybinds` runs
    /// `startAttack` and `continueAttack` on the same tick). The counts include the press
    /// tick, so the finish tick is the count itself.
    #[test]
    fn item_data_expected_break_ticks() {
        let accumulate = |mut p: MiningParams| {
            p.mining_efficiency = 0.0;
            let d = p.progress_per_tick();
            let mut progress = 0.0f32;
            let mut ticks = 0;
            while progress < 1.0 {
                progress += d;
                ticks += 1;
                assert!(ticks < 10000, "never broke");
            }
            ticks
        };
        // Stone (hardness 1.5, needs a tool): by hand the wrong divisor 100 applies.
        let hand = MiningParams {
            hardness: 1.5,
            requires_correct_tool: true,
            tool_speed: 1.0,
            tool_correct: None,
            ..Default::default()
        };
        // f32 accumulation: 150 additions of 1/150 reach 1.0 only on the 151st.
        assert_eq!(accumulate(hand), 151);
        let pickaxe = |speed: f32| MiningParams {
            hardness: 1.5,
            requires_correct_tool: true,
            tool_speed: speed,
            tool_correct: Some(true),
            ..Default::default()
        };
        // Wooden pickaxe: 2/45 per tick; diamond: 8/45.
        assert_eq!(accumulate(pickaxe(2.0)), 23);
        assert_eq!(accumulate(pickaxe(8.0)), 6);
        // Efficiency V: 8 + 26 = 34 speed (Player.getDestroySpeed adds the attribute when
        // the tool speed is above 1).
        assert_eq!(accumulate(pickaxe(34.0)), 2);
        // Obsidian (hardness 50) with a diamond pickaxe: 8/1500 per tick.
        assert_eq!(
            accumulate(MiningParams {
                hardness: 50.0,
                requires_correct_tool: true,
                tool_speed: 8.0,
                tool_correct: Some(true),
                ..Default::default()
            }),
            188
        );
        // Cobweb (hardness 4) with a sword or shears: both carry a cobweb rule at speed 15
        // and correct_for_drops true, so 15/4/30 = 0.125 exactly: 8 ticks.
        let cobweb = MiningParams {
            hardness: 4.0,
            requires_correct_tool: true,
            tool_speed: 15.0,
            tool_correct: Some(true),
            ..Default::default()
        };
        assert_eq!(accumulate(cobweb), 8);
    }

    #[test]
    fn modifiers() {
        let diamond = pickaxe(8.0, "incorrect_for_diamond_tool");
        let base = params(Some(&diamond), "stone", 1, 1.5, true);
        // Efficiency V adds 26 to a fitting tool: (8 + 26) / 1.5 / 30 = 0.76 per tick: 2 ticks;
        // with Haste II on top it is instant (wiki "Breaking": instant mining needs speed > 45).
        let eff = MiningParams {
            mining_efficiency: 26.0,
            ..base
        };
        // Efficiency V adds level^2 + 1 = 26 to the fitting tool: 8 + 26 = 34.
        assert!((eff.speed() - 34.0).abs() < 1e-5);
        assert_eq!(eff.ticks_to_break(), Some(2));
        assert_eq!(
            MiningParams {
                haste_level: 2,
                ..eff
            }
            .ticks_to_break(),
            Some(1)
        );
        // ...but not to the hand (speed 1).
        let hand = MiningParams {
            mining_efficiency: 26.0,
            ..params(None, "dirt", 3, 0.5, false)
        };
        assert_eq!(hand.ticks_to_break(), Some(15));
        // Off the ground and under water: each /5.
        let air = MiningParams {
            on_ground: false,
            ..base
        };
        assert_eq!(air.ticks_to_break(), Some(29));
        let wet = MiningParams {
            eyes_in_water: true,
            ..base
        };
        assert_eq!(wet.ticks_to_break(), Some(29));
        // Haste II: x1.4; Mining Fatigue I: x0.3.
        assert!(
            (MiningParams {
                haste_level: 2,
                ..base
            }
            .speed()
                - 11.2)
                .abs()
                < 1e-5
        );
        assert!(
            (MiningParams {
                fatigue_level: 1,
                ..base
            }
            .speed()
                - 2.4)
                .abs()
                < 1e-5
        );
    }

    #[test]
    fn first_matching_rule_wins_separately_for_speed_and_drops() {
        let t = Tool::from_json(&serde_json::json!({
            "can_destroy_blocks_in_creative": false,
            "rules": [
                {"blocks": "minecraft:cobweb", "correct_for_drops": true, "speed": 15.0},
                {"blocks": ["minecraft:stone", "minecraft:dirt"], "speed": 1.5},
                {"blocks": "minecraft:stone", "speed": 9.0, "correct_for_drops": true}
            ]
        }))
        .unwrap();
        assert!(!t.can_destroy_blocks_in_creative);
        let stone = Identifier::minecraft("stone");
        let none = BlockTags::default();
        assert_eq!(
            t.speed_for(|s| none.matches(s, &stone, 1)),
            (1.5, Some(true))
        );
        let web = Identifier::minecraft("cobweb");
        assert_eq!(
            t.speed_for(|s| none.matches(s, &web, 2)),
            (15.0, Some(true))
        );
        let other = Identifier::minecraft("sand");
        assert_eq!(t.speed_for(|s| none.matches(s, &other, 3)), (1.0, None));
    }

    #[test]
    #[ignore = "needs cache/datagen/26.3/reports (data generator)"]
    fn loads_the_data_generator_tools() {
        let t = ToolTable::load_default().unwrap();
        let p = t.get(&Identifier::minecraft("diamond_pickaxe")).unwrap();
        assert_eq!(p.rules.len(), 2);
        assert_eq!(p.rules[1].speed, Some(8.0));
        let sword = t.get(&Identifier::minecraft("diamond_sword")).unwrap();
        assert!(!sword.can_destroy_blocks_in_creative);
        assert!(t.get(&Identifier::minecraft("stick")).is_none());
        assert!(t.len() > 30, "{} tools", t.len());
        // The elytra's default components carry `minecraft:glider`; a stick does not.
        assert!(t.is_glider(&Identifier::minecraft("elytra")));
        assert!(!t.is_glider(&Identifier::minecraft("stick")));
    }
}
