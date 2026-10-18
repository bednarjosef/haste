//! Vanilla `net.minecraft.client.CloudStatus` (options.txt `renderClouds`): the cloud
//! mode of the video settings. OFF draws no cloud pass, FAST the flat cells, FANCY the
//! extruded boxes; vanilla's default is FANCY.

use serde::{Deserialize, Serialize};

/// Vanilla `CloudStatus`. Serializes as its options.txt value (vanilla's legacy names:
/// `"false"`, `"fast"`, `"true"` — `Codec.withAlternative(CloudStatus.CODEC, BOOL)`
/// accepts the plain booleans for OFF/FANCY).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudStatus {
    #[serde(rename = "false")]
    Off,
    #[serde(rename = "fast")]
    Fast,
    /// Vanilla's default (`Options.cloudStatus`).
    #[default]
    #[serde(rename = "true")]
    Fancy,
}

impl CloudStatus {
    /// Parses one option value: the options.txt spellings (`"false"`/`"fast"`/`"true"`,
    /// vanilla's serialized names) and the enum names the JVM's option bridge reports
    /// for a live change (`"OFF"`/`"FAST"`/`"FANCY"`). Anything else is malformed and
    /// leaves the option untouched, like vanilla's load skipping unknown values.
    pub fn parse(value: &str) -> Option<CloudStatus> {
        match value {
            "false" | "OFF" => Some(CloudStatus::Off),
            "fast" | "FAST" => Some(CloudStatus::Fast),
            "true" | "FANCY" => Some(CloudStatus::Fancy),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_spellings_and_defaults_to_fancy() {
        assert_eq!(CloudStatus::default(), CloudStatus::Fancy);
        // options.txt (vanilla's serialized names).
        assert_eq!(CloudStatus::parse("false"), Some(CloudStatus::Off));
        assert_eq!(CloudStatus::parse("fast"), Some(CloudStatus::Fast));
        assert_eq!(CloudStatus::parse("true"), Some(CloudStatus::Fancy));
        // The JVM bridge reports the enum's own toString.
        assert_eq!(CloudStatus::parse("OFF"), Some(CloudStatus::Off));
        assert_eq!(CloudStatus::parse("FAST"), Some(CloudStatus::Fast));
        assert_eq!(CloudStatus::parse("FANCY"), Some(CloudStatus::Fancy));
        // Unknown values are malformed, not a mode.
        assert_eq!(CloudStatus::parse("fancy"), None);
        assert_eq!(CloudStatus::parse("maybe"), None);
    }

    #[test]
    fn serializes_with_the_options_txt_names() {
        let text = toml::to_string(&super::super::Options {
            clouds: CloudStatus::Off,
            ..Default::default()
        })
        .unwrap();
        assert!(text.contains("clouds = \"false\""));
        let back: super::super::Options = toml::from_str(&text).unwrap();
        assert_eq!(back.clouds, CloudStatus::Off);
        let opts: super::super::Options = toml::from_str("clouds = \"fast\"\n").unwrap();
        assert_eq!(opts.clouds, CloudStatus::Fast);
    }
}
