//! Input scripts for the automated offscreen client: the script format and the step runner.
//!
//! A script is a list of steps, one per line. `#` starts a comment and blank lines are ignored.
//! `parse` turns the text into `Vec<Step>`; a [`Runner`] drives those steps and returns the
//! [`Action`]s the client must perform each game tick (20 ticks per second). This module is pure
//! logic only: no window, renderer or network; the `--script FILE` offscreen client wires it up
//! (app.rs `script_run`).
//!
//! Mouse buttons (`click`, `mouse_down`, `mouse_up`) are `left`, `middle` or `right`; a `click`
//! is a press and a release on the same tick, while `mouse_down`/`mouse_up` hold a button (e.g.
//! attack while the target is aimed). Key names (`key`, `key_down`, `key_up`) are
//! case-insensitive: letters `A`..`Z`, digits
//! `0`..`9`, function keys `F1`..`F12`, and `Escape`, `Enter`, `Tab`, `Space`, `Backspace`,
//! `Up`, `Down`, `Left`, `Right`, `LShift`/`LeftShift`, `LCtrl`/`LeftCtrl`, `LAlt`/`LeftAlt`.

use std::fmt;

/// The keys a script can name. Kept explicit so the names map one-to-one onto the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Escape,
    Enter,
    Tab,
    Space,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    LeftShift,
    LeftCtrl,
    LeftAlt,
}

impl Key {
    /// Parses a key name as written in a script. Case-insensitive, so `Escape`, `escape` and
    /// `ESCAPE` are the same key: digits are `0`..`9`, letters `A`..`Z`, the rest are the
    /// obvious words (`F1`..`F12`, `Escape`, `Enter`, `Tab`, `Space`, `Backspace`, `Up`, `Down`,
    /// `Left`, `Right`, `LShift`/`LeftShift`, `LCtrl`/`LeftCtrl`, `LAlt`/`LeftAlt`).
    /// Returns `None` for anything else.
    pub fn from_name(name: &str) -> Option<Key> {
        use Key::*;
        Some(match name.to_ascii_uppercase().as_str() {
            "A" => A,
            "B" => B,
            "C" => C,
            "D" => D,
            "E" => E,
            "F" => F,
            "G" => G,
            "H" => H,
            "I" => I,
            "J" => J,
            "K" => K,
            "L" => L,
            "M" => M,
            "N" => N,
            "O" => O,
            "P" => P,
            "Q" => Q,
            "R" => R,
            "S" => S,
            "T" => T,
            "U" => U,
            "V" => V,
            "W" => W,
            "X" => X,
            "Y" => Y,
            "Z" => Z,
            "0" => Num0,
            "1" => Num1,
            "2" => Num2,
            "3" => Num3,
            "4" => Num4,
            "5" => Num5,
            "6" => Num6,
            "7" => Num7,
            "8" => Num8,
            "9" => Num9,
            "F1" => F1,
            "F2" => F2,
            "F3" => F3,
            "F4" => F4,
            "F5" => F5,
            "F6" => F6,
            "F7" => F7,
            "F8" => F8,
            "F9" => F9,
            "F10" => F10,
            "F11" => F11,
            "F12" => F12,
            "ESCAPE" => Escape,
            "ENTER" => Enter,
            "TAB" => Tab,
            "SPACE" => Space,
            "BACKSPACE" => Backspace,
            "UP" => Up,
            "DOWN" => Down,
            "LEFT" => Left,
            "RIGHT" => Right,
            "LSHIFT" | "LEFTSHIFT" => LeftShift,
            "LCTRL" | "LEFTCTRL" => LeftCtrl,
            "LALT" | "LEFTALT" => LeftAlt,
            _ => return None,
        })
    }

