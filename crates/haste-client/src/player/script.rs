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
