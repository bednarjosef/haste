// Blindness fog environment (vanilla `BlindnessFogEnvironment`).

/// `BlindnessFogEnvironment.getModifiedDarkness`: full darkness except in the last second of
/// the effect, where it is the remaining fraction.
pub(super) fn blindness_darkness(duration: i32, darkness: f32) -> f32 {
    if duration != -1 && duration <= 19 {
        (duration as f32 / 20.0).max(darkness)
    } else {
        1.0
    }
}
