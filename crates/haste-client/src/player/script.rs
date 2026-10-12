//! Scripted input for headless movement tests against a server (`HASTE_WALK_SCRIPT=<file>`).
//!
//! One step per line, `#` starts a comment:
//! - `cmd <command>`: sends `/<command>` as `chat_command` (the bot must be an operator); 1 tick.
//! - `wait <ticks>`: no keys held.
//! - `hold <ticks> [forward|back|left|right|jump|sneak|sprint]... [yaw=<deg>] [pitch=<deg>]`.
//! - `look <yaw> <pitch>`: absolute look for the following steps (default 0 0); no tick.
//! - `fly`: toggles creative flight (double-tap jump); 1 tick.
//! - `mark <label>`: prints the player state with the label; no tick.
use haste_core::input::InputState;

use super::LocalPlayer;

/// One parsed step.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Command(String),
    Hold { ticks: u32, input: InputState },
    Look(f32, f32),
    Fly,
    Mark(String),
}

/// What the script wants for one tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptTick {
    /// Keys and absolute look (the caller subtracts the player's look offset).
    pub input: InputState,
    pub command: Option<String>,
    pub toggle_flight: bool,
    /// Labels to print before this tick.
    pub marks: Vec<String>,
}

/// A walk script being played back.
pub struct WalkScript {
    steps: Vec<Step>,
    next: usize,
    /// Ticks left in the current `hold`.
    remaining: u32,
    look: (f32, f32),
}

impl WalkScript {
    pub fn parse(text: &str) -> anyhow::Result<WalkScript> {
        let mut steps = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            let rest = rest.trim();
            let bad = |what: &str| anyhow::anyhow!("line {}: {what}: {line}", n + 1);
            steps.push(match word {
                "cmd" => Step::Command(rest.trim_start_matches('/').to_string()),
                "fly" => Step::Fly,
                "mark" => Step::Mark(rest.to_string()),
                "look" => {
                    let mut it = rest.split_whitespace().map(str::parse::<f32>);
                    match (it.next(), it.next()) {
                        (Some(Ok(y)), Some(Ok(p))) => Step::Look(y, p),
                        _ => return Err(bad("look needs <yaw> <pitch>")),
                    }
                }
                "wait" | "hold" => {
                    let mut it = rest.split_whitespace();
                    let ticks = it
                        .next()
                        .and_then(|t| t.parse().ok())
                        .ok_or_else(|| bad("needs a tick count"))?;
                    let mut input = InputState {
                        yaw: f32::NAN,
                        pitch: f32::NAN,
                        ..Default::default()
                    };
                    for w in it {
                        match w {
                            "forward" => input.forward = true,
                            "back" => input.back = true,
                            "left" => input.left = true,
                            "right" => input.right = true,
                            "jump" => input.jump = true,
                            "sneak" => input.sneak = true,
                            "sprint" => input.sprint = true,
                            _ => {
                                if let Some(v) = w.strip_prefix("yaw=") {
                                    input.yaw = v.parse().map_err(|_| bad("bad yaw"))?;
                                } else if let Some(v) = w.strip_prefix("pitch=") {
                                    input.pitch = v.parse().map_err(|_| bad("bad pitch"))?;
                                } else {
                                    return Err(bad("unknown key"));
                                }
                            }
                        }
                    }
                    Step::Hold { ticks, input }
                }
                _ => return Err(bad("unknown step")),
            });
        }
        Ok(WalkScript {
            steps,
            next: 0,
            remaining: 0,
            look: (0.0, 0.0),
        })
    }

    /// The next tick, or `None` when the script is over.
    pub fn next_tick(&mut self) -> Option<ScriptTick> {
        let mut t = ScriptTick {
            input: InputState {
                yaw: self.look.0,
                pitch: self.look.1,
                ..Default::default()
            },
            ..Default::default()
        };
        loop {
            if self.remaining > 0 {
                let Some(Step::Hold { input, .. }) = self.steps.get(self.next - 1) else {
                    unreachable!("remaining ticks belong to a hold step");
                };
                self.remaining -= 1;
                t.input = InputState {
                    yaw: if input.yaw.is_nan() {
                        self.look.0
                    } else {
                        input.yaw
                    },
                    pitch: if input.pitch.is_nan() {
                        self.look.1
                    } else {
                        input.pitch
                    },
                    ..*input
                };
                return Some(t);
            }
            let step = self.steps.get(self.next)?.clone();
            self.next += 1;
            match step {
                Step::Command(c) => {
                    t.command = Some(c);
                    return Some(t);
                }
                Step::Fly => {
                    // The second press of the double-tap: jump is down on this tick.
                    t.toggle_flight = true;
                    t.input.jump = true;
                    return Some(t);
                }
                Step::Mark(m) => t.marks.push(m),
                Step::Look(y, p) => {
                    self.look = (y, p);
                    t.input.yaw = y;
                    t.input.pitch = p;
                }
                Step::Hold { ticks, .. } => self.remaining = ticks,
            }
        }
    }
}

