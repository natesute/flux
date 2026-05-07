//! `solid` — fills its output with a constant color. The simplest node;
//! useful as a baseline and a sanity check that the engine works.

use anyhow::Result;

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::Node;
use crate::project::{NodeSpec, ParamValue};

pub struct SolidNode {
    inputs: Vec<String>,
    color: ParamValue,
    /// If `intensity` is bound to audio, the color is multiplied by it.
    intensity: ParamValue,
}

impl SolidNode {
    pub fn new(spec: &NodeSpec, _gpu: &GpuContext) -> Result<Self> {
        Ok(Self {
            inputs: spec.inputs.clone(),
            color: spec.color_param("color", [1.0, 1.0, 1.0, 1.0])?,
            intensity: spec.scalar_param("intensity", 1.0)?,
        })
    }
}

impl Node for SolidNode {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn kind(&self) -> &'static str {
        "solid"
    }

    fn input_refs(&self) -> &[String] {
        &self.inputs
    }

    fn update_params(&mut self, spec: &NodeSpec) -> Result<()> {
        self.color = spec.color_param("color", [1.0, 1.0, 1.0, 1.0])?;
        self.intensity = spec.scalar_param("intensity", 1.0)?;
        Ok(())
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        _inputs: &[&wgpu::Texture],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let intensity = self.intensity.resolve_scalar(&ctx.audio);
        let c = self.color.as_color();
        let view = output.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = ctx
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("solid"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("solid pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: (c[0] * intensity) as f64,
                            g: (c[1] * intensity) as f64,
                            b: (c[2] * intensity) as f64,
                            a: c[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        ctx.gpu.queue.submit(Some(encoder.finish()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::FrameAudioFeatures;
    use crate::test_utils::TestHarness;

    #[test]
    fn fills_with_color() {
        let Some(harness) = TestHarness::try_init(32, 32) else {
            return;
        };
        let spec: NodeSpec =
            ron::from_str(r#"(type: "solid", params: { "color": [0.4, 0.7, 0.2, 1.0] })"#).unwrap();
        let mut node = SolidNode::new(&spec, &harness.gpu).unwrap();
        let stats = harness.cook(&mut node, &[], FrameAudioFeatures::default(), 0.0);
        insta::assert_snapshot!(stats);
    }
}
