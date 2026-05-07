//! Node trait and built-in node registry.
//!
//! Every node type lives in its own file. `node_from_spec` dispatches by
//! the type string in the project file. Nodes that share the
//! "fullscreen-shader" pattern use helpers from `shader_pass`.

mod blend;
mod bloom;
mod chromatic_aberration;
mod color_grade;
mod custom_shader;
mod displace;
mod feedback;
mod gradient;
mod grain;
mod instance;
mod levels;
mod noise;
mod raymarch;
mod shader_pass;
mod solid;
mod transform;

pub use blend::BlendNode;
pub use bloom::BloomNode;
pub use chromatic_aberration::ChromaticAberrationNode;
pub use color_grade::ColorGradeNode;
pub use custom_shader::CustomShaderNode;
pub use displace::DisplaceNode;
pub use feedback::FeedbackNode;
pub use gradient::GradientNode;
pub use grain::GrainNode;
pub use instance::InstanceNode;
pub use levels::LevelsNode;
pub use noise::NoiseNode;
pub use raymarch::RaymarchNode;
pub use solid::SolidNode;
pub use transform::TransformNode;

use std::path::Path;

use anyhow::{anyhow, Result};
use indexmap::IndexMap;

use crate::engine::{FrameContext, GpuContext};
use crate::project::{NodeSpec, ParamValue};

/// A node in the dataflow graph.
///
/// Cooking happens once per frame. The node receives its input textures
/// as a slice of `(name, &Texture)` pairs in the order declared in the
/// project file, and writes its output into the provided `output` texture.
pub trait Node: Send {
    /// Downcast hatch used by `Graph::transfer_preservable_state_from` to
    /// look at concrete node types without wedging a giant enum in. Keep
    /// the body trivial (`fn as_any_mut(&mut self) -> &mut dyn Any { self }`)
    /// in every implementation.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// The string this node type is registered under in `node_from_spec`.
    /// Used by `Graph::topology_matches` to detect whether a hot-reload
    /// can take the cheap "patch params in place" path or needs a full
    /// rebuild.
    fn kind(&self) -> &'static str;

    /// Names of input nodes, in the order they should be passed to `cook`.
    /// Pulled from the project file at construction; cached here so the
    /// graph can topologically sort without re-parsing. Returned by
    /// borrow so the per-frame cook loop doesn't allocate.
    fn input_refs(&self) -> &[String];

    /// How many input textures `cook` is guaranteed to receive. The
    /// engine pads short input lists with a fallback black texture to
    /// reach this count, so a node can always index `inputs[0]` etc.
    /// without checking. Default is 0; nodes with required inputs
    /// override.
    fn expected_input_count(&self) -> usize {
        0
    }

    /// Re-read parameter values from a fresh spec **without** recreating
    /// any GPU resources. The caller (`Graph::update_params`) guarantees
    /// `spec.kind == self.kind()` and `spec.inputs == self.input_refs()`.
    /// Use this on the slider-drag hot path: building a render pipeline
    /// is hundreds of microseconds; flipping a `ParamValue` is nothing.
    ///
    /// Return `Err` only if the new spec can't be applied in place
    /// (e.g. a `custom_shader` whose source path changed); the caller
    /// will fall back to a full rebuild.
    fn update_params(&mut self, spec: &NodeSpec) -> Result<()>;

    /// Render this node's output for the current frame. `inputs` holds
    /// the upstream textures in the order declared by `input_refs()`.
    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[&wgpu::Texture],
        output: &wgpu::Texture,
    ) -> Result<()>;
}

pub type BoxedNode = Box<dyn Node>;

/// Look up and instantiate a node by its type name. `project_dir` is the
/// directory the project file lives in; nodes that load sibling files
/// (currently `custom_shader`) resolve their paths relative to it.
pub fn node_from_spec(
    _name: &str,
    spec: &NodeSpec,
    project_dir: &Path,
    gpu: &GpuContext,
) -> Result<BoxedNode> {
    let node: BoxedNode = match spec.kind.as_str() {
        "solid" => Box::new(SolidNode::new(spec, gpu)?),
        "gradient" => Box::new(GradientNode::new(spec, gpu)?),
        "noise" => Box::new(NoiseNode::new(spec, gpu)?),
        "feedback" => Box::new(FeedbackNode::new(spec, gpu)?),
        "blend" => Box::new(BlendNode::new(spec, gpu)?),
        "bloom" => Box::new(BloomNode::new(spec, gpu)?),
        "transform" => Box::new(TransformNode::new(spec, gpu)?),
        "levels" => Box::new(LevelsNode::new(spec, gpu)?),
        "displace" => Box::new(DisplaceNode::new(spec, gpu)?),
        "chromatic_aberration" => Box::new(ChromaticAberrationNode::new(spec, gpu)?),
        "grain" => Box::new(GrainNode::new(spec, gpu)?),
        "color_grade" => Box::new(ColorGradeNode::new(spec, project_dir, gpu)?),
        "raymarch" => Box::new(RaymarchNode::new(spec, gpu)?),
        "instance" => Box::new(InstanceNode::new(spec, gpu)?),
        "custom_shader" => Box::new(CustomShaderNode::new(spec, project_dir, gpu)?),
        other => {
            return Err(anyhow!(
                "unknown node type `{other}`. Run `flux nodes` to list available types."
            ))
        }
    };
    Ok(node)
}