    /// The script name for this key, the inverse of [`Key::from_name`].
    pub fn name(&self) -> &'static str {
        use Key::*;
        match self {
            A => "A",
            B => "B",
            C => "C",
            D => "D",
            E => "E",
            F => "F",
            G => "G",
            H => "H",
            I => "I",
            J => "J",
            K => "K",
            L => "L",
            M => "M",
            N => "N",
            O => "O",
            P => "P",
            Q => "Q",
            R => "R",
            S => "S",
            T => "T",
            U => "U",
            V => "V",
            W => "W",
            X => "X",
            Y => "Y",
            Z => "Z",
            Num0 => "0",
            Num1 => "1",
            Num2 => "2",
            Num3 => "3",
            Num4 => "4",
            Num5 => "5",
            Num6 => "6",
            Num7 => "7",
            Num8 => "8",
            Num9 => "9",
            F1 => "F1",
            F2 => "F2",
            F3 => "F3",
            F4 => "F4",
            F5 => "F5",
            F6 => "F6",
            F7 => "F7",
            F8 => "F8",
            F9 => "F9",
            F10 => "F10",
            F11 => "F11",
            F12 => "F12",
            Escape => "ESCAPE",
            Enter => "ENTER",
            Tab => "TAB",
            Space => "SPACE",
            Backspace => "BACKSPACE",
            Up => "UP",
            Down => "DOWN",
            Left => "LEFT",
            Right => "RIGHT",
            LeftShift => "LSHIFT",
            LeftCtrl => "LCTRL",
            LeftAlt => "LALT",
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A mouse button for `click`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn parse(name: &str, line: usize) -> Result<MouseButton, ScriptError> {
        match name {
            "left" => Ok(MouseButton::Left),
            "right" => Ok(MouseButton::Right),
            "middle" => Ok(MouseButton::Middle),
            _ => Err(ScriptError::UnknownButton {
                name: name.to_owned(),
                line,
            }),
        }
    }
}

/// A `wait_until` condition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Condition {
    Joined,
    Settled,
    ScreenOpen,
    ScreenClosed,
    /// The open screen's class simple name is one of these (`TitleScreen`,
    /// `JoinMultiplayerScreen`, ...; `screen A|B` waits for either); no screen open matches
    /// nothing.
    Screen(Vec<String>),
}

impl Condition {
    /// Parses the arguments of a `wait_until` line: one of the four named conditions, or
    /// `screen <SimpleName>` (alternatives separated by `|`).
    fn parse(rest: &str, line: usize) -> Result<Condition, ScriptError> {
        let name = rest.split_whitespace().next().unwrap_or_default();
        match name {
            "joined" => Ok(Condition::Joined),
            "settled" => Ok(Condition::Settled),
            "screen_open" => Ok(Condition::ScreenOpen),
            "screen_closed" => Ok(Condition::ScreenClosed),
            "screen" => {
                let class = rest.split_whitespace().nth(1).ok_or_else(|| {
                    ScriptError::UnknownCondition {
                        name: rest.to_owned(),
                        line,
                    }
                })?;
                Ok(Condition::Screen(
                    class
                        .split('|')
                        .map(|c| c.rsplit('.').next().unwrap_or(c).to_owned())
                        .collect(),
                ))
            }
            _ => Err(ScriptError::UnknownCondition {
                name: name.to_owned(),
                line,
            }),
        }
    }

    fn matches(&self, state: &ScriptState) -> bool {
        match self {
            Condition::Joined => state.joined,
            Condition::Settled => state.settled,
            Condition::ScreenOpen => state.screen_open,
            Condition::ScreenClosed => !state.screen_open,
            Condition::Screen(want) => state.screen_class.as_deref().is_some_and(|open| {
                open.rsplit('.')
                    .next()
                    .is_some_and(|n| want.iter().any(|w| w == n))
            }),
        }
    }

    fn name(&self) -> String {
        match self {
            Condition::Joined => "joined".into(),
            Condition::Settled => "settled".into(),
            Condition::ScreenOpen => "screen_open".into(),
            Condition::ScreenClosed => "screen_closed".into(),
            Condition::Screen(s) => format!("screen {}", s.join("|")),
        }
    }
}

