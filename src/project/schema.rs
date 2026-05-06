//! Serde schema for project files. Stable; changes need a migration.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
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
    /// HDR-to-SDR tone map applied at readback. Defaults to ACES, which
    /// preserves saturation in highlights better than Reinhard.
    #[serde(default)]
    pub tone_map: ToneMap,
    /// All node instances, keyed by name.
    pub nodes: IndexMap<String, NodeSpec>,
    /// Name of the node whose output texture is the final video frame.
    pub output: String,

    /// Directory the project was loaded from. Used to resolve relative paths
    /// (e.g. `custom_shader`'s `path` param). Populated by `Project::load`;
    /// projects deserialized in-memory have an empty value, which is fine
    /// for builds that don't reference any file-loading nodes.
    #[serde(skip, default)]
    pub source_dir: PathBuf,
}

/// Tone-mapping curve applied to engine-internal Rgba16Float values when
/// converting them to 8-bit RGBA for video output. The internal pipeline
/// runs in HDR (values can exceed 1.0); this is what compresses them back
/// into displayable range.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToneMap {
    /// ACES filmic approximation (Narkowicz). Industry-standard, preserves
    /// highlight saturation, gentle roll-off. Good default for most pieces.
    #[default]
    Aces,
    /// Reinhard (`x / (1 + x)`). Simple, but desaturates bright RGB values.
    /// Use when you want a softer, more washed look.
    Reinhard,
    /// No tone mapping; values are simply clamped into [0, 1]. Useful when
    /// the project is already SDR-bounded and you want bit-exact output.
    None,
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

impl NodeSpec {
    /// Read a scalar parameter (number or audio binding), falling back to
    /// `default` if absent. Errors if the user supplied a non-scalar value
    /// (e.g. a color literal where a number was expected).
    pub fn scalar_param(&self, name: &str, default: f32) -> Result<ParamValue> {
        self.params
            .get(name)
            .cloned()
            .unwrap_or(ParamValue::Number(default))
            .require_scalar(name)
    }

    /// Read a color parameter, falling back to `default` (RGBA) if absent.
    /// Strings and numbers are rejected.
    pub fn color_param(&self, name: &str, default: [f32; 4]) -> Result<ParamValue> {
        let v = self
            .params
            .get(name)
            .cloned()
            .unwrap_or_else(|| ParamValue::Color(default.to_vec()));
        match &v {
            ParamValue::Color(_) => Ok(v),
            other => Err(anyhow!(
                "param `{name}` must be a color, got {}",
                other.type_name()
            )),
        }
    }
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
            ParamValue::Audio {
                feature,
                scale,
                bias,
            } => {
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

    /// Returns `self` if this value can be resolved to a scalar (a number
    /// literal or an audio binding); errors otherwise. Use at node
    /// construction so malformed projects fail fast with a useful message
    /// instead of panicking mid-render.
    pub fn require_scalar(self, param_name: &str) -> Result<Self> {
        match &self {
            ParamValue::Number(_) | ParamValue::Audio { .. } => Ok(self),
            other => Err(anyhow!(
                "param `{param_name}` must be a number or audio binding, got {}",
                other.type_name()
            )),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            ParamValue::Number(_) => "a number",
            ParamValue::Color(_) => "a color",
            ParamValue::String(_) => "a string",
            ParamValue::Audio { .. } => "an audio binding",
        }
    }
}

fn default_version() -> u32 {
    1
}

fn one() -> f32 {
    1.0
}
