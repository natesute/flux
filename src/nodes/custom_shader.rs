//! `custom_shader` — load a user-authored WGSL fullscreen shader from a path
//! relative to the project file. The escape hatch from "whatever nodes
//! flux ships with."
//!
//! ## Binding contract (must match in the user shader)
//!
//! ```wgsl
//! struct Uniforms {
//!     time: f32,
//!     frame: f32,
//!     resolution: vec2<f32>,
//!     rms: f32,
//!     bass: f32,
//!     low_mid: f32,
//!     high_mid: f32,
//!     treble: f32,
//! };
//!
//! @group(0) @binding(0) var<uniform> u: Uniforms;
//! @group(0) @binding(1) var samp: sampler;
//! @group(0) @binding(2) var input0: texture_2d<f32>;
//! // up to input3 at binding 5
//!
//! @vertex   fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> { ... }
//! @fragment fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> { ... }
//! ```
//!
//! Number of `inputN` bindings must equal the node's `inputs.len()`. Stock
//! uniforms and sampler are always bound; pass user-tunable values via
//! audio bindings on the existing audio bands.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::NodeSpec;

/// Maximum number of input textures a custom shader can declare.
const MAX_INPUTS: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct StockUniforms {
    time: f32,
    frame: f32,
    resolution: [f32; 2],
    rms: f32,
    bass: f32,
    low_mid: f32,
    high_mid: f32,
    treble: f32,
    // WGSL rounds the struct size up to the alignment of its largest
    // member (vec2 = 8). Without this trailing slot the buffer is 36
    // bytes where the shader expects 40.
    _pad: f32,
}

pub struct CustomShaderNode {
    inputs: Vec<String>,
    /// Source path the current pipeline was compiled from. `None` when
    /// constructed directly from source via `from_source` (test harness).
    /// Used by `update_params` to detect path changes.
    shader_path: Option<String>,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
}

impl CustomShaderNode {
    pub fn new(spec: &NodeSpec, project_dir: &Path, gpu: &GpuContext) -> Result<Self> {
        let path = spec
            .params
            .get("path")
            .and_then(|v| v.as_string())
            .ok_or_else(|| anyhow!("`custom_shader` requires a `path` string param"))?;

        let abs = project_dir.join(path);
        let source = std::fs::read_to_string(&abs)
            .with_context(|| format!("reading custom shader {}", abs.display()))?;

        Self::from_source(&source, spec.inputs.len(), gpu)
            .with_context(|| format!("compiling custom shader {}", abs.display()))
            .map(|mut n| {
                n.inputs = spec.inputs.clone();
                n.shader_path = Some(path.to_string());
                n
            })
    }

    /// Build a node directly from shader source. Used by tests; `new` calls
    /// this after reading from disk. `input_count` decides how many texture
    /// bindings the layout has — the user shader must declare exactly that
    /// many `inputN` textures.
    pub fn from_source(shader_source: &str, input_count: usize, gpu: &GpuContext) -> Result<Self> {
        if input_count > MAX_INPUTS {
            return Err(anyhow!(
                "`custom_shader` supports at most {MAX_INPUTS} inputs, got {input_count}"
            ));
        }

        let device = &gpu.device;

        // Bind group layout: uniforms + sampler + input_count textures.
        let mut entries = vec![shader_pass::uniform_entry(0), shader_pass::sampler_entry(1)];
        for i in 0..input_count {
            entries.push(shader_pass::texture_entry(2 + i as u32));
        }
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("custom_shader bgl"),
            entries: &entries,
        });

        // Compile under a validation error scope so we surface a real error
        // message instead of an opaque later panic.
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline =
            shader_pass::build_fullscreen_pipeline(gpu, "custom_shader", shader_source, &bgl);
        if let Some(err) = pollster::block_on(device.pop_error_scope()) {
            return Err(anyhow!("shader validation failed: {err}"));
        }

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("custom_shader uniforms"),
            size: std::mem::size_of::<StockUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            inputs: Vec::new(),
            shader_path: None,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
            bind_group: None,
        })
    }
}

impl Node for CustomShaderNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "custom_shader"
    }

    fn input_refs(&self) -> &[String] {
        &self.inputs
    }

    fn expected_input_count(&self) -> usize {
        // The shader's bind-group layout was compiled with this many
        // texture bindings; cook needs the input slice padded to that
        // length even if the spec wires fewer.
        self.inputs.len()
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        // Only the path matters for in-place updates — there are no
        // tunable params on a custom shader. If the path changed the
        // node has to be rebuilt to recompile the new shader source.
        let new_path = spec
            .params
            .get("path")
            .and_then(|v| v.as_string())
            .map(String::from);
        if new_path != self.shader_path {
            return Err(anyhow!(
                "custom_shader `path` changed (recompile required; full rebuild)"
            ));
        }
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[&wgpu::Texture],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let uniforms = StockUniforms {
            time: ctx.time,
            frame: ctx.frame_index as f32,
            resolution: [ctx.width as f32, ctx.height as f32],
            rms: ctx.audio.rms,
            bass: ctx.audio.bass,
            low_mid: ctx.audio.low_mid,
            high_mid: ctx.audio.high_mid,
            treble: ctx.audio.treble,
            _pad: 0.0,
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        if self.bind_group.is_none() {
            // Build texture views once; they live as long as the bind group.
            let views: Vec<wgpu::TextureView> = inputs
                .iter()
                .map(|tex| tex.create_view(&wgpu::TextureViewDescriptor::default()))
                .collect();
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ];
            for (i, view) in views.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry {
                    binding: 2 + i as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
            self.bind_group = Some(
                ctx.gpu
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("custom_shader bg"),
                        layout: &self.bgl,
                        entries: &entries,
                    }),
            );
        }

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "custom_shader",
            &self.pipeline,
            self.bind_group.as_ref().unwrap(),
            output,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    /// A no-input shader that fills the frame with a per-frame color from
    /// stock uniforms. Verifies the binding contract end-to-end.
    #[test]
    fn no_input_shader_runs() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let source = r#"
struct Uniforms {
    time: f32,
    frame: f32,
    resolution: vec2<f32>,
    rms: f32,
    bass: f32,
    low_mid: f32,
    high_mid: f32,
    treble: f32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(0.4, 0.7, 0.2, 1.0);
}
"#;
        let mut node = CustomShaderNode::from_source(source, 0, &harness.gpu).unwrap();
        let stats = harness.cook(&mut node, &[], FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }

    /// A 1-input passthrough shader. Verifies the input texture bindings work.
    #[test]
    fn one_input_passthrough() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let source = r#"
struct Uniforms {
    time: f32,
    frame: f32,
    resolution: vec2<f32>,
    rms: f32,
    bass: f32,
    low_mid: f32,
    high_mid: f32,
    treble: f32,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var input0: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / u.resolution;
    return textureSample(input0, samp, uv);
}
"#;
        let mut node = CustomShaderNode::from_source(source, 1, &harness.gpu).unwrap();
        let src = harness.constant_texture([0.5, 0.25, 0.75, 1.0]);
        let inputs: &[&wgpu::Texture] = &[&src];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }

    /// Compile errors should surface as anyhow errors, not panics.
    #[test]
    fn malformed_shader_errors() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let bad_source = "this is not WGSL";
        let result = CustomShaderNode::from_source(bad_source, 0, &harness.gpu);
        assert!(result.is_err(), "expected compile failure to return Err");
    }
}