/// One parsed script step.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Idle for `n` game ticks before the next step.
    Wait(u32),
    /// Block until `condition` holds (or the timeout fires). `line` names the source line for the
    /// timeout error.
    WaitUntil { condition: Condition, line: usize },
    /// Press and release a key: a down this tick, an up the next.
    Key(Key),
    /// Press a key and hold it.
    KeyDown(Key),
    /// Release a held key.
    KeyUp(Key),
    /// Type text.
    Text(String),
    /// Move the mouse to GUI coordinates.
    MouseMove(i32, i32),
    /// Click a button at GUI coordinates.
    Click(MouseButton, i32, i32),
    /// Click the widget whose label is `text`, at the label's position plus `(dx, dy)`. The label
    /// is matched exactly, as `text: value` (a slider's dynamic label) or as one of `|`
    /// alternatives; the offset lets a script hit a slider's track rather than its centred
    /// label.
    ClickText(String, i32, i32),
    /// Press and hold a mouse button (attack/use while a screen is closed).
    MouseDown(MouseButton),
    /// Release a held mouse button.
    MouseUp(MouseButton),
    /// Turn the wheel by `n` notches (may be negative).
    Scroll(i32),
    /// Set the camera yaw and pitch in degrees.
    Look(f32, f32),
    /// Take a screenshot named `name`.
    Screenshot(String),
}

/// What the client must do on one game tick.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    KeyDown(Key),
    KeyUp(Key),
    Text(String),
    MouseMove(i32, i32),
    Click(MouseButton, i32, i32),
    ClickText(String, i32, i32),
    MouseDown(MouseButton),
    MouseUp(MouseButton),
    Scroll(i32),
    Look(f32, f32),
    Screenshot(String),
    /// A `wait_until` timed out; the message names the source line.
    Error(String),
    /// The script is done.
    Finished,
}

/// The per-tick game state the runner reads for `wait_until`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScriptState {
    pub joined: bool,
    pub settled: bool,
    pub screen_open: bool,
    /// The open screen's class name (`wait_until screen <SimpleName>`), if any.
    pub screen_class: Option<String>,
}

/// A parse error. Every variant carries the 1-based source line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    UnknownCommand { command: String, line: usize },
    BadKeyName { name: String, line: usize },
    BadNumber { text: String, line: usize },
    UnknownCondition { name: String, line: usize },
    UnknownButton { name: String, line: usize },
    MissingArgument { command: &'static str, line: usize },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::UnknownCommand { command, line } => {
                write!(f, "line {line}: unknown command `{command}`")
            }
            ScriptError::BadKeyName { name, line } => {
                write!(f, "line {line}: unknown key `{name}`")
            }
            ScriptError::BadNumber { text, line } => {
                write!(f, "line {line}: not a number `{text}`")
            }
            ScriptError::UnknownCondition { name, line } => {
                write!(f, "line {line}: unknown condition `{name}`")
            }
            ScriptError::UnknownButton { name, line } => {
                write!(f, "line {line}: unknown button `{name}`")
            }
            ScriptError::MissingArgument { command, line } => {
                write!(f, "line {line}: `{command}` needs an argument")
            }
        }
    }
}

impl std::error::Error for ScriptError {}

/// `wait_until` gives up after this many ticks (default 600 ticks = 30 seconds).
const WAIT_UNTIL_TIMEOUT: u32 = 600;