/// Plays a [`WalkScript`] into a [`LocalPlayer`] in headless mode and prints what happens:
/// marks, every teleport (the server's corrections show up here), health changes.
pub struct Runner {
    script: WalkScript,
    finished_at: Option<u64>,
    teleports: u64,
    health: f32,
}

impl Runner {
    /// The script named by `HASTE_WALK_SCRIPT`, if set.
    pub fn from_env() -> anyhow::Result<Option<Runner>> {
        let Some(path) = std::env::var_os("HASTE_WALK_SCRIPT") else {
            return Ok(None);
        };
        Ok(Some(Runner {
            script: WalkScript::parse(&std::fs::read_to_string(&path)?)?,
            finished_at: None,
            teleports: 0,
            health: 20.0,
        }))
    }

    /// This tick's input (look angles already corrected by the player's look offset) and the
    /// command packet to send, if any.
    pub fn before_tick(
        &mut self,
        tick: u64,
        player: &mut LocalPlayer,
    ) -> (InputState, Option<haste_protocol::ServerboundPlay>) {
        if player.health <= 0.0 && player.has_position {
            // Dead: press "Respawn" (client_command 0, perform respawn) and wait for it.
            println!("[tick {tick}] dead, respawning");
            player.health = 20.0;
            return (
                InputState::default(),
                Some(haste_protocol::ServerboundPlay::ClientCommand(
                    haste_protocol::packets::play::serverbound::ClientCommand { action: 0 },
                )),
            );
        }
        let Some(t) = self.script.next_tick() else {
            if self.finished_at.is_none() {
                println!("[tick {tick}] script finished");
                self.finished_at = Some(tick);
            }
            return (InputState::default(), None);
        };
        for m in &t.marks {
            let s = &player.state;
            println!(
                "[tick {tick}] mark {m}: pos ({:.5}, {:.5}, {:.5}) v ({:.4}, {:.4}, {:.4}) ground {} sprint {} sneak {} fly {} water {} health {} air {} fall {:.3}",
                s.pos.x, s.pos.y, s.pos.z, s.velocity.x, s.velocity.y, s.velocity.z, s.on_ground,
                s.sprinting, s.sneaking, s.flying, s.in_water, player.health, player.air_supply, s.fall_distance,
            );
        }
        if t.toggle_flight {
            player.toggle_flight();
        }
        let (dy, dp) = player.look_offset();
        let mut input = t.input;
        input.yaw -= dy;
        input.pitch -= dp;
        let cmd = t.command.map(|command| {
            haste_protocol::ServerboundPlay::ChatCommand(
                haste_protocol::packets::play::serverbound::ChatCommand { command },
            )
        });
        (input, cmd)
    }

    /// Prints teleports and health changes; true once the script is over (plus 2 s to settle).
    pub fn after_tick(&mut self, tick: u64, player: &LocalPlayer) -> bool {
        if player.health != self.health {
            self.health = player.health;
            println!("[tick {tick}] health {}", self.health);
        }
        if player.teleports != self.teleports {
            self.teleports = player.teleports;
            let p = player.state.pos;
            println!(
                "[tick {tick}] teleport #{} to ({:.5}, {:.5}, {:.5})",
                self.teleports, p.x, p.y, p.z
            );
        }
        self.finished_at.is_some_and(|t| tick >= t + 40)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_plays_back() {
        let mut s = WalkScript::parse(
            "# test\nlook 90 10\nmark start\nhold 2 forward sprint\ncmd /tp @s 0 0 0\nhold 1 jump yaw=45\nfly\nwait 1\n",
        )
        .unwrap();
        let t = s.next_tick().unwrap();
        assert_eq!(t.marks, ["start"]);
        assert!(t.input.forward && t.input.sprint);
        assert_eq!((t.input.yaw, t.input.pitch), (90.0, 10.0));
        assert!(s.next_tick().unwrap().input.forward);
        let t = s.next_tick().unwrap();
        assert_eq!(t.command.as_deref(), Some("tp @s 0 0 0"));
        assert!(!t.input.forward);
        let t = s.next_tick().unwrap();
        assert!(t.input.jump);
        assert_eq!((t.input.yaw, t.input.pitch), (45.0, 10.0));
        assert!(s.next_tick().unwrap().toggle_flight);
        let t = s.next_tick().unwrap();
        assert_eq!(t.input.yaw, 90.0);
        assert!(s.next_tick().is_none());
    }

    #[test]
    fn rejects_unknown_steps() {
        assert!(WalkScript::parse("dance 3").is_err());
        assert!(WalkScript::parse("hold x").is_err());
        assert!(WalkScript::parse("hold 3 flap").is_err());
    }
}
