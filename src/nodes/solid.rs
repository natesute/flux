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
        let color = spec
            .params
            .get("color")
            .cloned()
            .unwrap_or(ParamValue::Color(vec![1.0, 1.0, 1.0, 1.0]));
        let intensity = spec
            .params
            .get("intensity")
            .cloned()
            .unwrap_or(ParamValue::Number(1.0));
        Ok(Self {
            inputs: spec.inputs.clone(),
            color,
            intensity,
        })
    }
}

impl Node for SolidNode {
    fn input_refs(&self) -> Vec<String> {
        self.inputs.clone()
    }

    fn cook(
        &mut self,
        ctx: &FrameContext,
        _inputs: &[(String, &wgpu::Texture)],
        output: &wgpu::Texture,
    ) -> Result<()> {
        let intensity = self.intensity.resolve_scalar(&ctx.audio);
        let c = self.color.as_color();
        let view = output.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = ctx.gpu.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("solid") },
        );
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
