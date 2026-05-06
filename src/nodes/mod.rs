//! Node trait and built-in node registry.
//!
//! Every node type lives in its own file. `node_from_spec` dispatches by
//! the type string in the project file. Nodes that share the
//! "fullscreen-shader" pattern use helpers from `shader_pass`.

mod blend;
mod bloom;
mod custom_shader;
mod displace;
mod feedback;
mod gradient;
mod levels;
mod noise;
mod shader_pass;
mod solid;
mod transform;

pub use blend::BlendNode;
pub use bloom::BloomNode;
pub use custom_shader::CustomShaderNode;
pub use displace::DisplaceNode;
pub use feedback::FeedbackNode;
pub use gradient::GradientNode;
pub use levels::LevelsNode;
pub use noise::NoiseNode;
pub use solid::SolidNode;
pub use transform::TransformNode;

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::engine::{FrameContext, GpuContext};
use crate::project::NodeSpec;

/// A node in the dataflow graph.
///
/// Cooking happens once per frame. The node receives its input textures
/// as a slice of `(name, &Texture)` pairs in the order declared in the
/// project file, and writes its output into the provided `output` texture.
pub trait Node: Send {
    /// Names of input nodes, in the order they should be passed to `cook`.
    /// Pulled from the project file at construction; cached here so the
    /// graph can topologically sort without re-parsing.
    fn input_refs(&self) -> Vec<String>;

    /// Render this node's output for the current frame.
    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[(String, &wgpu::Texture)],
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
        "custom_shader" => Box::new(CustomShaderNode::new(spec, project_dir, gpu)?),
        other => {
            return Err(anyhow!(
                "unknown node type `{other}`. Run `flux nodes` to list available types."
            ))
        }
    };
    Ok(node)
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
        "custom_shader",
    ]
}