/// Canonical default parameter set for a node `kind`. Mirrors what each
/// node's `new()` falls back to when a param is missing from the spec.
///
/// Used by the GUI inspector for two things: pre-populating a freshly
/// added node so its sliders appear immediately, and merging defaults
/// over a partially-specified .ron at render time so existing pieces'
/// hidden defaults become visible knobs. Keep this in sync with the
/// `scalar_param("foo", DEFAULT)?` calls in each node file — there's a
/// debug assertion at the bottom that flags drift.
pub fn default_params_for(kind: &str) -> IndexMap<String, ParamValue> {
    let mut p = IndexMap::new();
    let n = |x: f32| ParamValue::Number(x);
    let c = |r: f32, g: f32, b: f32, a: f32| ParamValue::Color(vec![r, g, b, a]);
    let s = |x: &str| ParamValue::String(x.to_string());
    match kind {
        "solid" => {
            p.insert("color".into(), c(1.0, 1.0, 1.0, 1.0));
            p.insert("intensity".into(), n(1.0));
        }
        "gradient" => {
            p.insert("inner_color".into(), c(1.0, 1.0, 1.0, 1.0));
            p.insert("outer_color".into(), c(0.0, 0.0, 0.0, 1.0));
            p.insert("radius".into(), n(0.5));
            p.insert("intensity".into(), n(1.0));
        }
        "noise" => {
            p.insert("color_a".into(), c(0.0, 0.0, 0.0, 1.0));
            p.insert("color_b".into(), c(1.0, 1.0, 1.0, 1.0));
            p.insert("scale".into(), n(3.0));
            p.insert("speed".into(), n(0.3));
            p.insert("octaves".into(), n(4.0));
            p.insert("contrast".into(), n(1.0));
            p.insert("intensity".into(), n(1.0));
        }
        "feedback" => {
            p.insert("decay".into(), n(0.92));
            p.insert("zoom".into(), n(1.01));
            p.insert("rotation".into(), n(0.0));
            p.insert("offset_x".into(), n(0.0));
            p.insert("offset_y".into(), n(0.0));
            p.insert("mix_in".into(), n(1.0));
        }
        "blend" => {
            p.insert("mode".into(), s("over"));
            p.insert("factor".into(), n(1.0));
            p.insert("opacity".into(), n(1.0));
        }
        "bloom" => {
            p.insert("threshold".into(), n(0.7));
            p.insert("intensity".into(), n(1.0));
            p.insert("radius".into(), n(4.0));
        }
        "transform" => {
            p.insert("offset_x".into(), n(0.0));
            p.insert("offset_y".into(), n(0.0));
            p.insert("rotation".into(), n(0.0));
            p.insert("scale_x".into(), n(1.0));
            p.insert("scale_y".into(), n(1.0));
        }
        "levels" => {
            p.insert("gain".into(), n(1.0));
            p.insert("brightness".into(), n(0.0));
            p.insert("contrast".into(), n(1.0));
            p.insert("saturation".into(), n(1.0));
        }
        "displace" => {
            p.insert("amount".into(), n(0.05));
            p.insert("mode".into(), s("derivative"));
        }
        "chromatic_aberration" => {
            p.insert("amount".into(), n(0.005));
            p.insert("center_x".into(), n(0.5));
            p.insert("center_y".into(), n(0.5));
        }
        "grain" => {
            p.insert("amount".into(), n(0.04));
            p.insert("scale".into(), n(1.0));
        }
        "color_grade" => {
            p.insert("intensity".into(), n(1.0));
        }
        "raymarch" => {
            p.insert("cam_x".into(), n(0.0));
            p.insert("cam_y".into(), n(0.5));
            p.insert("cam_z".into(), n(3.0));
            p.insert("look_x".into(), n(0.0));
            p.insert("look_y".into(), n(0.0));
            p.insert("look_z".into(), n(0.0));
            p.insert("fov".into(), n(0.9));
            p.insert("radius".into(), n(1.0));
            p.insert("displacement".into(), n(0.05));
            p.insert("light_x".into(), n(0.5));
            p.insert("light_y".into(), n(0.8));
            p.insert("light_z".into(), n(0.3));
            p.insert("sky_top".into(), c(0.4, 0.6, 0.9, 1.0));
            p.insert("sky_bottom".into(), c(0.05, 0.05, 0.1, 1.0));
        }
        "instance" => {
            p.insert("cam_x".into(), n(4.0));
            p.insert("cam_y".into(), n(3.0));
            p.insert("cam_z".into(), n(6.0));
            p.insert("look_x".into(), n(0.0));
            p.insert("look_y".into(), n(0.0));
            p.insert("look_z".into(), n(0.0));
            p.insert("fov".into(), n(0.8));
            p.insert("base_scale".into(), n(0.25));
            p.insert("audio_drive".into(), n(1.0));
            p.insert("light_x".into(), n(0.5));
            p.insert("light_y".into(), n(0.8));
            p.insert("light_z".into(), n(0.4));
            p.insert("rim_color".into(), c(1.0, 0.7, 0.4, 1.0));
        }
        "custom_shader" => {
            p.insert("path".into(), s("shaders/your_shader.wgsl"));
        }
        _ => {}
    }
    p
}

/// Names of all registered node types. Used by `flux nodes`.
pub fn registered_names() -> Vec<&'static str> {
    vec![
        "solid",
        "gradient",
        "noise",
        "feedback",
        "blend",
        "bloom",
        "transform",
        "levels",
        "displace",
        "chromatic_aberration",
        "grain",
        "color_grade",
        "raymarch",
        "instance",
        "custom_shader",
    ]
}