/// Parses a whole script into steps. Blank lines and `# ...` comments are ignored; whitespace
/// around a command is trimmed.
pub fn parse(text: &str) -> Result<Vec<Step>, ScriptError> {
    let mut steps = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = idx + 1;
        // Strip the comment, then trim. A `#` inside a `type` line therefore starts a comment too.
        let content = match raw.find('#') {
            Some(at) => &raw[..at],
            None => raw,
        }
        .trim();
        if content.is_empty() {
            continue;
        }
        let (command, rest) = match content.split_once(char::is_whitespace) {
            Some((command, rest)) => (command, rest.trim_start()),
            None => (content, ""),
        };
        steps.push(match command {
            "wait" => {
                let n = first_token(rest, "wait", line)?;
                Step::Wait(parse_u32(n, line)?)
            }
            "wait_until" => {
                let condition = Condition::parse(rest, line)?;
                Step::WaitUntil { condition, line }
            }
            "key" => Step::Key(parse_key(first_token(rest, "key", line)?, line)?),
            "key_down" => Step::KeyDown(parse_key(first_token(rest, "key_down", line)?, line)?),
            "key_up" => Step::KeyUp(parse_key(first_token(rest, "key_up", line)?, line)?),
            "type" => Step::Text(rest.to_owned()),
            "mouse" => {
                let mut parts = rest.split_whitespace();
                let x = parts.next().ok_or_else(|| missing("mouse", line))?;
                let y = parts.next().ok_or_else(|| missing("mouse", line))?;
                Step::MouseMove(parse_i32(x, line)?, parse_i32(y, line)?)
            }
            "click" => {
                let mut parts = rest.split_whitespace();
                let button = parts.next().ok_or_else(|| missing("click", line))?;
                let x = parts.next().ok_or_else(|| missing("click", line))?;
                let y = parts.next().ok_or_else(|| missing("click", line))?;
                let button = MouseButton::parse(button, line)?;
                Step::Click(button, parse_i32(x, line)?, parse_i32(y, line)?)
            }
            "click_text" => {
                // A quoted label may carry `dx dy` after it (a slider's track); an unquoted
                // one is the whole rest of the line (multi-word labels, no offsets).
                let quoted = rest.trim_start().starts_with('"');
                let (text, rest) = parse_quoted(rest, "click_text", line)?;
                if quoted {
                    let mut parts = rest.split_whitespace();
                    let dx = match parts.next() {
                        Some(s) => parse_i32(s, line)?,
                        None => 0,
                    };
                    let dy = match parts.next() {
                        Some(s) => parse_i32(s, line)?,
                        None => 0,
                    };
                    Step::ClickText(text, dx, dy)
                } else {
                    // Re-join the words `parse_quoted` split off (it takes the first word only).
                    let label = match rest.is_empty() {
                        true => text,
                        false => format!("{text} {rest}"),
                    };
                    Step::ClickText(label, 0, 0)
                }
            }
            "mouse_down" => Step::MouseDown(MouseButton::parse(
                first_token(rest, "mouse_down", line)?,
                line,
            )?),
            "mouse_up" => Step::MouseUp(MouseButton::parse(
                first_token(rest, "mouse_up", line)?,
                line,
            )?),
            "scroll" => Step::Scroll(parse_i32(first_token(rest, "scroll", line)?, line)?),
            "look" => {
                let mut parts = rest.split_whitespace();
                let yaw = parts.next().ok_or_else(|| missing("look", line))?;
                let pitch = parts.next().ok_or_else(|| missing("look", line))?;
                Step::Look(parse_f32(yaw, line)?, parse_f32(pitch, line)?)
            }
            "screenshot" => Step::Screenshot(first_token(rest, "screenshot", line)?.to_owned()),
            _ => {
                return Err(ScriptError::UnknownCommand {
                    command: command.to_owned(),
                    line,
                })
            }
        });
    }
    Ok(steps)
}

fn missing(command: &'static str, line: usize) -> ScriptError {
    ScriptError::MissingArgument { command, line }
}

fn first_token<'a>(
    rest: &'a str,
    command: &'static str,
    line: usize,
) -> Result<&'a str, ScriptError> {
    rest.split_whitespace()
        .next()
        .ok_or_else(|| missing(command, line))
}

/// The first argument of a command: a `"quoted string"` (so it may contain spaces and a `:`) or a
/// single bare word. Returns it and the rest of the line.
fn parse_quoted<'a>(
    rest: &'a str,
    command: &'static str,
    line: usize,
) -> Result<(String, &'a str), ScriptError> {
    let rest = rest.trim_start();
    let Some(after) = rest.strip_prefix('"') else {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let word = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| missing(command, line))?;
        return Ok((word.to_owned(), parts.next().unwrap_or("")));
    };
    let end = after
        .find('"')
        .ok_or(ScriptError::MissingArgument { command, line })?;
    Ok((after[..end].to_owned(), &after[end + 1..]))
}

fn parse_key(name: &str, line: usize) -> Result<Key, ScriptError> {
    Key::from_name(name).ok_or_else(|| ScriptError::BadKeyName {
        name: name.to_owned(),
        line,
    })
}

