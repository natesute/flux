//! Serde schema for project files. Stable; changes need a migration.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A complete project. Equivalent to a `.toe` file in TouchDesigner's
/// vocabulary, but human-authored and diffable.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Project {
    /// Schema version. Bumped on breaking changes; older versions get
    /// migrated automatically when supported.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Output resolution in pixels.
    pub width: u32,
    pub height: u32,
    /// Default framerate for rendering.
    pub fps: u32,
    /// All node instances, keyed by name.
    pub nodes: IndexMap<String, NodeSpec>,
    /// Name of the node whose output texture is the final video frame.
    pub output: String,
}

/// One node in the graph.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NodeSpec {
    /// Built-in node type name (e.g. "gradient", "feedback", "bloom").
    #[serde(rename = "type")]
    pub kind: String,
    /// Names of input nodes, in the order this node expects them.
    /// Most nodes have 0..2 inputs.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Static parameters. Each value can also be a binding to an audio
    /// feature; see `ParamValue`.
    #[serde(default)]
    pub params: IndexMap<String, ParamValue>,
}

/// A parameter is either a literal or a binding to an audio feature.
/// Bindings let you say "drive this parameter with the bass band".
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// Plain number.
    Number(f32),
    /// RGB or RGBA color, components in 0..1.
    Color(Vec<f32>),
    /// String literal (used for enums like blend modes).
    String(String),
    /// Audio binding: { feature: "bass", scale: 1.0, bias: 0.0 }.
    Audio {
        feature: AudioFeature,
        #[serde(default = "one")]
        scale: f32,
        #[serde(default)]
        bias: f32,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFeature {
    Rms,
    Bass,
    LowMid,
    HighMid,
    Treble,
}

impl ParamValue {
    /// Resolve this parameter to a scalar at render time, given current
    /// audio features. Non-numeric values panic — call only on numeric
    /// params.
    pub fn resolve_scalar(&self, audio: &crate::audio::FrameAudioFeatures) -> f32 {
        match self {
            ParamValue::Number(n) => *n,
            ParamValue::Audio { feature, scale, bias } => {
                let v = match feature {
                    AudioFeature::Rms => audio.rms,
                    AudioFeature::Bass => audio.bass,
                    AudioFeature::LowMid => audio.low_mid,
                    AudioFeature::HighMid => audio.high_mid,
                    AudioFeature::Treble => audio.treble,
                };
                v * scale + bias
            }
            ParamValue::Color(_) | ParamValue::String(_) => {
                panic!("resolve_scalar called on non-scalar parameter")
            }
        }
    }

    pub fn as_color(&self) -> [f32; 4] {
        match self {
            ParamValue::Color(v) => match v.len() {
                3 => [v[0], v[1], v[2], 1.0],
                4 => [v[0], v[1], v[2], v[3]],
                _ => [1.0, 1.0, 1.0, 1.0],
            },
            _ => [1.0, 1.0, 1.0, 1.0],
        }
    }

    pub fn as_string(&self) -> Option<&str> {
        if let ParamValue::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
}

fn default_version() -> u32 {
    1
}

fn one() -> f32 {
    1.0
}
