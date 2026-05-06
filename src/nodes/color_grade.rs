//! `color_grade` — 3D LUT color grading via a 256×16 PNG strip.
//!
//! Load order: if `path` is set, read that PNG (relative to the project
//! file). Otherwise an identity LUT is generated on the fly so the node
//! is a no-op until you point it at a real LUT.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::shader_pass;
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

const SHADER: &str = include_str!("../../shaders/color_grade.wgsl");
const LUT_SIZE: u32 = 16;
const LUT_WIDTH: u32 = LUT_SIZE * LUT_SIZE; // 256
const LUT_HEIGHT: u32 = LUT_SIZE;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniforms {
    intensity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

pub struct ColorGradeNode {
    inputs: Vec<String>,
    intensity: ParamValue,

    bgl: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    lut_texture: wgpu::Texture,
}

impl ColorGradeNode {
    pub fn new(spec: &NodeSpec, project_dir: &Path, gpu: &GpuContext) -> Result<Self> {
        if spec.inputs.len() != 1 {
            return Err(anyhow!(
                "`color_grade` requires exactly 1 input, got {}",
                spec.inputs.len()
            ));
        }

        let lut_pixels = match spec.params.get("path").and_then(|v| v.as_string()) {
            Some(p) => {
                let abs = project_dir.join(p);
                load_lut_png(&abs)?
            }
            None => identity_lut(),
        };

        let device = &gpu.device;
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("color_grade bgl"),
            entries: &[
                shader_pass::uniform_entry(0),
                shader_pass::texture_entry(1),
                shader_pass::texture_entry(2),
                shader_pass::sampler_entry(3),
            ],
        });
        let pipeline = shader_pass::build_fullscreen_pipeline(gpu, "color_grade", SHADER, &bgl);

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("color_grade uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Upload the LUT as Rgba8Unorm. Filterable so linear sampling works
        // for the within-cell red/green interpolation.
        let lut_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color_grade lut"),
            size: wgpu::Extent3d {
                width: LUT_WIDTH,
                height: LUT_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &lut_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &lut_pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(LUT_WIDTH * 4),
                rows_per_image: Some(LUT_HEIGHT),
            },
            wgpu::Extent3d {
                width: LUT_WIDTH,
                height: LUT_HEIGHT,
                depth_or_array_layers: 1,
            },
        );

        Ok(Self {
            inputs: spec.inputs.clone(),
            intensity: spec.scalar_param("intensity", 1.0)?,
            bgl,
            pipeline,
            uniform_buffer,
            sampler: shader_pass::linear_clamp_sampler(gpu),
            lut_texture,
        })
    }
}

impl Node for ColorGradeNode {
    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        inputs: &[(String, &wgpu::Texture)],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let view_in = inputs[0]
            .1
            .create_view(&wgpu::TextureViewDescriptor::default());
        let view_lut = self
            .lut_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let uniforms = Uniforms {
            intensity: self.intensity.resolve_scalar(&ctx.audio),
            ..Default::default()
        };
        ctx.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = ctx
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("color_grade bg"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view_in),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view_lut),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

        shader_pass::run_fullscreen_pass(
            ctx.gpu,
            "color_grade",
            &self.pipeline,
            &bind_group,
            output,
        );
        Ok(())
    }
}

/// Build the identity LUT (256×16 RGBA8): for each cell `b` in 0..16 and
/// each pixel (r, g) in 0..16, the color is `(r/15, g/15, b/15, 1)`. Used
/// when the user hasn't pointed the node at a custom LUT yet.
fn identity_lut() -> Vec<u8> {
    let mut out = Vec::with_capacity((LUT_WIDTH * LUT_HEIGHT * 4) as usize);
    let s = (LUT_SIZE - 1) as f32;
    for y in 0..LUT_HEIGHT {
        for x in 0..LUT_WIDTH {
            let cell = x / LUT_SIZE; // 0..15
            let r_idx = x % LUT_SIZE;
            out.push(((r_idx as f32 / s) * 255.0).round() as u8);
            out.push(((y as f32 / s) * 255.0).round() as u8);
            out.push(((cell as f32 / s) * 255.0).round() as u8);
            out.push(255);
        }
    }
    out
}

fn load_lut_png(path: &Path) -> Result<Vec<u8>> {
    let img = image::open(path).with_context(|| format!("opening LUT {}", path.display()))?;
    let rgba = img.to_rgba8();
    if rgba.width() != LUT_WIDTH || rgba.height() != LUT_HEIGHT {
        return Err(anyhow!(
            "LUT {} must be {}×{}, got {}×{}",
            path.display(),
            LUT_WIDTH,
            LUT_HEIGHT,
            rgba.width(),
            rgba.height()
        ));
    }
    Ok(rgba.into_raw())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    /// With the identity LUT and intensity=1.0, output should equal input.
    #[test]
    fn identity_lut_is_passthrough() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec = ron::from_str(
            r#"(type: "color_grade", inputs: ["src"], params: { "intensity": 1.0 })"#,
        )
        .unwrap();
        let mut node = ColorGradeNode::new(&spec, Path::new("."), &harness.gpu).unwrap();
        let src = harness.constant_texture([0.5, 0.25, 0.75, 1.0]);
        let inputs: &[(String, &wgpu::Texture)] = &[("src".to_string(), &src)];
        let stats = harness.cook(&mut node, inputs, FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