fn parse_u32(text: &str, line: usize) -> Result<u32, ScriptError> {
    text.parse::<u32>().map_err(|_| ScriptError::BadNumber {
        text: text.to_owned(),
        line,
    })
}

fn parse_i32(text: &str, line: usize) -> Result<i32, ScriptError> {
    text.parse::<i32>().map_err(|_| ScriptError::BadNumber {
        text: text.to_owned(),
        line,
    })
}

fn parse_f32(text: &str, line: usize) -> Result<f32, ScriptError> {
    text.parse::<f32>().map_err(|_| ScriptError::BadNumber {
        text: text.to_owned(),
        line,
    })
}

/// Where the runner is inside the current step.
enum Progress {
    /// No intra-step work in flight; execute `steps[idx]` fresh.
    Fresh,
    /// A `key` was pressed this tick; release it the next.
    Held(Key),
    /// Idling: this many more ticks after the current one.
    Waiting(u32),
    /// Waiting for a condition; `elapsed` ticks checked so far (for the timeout).
    Until {
        condition: Condition,
        elapsed: u32,
        line: usize,
    },
}

/// Drives a parsed script, producing the [`Action`]s due on each game tick.
pub struct Runner {
    steps: Vec<Step>,
    idx: usize,
    progress: Progress,
    finished: bool,
}

impl Runner {
    pub fn new(steps: Vec<Step>) -> Self {
        Runner {
            idx: 0,
            finished: steps.is_empty(),
            progress: Progress::Fresh,
            steps,
        }
    }

    /// Whether the script has finished (an `Action::Finished` also reports it; this is the
    /// query form, used by tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Returns the actions due on this tick. Idle ticks return an empty vector (no allocation).
    pub fn tick(&mut self, state: &ScriptState) -> Vec<Action> {
        loop {
            if self.finished {
                return vec![Action::Finished];
            }
            match std::mem::replace(&mut self.progress, Progress::Fresh) {
                Progress::Held(key) => {
                    self.advance();
                    return vec![Action::KeyUp(key)];
                }
                Progress::Waiting(remaining) => {
                    if remaining == 0 {
                        self.advance();
                        continue;
                    }
                    self.progress = Progress::Waiting(remaining - 1);
                    return Vec::new();
                }
                Progress::Until {
                    condition,
                    elapsed,
                    line,
                } => {
                    if condition.matches(state) {
                        self.advance();
                        continue;
                    }
                    let elapsed = elapsed + 1;
                    if elapsed >= WAIT_UNTIL_TIMEOUT {
                        self.finished = true;
                        return vec![Action::Error(format!(
                            "line {line}: `wait_until {}` timed out after {WAIT_UNTIL_TIMEOUT} ticks",
                            condition.name()
                        ))];
                    }
                    self.progress = Progress::Until {
                        condition,
                        elapsed,
                        line,
                    };
                    return Vec::new();
                }
                Progress::Fresh => {
                    let step = self.steps[self.idx].clone();
                    match step {
                        Step::Wait(n) => {
                            if n == 0 {
                                self.advance();
                                continue;
                            }
                            self.progress = Progress::Waiting(n - 1);
                            return Vec::new();
                        }
                        Step::WaitUntil { condition, line } => {
                            self.progress = Progress::Until {
                                condition,
                                elapsed: 0,
                                line,
                            };
                            continue;
                        }
                        Step::Key(key) => {
                            self.progress = Progress::Held(key);
                            return vec![Action::KeyDown(key)];
                        }
                        Step::KeyDown(key) => {
                            self.advance();
                            return vec![Action::KeyDown(key)];
                        }
                        Step::KeyUp(key) => {
                            self.advance();
                            return vec![Action::KeyUp(key)];
                        }
                        Step::Text(text) => {
                            self.advance();
                            return vec![Action::Text(text)];
                        }
                        Step::MouseMove(x, y) => {
                            self.advance();
                            return vec![Action::MouseMove(x, y)];
                        }
                        Step::Click(button, x, y) => {
                            self.advance();
                            return vec![Action::Click(button, x, y)];
                        }
                        Step::ClickText(text, dx, dy) => {
                            self.advance();
                            return vec![Action::ClickText(text, dx, dy)];
                        }
                        Step::MouseDown(button) => {
                            self.advance();
                            return vec![Action::MouseDown(button)];
                        }
                        Step::MouseUp(button) => {
                            self.advance();
                            return vec![Action::MouseUp(button)];
                        }
                        Step::Scroll(n) => {
                            self.advance();
                            return vec![Action::Scroll(n)];
                        }
                        Step::Look(yaw, pitch) => {
                            self.advance();
                            return vec![Action::Look(yaw, pitch)];
                        }
                        Step::Screenshot(name) => {
                            self.advance();
                            return vec![Action::Screenshot(name)];
                        }
                    }
                }
            }
        }
    }

    fn advance(&mut self) {
        self.idx += 1;
        self.progress = Progress::Fresh;
        if self.idx >= self.steps.len() {
            self.finished = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_every_command() {
        let script = [
            "wait 40",
            "wait_until settled",
            "key T",
            "key_down LSHIFT",
            "key_up LCTRL",
            "type hello world",
            "mouse 160 90",
            "click right 10 20",
            "click_text Multiplayer",
            "click_text \"FOV\" 90 5",
            "click_text Video",
            "mouse_down left",
            "mouse_up left",
            "scroll -1",
            "look 90 10",
            "screenshot name1",
        ]
        .join("\n");
        let steps = parse(&script).unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Wait(40),
                Step::WaitUntil {
                    condition: Condition::Settled,
                    line: 2,
                },
                Step::Key(Key::T),
                Step::KeyDown(Key::LeftShift),
                Step::KeyUp(Key::LeftCtrl),
                Step::Text("hello world".to_owned()),
                Step::MouseMove(160, 90),
                Step::Click(MouseButton::Right, 10, 20),
                Step::ClickText("Multiplayer".to_owned(), 0, 0),
                Step::ClickText("FOV".to_owned(), 90, 5),
                Step::ClickText("Video".to_owned(), 0, 0),
                Step::MouseDown(MouseButton::Left),
                Step::MouseUp(MouseButton::Left),
                Step::Scroll(-1),
                Step::Look(90.0, 10.0),
                Step::Screenshot("name1".to_owned()),
            ]
        );
        // The screen condition takes a class simple name (the package part is trimmed), or
        // several separated by `|` (either matches).
        let steps = parse("wait_until screen multiplayer.JoinMultiplayerScreen").unwrap();
        assert_eq!(
            steps,
            vec![Step::WaitUntil {
                condition: Condition::Screen(vec!["JoinMultiplayerScreen".to_owned()]),
                line: 1,
            }]
        );
        let steps = parse("wait_until screen TitleScreen|CreateWorldScreen").unwrap();
        assert_eq!(
            steps,
            vec![Step::WaitUntil {
                condition: Condition::Screen(vec![
                    "TitleScreen".to_owned(),
                    "CreateWorldScreen".to_owned()
                ]),
                line: 1,
            }]
        );
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let script =
            "# header\n\n   \nwait 1  # trailing comment\n   # spaces then comment\nwait 0\n";
        let steps = parse(script).unwrap();
        assert_eq!(steps, vec![Step::Wait(1), Step::Wait(0)]);
    }

    #[test]
    fn parse_errors_carry_line_numbers() {
        let cases: &[(&str, &str)] = &[
            ("frobnicate", "line 1"),
            ("\n\nkey NOTAKEY", "line 3"),
            ("wait 1\nscroll abc", "line 2"),
            ("\nwait_until sideways", "line 2"),
            ("\nclick sideways 1 2", "line 2"),
            ("\n\nscreenshot", "line 3"),
            ("mouse 5", "line 1"),
        ];
        for (script, want) in cases {
            let err = parse(script).unwrap_err();
            assert!(
                err.to_string().contains(want),
                "{script:?}: {err} missing {want:?}"
            );
        }
        assert!(matches!(
            parse("frobnicate").unwrap_err(),
            ScriptError::UnknownCommand { ref command, line } if command == "frobnicate" && line == 1
        ));
        assert!(matches!(
            parse("key NOTAKEY").unwrap_err(),
            ScriptError::BadKeyName { ref name, line } if name == "NOTAKEY" && line == 1
        ));
        assert!(matches!(
            parse("wait notanumber").unwrap_err(),
            ScriptError::BadNumber { ref text, line } if text == "notanumber" && line == 1
        ));
    }

    #[test]
    fn every_key_name_round_trips() {
        let names = [
            "A",
            "B",
            "C",
            "D",
            "E",
            "F",
            "G",
            "H",
            "I",
            "J",
            "K",
            "L",
            "M",
            "N",
            "O",
            "P",
            "Q",
            "R",
            "S",
            "T",
            "U",
            "V",
            "W",
            "X",
            "Y",
            "Z",
            "0",
            "1",
            "2",
            "3",
            "4",
            "5",
            "6",
            "7",
            "8",
            "9",
            "F1",
            "F2",
            "F3",
            "F4",
            "F5",
            "F6",
            "F7",
            "F8",
            "F9",
            "F10",
            "F11",
            "F12",
            "ESCAPE",
            "ENTER",
            "TAB",
            "SPACE",
            "BACKSPACE",
            "UP",
            "DOWN",
            "LEFT",
            "RIGHT",
            "LSHIFT",
            "LCTRL",
            "LALT",
        ];
        for name in names {
            let key = Key::from_name(name).unwrap_or_else(|| panic!("key {name} did not parse"));
            assert_eq!(key.name(), name);
            assert_eq!(key.to_string(), name);
        }
        assert!(Key::from_name("zz").is_none());
        assert!(Key::from_name("F13").is_none());
    }

    #[test]
    fn key_names_parse_in_any_case() {
        // The canonical names round-trip; other casings parse to the same key.
        for (written, want) in [
            ("Escape", Key::Escape),
            ("escape", Key::Escape),
            ("ESCAPE", Key::Escape),
            ("E", Key::E),
            ("e", Key::E),
            ("F3", Key::F3),
            ("f3", Key::F3),
            ("f12", Key::F12),
            ("Tab", Key::Tab),
            ("Enter", Key::Enter),
            ("Space", Key::Space),
            ("Backspace", Key::Backspace),
            ("LeftShift", Key::LeftShift),
            ("lshift", Key::LeftShift),
            ("LShift", Key::LeftShift),
            ("LeftCtrl", Key::LeftCtrl),
            ("LeftAlt", Key::LeftAlt),
            ("Up", Key::Up),
            ("Down", Key::Down),
            ("Left", Key::Left),
            ("Right", Key::Right),
            ("W", Key::W),
            ("5", Key::Num5),
        ] {
            assert_eq!(Key::from_name(written), Some(want), "key {written}");
        }
        // Still one key per name: casings do not create duplicates.
        assert_eq!(Key::from_name("Escape"), Key::from_name("ESCAPE"));
    }

    #[test]
    fn wait_counts_ticks() {
        let mut runner = Runner::new(parse("wait 3\nscreenshot x").unwrap());
        let state = ScriptState::default();
        assert!(runner.tick(&state).is_empty());
        assert!(runner.tick(&state).is_empty());
        assert!(runner.tick(&state).is_empty());
        assert_eq!(
            runner.tick(&state),
            vec![Action::Screenshot("x".to_owned())]
        );
        assert_eq!(runner.tick(&state), vec![Action::Finished]);
    }

    #[test]
    fn wait_zero_takes_no_idle_tick() {
        let mut runner = Runner::new(parse("wait 0\nscreenshot x").unwrap());
        let state = ScriptState::default();
        assert_eq!(
            runner.tick(&state),
            vec![Action::Screenshot("x".to_owned())]
        );
    }

    #[test]
    fn key_presses_then_releases() {
        let mut runner = Runner::new(parse("key W").unwrap());
        let state = ScriptState::default();
        assert_eq!(runner.tick(&state), vec![Action::KeyDown(Key::W)]);
        assert_eq!(runner.tick(&state), vec![Action::KeyUp(Key::W)]);
        assert_eq!(runner.tick(&state), vec![Action::Finished]);
    }

    #[test]
    fn key_down_and_key_up_are_single_actions() {
        let mut runner = Runner::new(parse("key_down SPACE\nkey_up SPACE").unwrap());
        let state = ScriptState::default();
        assert_eq!(runner.tick(&state), vec![Action::KeyDown(Key::Space)]);
        assert_eq!(runner.tick(&state), vec![Action::KeyUp(Key::Space)]);
    }

    #[test]
    fn mouse_down_and_mouse_up_are_single_actions() {
        let mut runner = Runner::new(parse("mouse_down left\nmouse_up left").unwrap());
        let state = ScriptState::default();
        assert_eq!(
            runner.tick(&state),
            vec![Action::MouseDown(MouseButton::Left)]
        );
        assert_eq!(
            runner.tick(&state),
            vec![Action::MouseUp(MouseButton::Left)]
        );
        assert_eq!(runner.tick(&state), vec![Action::Finished]);
    }

    #[test]
    fn wait_until_blocks_then_proceeds() {
        let mut runner = Runner::new(parse("wait_until joined\nscreenshot x").unwrap());
        let mut state = ScriptState::default();
        for _ in 0..5 {
            assert!(runner.tick(&state).is_empty());
        }
        state.joined = true;
        assert_eq!(
            runner.tick(&state),
            vec![Action::Screenshot("x".to_owned())]
        );
    }

    #[test]
    fn wait_until_proceeds_immediately_when_already_true() {
        let mut runner = Runner::new(parse("wait_until settled\nscreenshot x").unwrap());
        let state = ScriptState {
            settled: true,
            ..Default::default()
        };
        assert_eq!(
            runner.tick(&state),
            vec![Action::Screenshot("x".to_owned())]
        );
    }

    #[test]
    fn wait_until_screen_closed_blocks_until_closed() {
        let mut runner = Runner::new(parse("wait_until screen_closed\nscreenshot x").unwrap());
        // Closed means "not open" (`!screen_open`); while a screen is up the condition is false.
        let mut state = ScriptState {
            screen_open: true,
            ..Default::default()
        };
        for _ in 0..3 {
            assert!(runner.tick(&state).is_empty());
        }
        state.screen_open = false;
        assert_eq!(
            runner.tick(&state),
            vec![Action::Screenshot("x".to_owned())]
        );
    }

    #[test]
    fn wait_until_times_out_with_line_number() {
        let mut runner = Runner::new(parse("\nwait_until settled").unwrap());
        let state = ScriptState::default();
        for _ in 0..(WAIT_UNTIL_TIMEOUT - 1) {
            assert!(runner.tick(&state).is_empty());
        }
        let actions = runner.tick(&state);
        assert_eq!(actions.len(), 1);
        let Action::Error(message) = &actions[0] else {
            panic!("expected an error action, got {actions:?}");
        };
        assert!(message.contains("line 2"), "{message}");
        assert_eq!(runner.tick(&state), vec![Action::Finished]);
    }

    #[test]
    fn full_script_produces_expected_actions() {
        let script = [
            "wait_until joined",
            "look 90 0",
            "key W",
            "key_down SPACE",
            "wait 2",
            "key_up SPACE",
            "type hello world",
            "mouse 100 50",
            "click left 100 50",
            "scroll -1",
            "screenshot out",
        ]
        .join("\n");
        let mut runner = Runner::new(parse(&script).unwrap());
        let state = ScriptState {
            joined: true,
            ..Default::default()
        };
        let mut got = Vec::new();
        for _ in 0..1000 {
            let actions = runner.tick(&state);
            if actions.iter().any(|a| matches!(a, Action::Finished)) {
                break;
            }
            got.extend(actions);
        }
        assert!(runner.is_finished());
        assert_eq!(
            got,
            vec![
                Action::Look(90.0, 0.0),
                Action::KeyDown(Key::W),
                Action::KeyUp(Key::W),
                Action::KeyDown(Key::Space),
                Action::KeyUp(Key::Space),
                Action::Text("hello world".to_owned()),
                Action::MouseMove(100, 50),
                Action::Click(MouseButton::Left, 100, 50),
                Action::Scroll(-1),
                Action::Screenshot("out".to_owned()),
            ]
        );
    }
}
