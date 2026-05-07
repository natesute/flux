//! The dataflow graph: nodes connected by named inputs.
//!
//! On every frame the graph is "cooked" — nodes are evaluated in topological
//! order. Each node owns an output texture, which downstream nodes sample.

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use indexmap::IndexMap;

use crate::engine::{FrameContext, GpuContext};
use crate::nodes::{node_from_spec, BoxedNode};
use crate::project::{Project, ToneMap};

/// Identifier for a node in the graph. Currently just the node's name from
/// the project file.
pub type NodeId = String;

/// The cooked graph. Holds owned nodes and their textures, plus a precomputed
/// topological order.
pub struct Graph {
    nodes: IndexMap<NodeId, BoxedNode>,
    textures: HashMap<NodeId, wgpu::Texture>,
    eval_order: Vec<NodeId>,
    output_id: NodeId,
}

impl Graph {
    pub fn from_project(project: &Project, gpu: &GpuContext) -> Result<Self> {
        // Instantiate every node.
        let mut nodes: IndexMap<NodeId, BoxedNode> = IndexMap::new();
        for (name, spec) in &project.nodes {
            let node = node_from_spec(name, spec, &project.source_dir, gpu)
                .with_context(|| format!("instantiating node `{name}`"))?;
            nodes.insert(name.clone(), node);
        }

        // Validate output exists.
        if !nodes.contains_key(&project.output) {
            return Err(anyhow!(
                "output node `{}` not declared in project nodes",
                project.output
            ));
        }

        // Validate all referenced inputs exist.
        for (name, node) in &nodes {
            for input in node.input_refs() {
                if !nodes.contains_key(&input) {
                    return Err(anyhow!(
                        "node `{}` references unknown input node `{}`",
                        name,
                        input
                    ));
                }
            }
        }

        // Topological sort. Standard Kahn's algorithm.
        let eval_order = topo_sort(&nodes, &project.output)?;

        // Allocate output textures.
        let mut textures = HashMap::new();
        for (name, _node) in &nodes {
            let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("node:{name}")),
                size: wgpu::Extent3d {
                    width: project.width,
                    height: project.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: gpu.texture_format(),
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            textures.insert(name.clone(), tex);
        }

        Ok(Self {
            nodes,
            textures,
            eval_order,
            output_id: project.output.clone(),
        })
    }

    /// True if `project` is structurally identical to the live graph —
    /// same set/order of node names, same node `kind` for each, same
    /// inputs, same output, same resolution. When this returns true,
    /// `update_params` can apply the diff without touching any GPU
    /// resource other than uniforms.
    pub fn topology_matches(&self, project: &Project) -> bool {
        if self.output_id != project.output {
            return false;
        }
        if self.nodes.len() != project.nodes.len() {
            return false;
        }
        for ((live_name, live_node), (spec_name, spec)) in
            self.nodes.iter().zip(project.nodes.iter())
        {
            if live_name != spec_name {
                return false;
            }
            if live_node.kind() != spec.kind {
                return false;
            }
            if live_node.input_refs() != spec.inputs {
                return false;
            }
        }
        true
    }

    /// Apply per-node param updates from `project`. Caller guarantees
    /// `topology_matches(project)` was true. Any node whose
    /// `update_params` errors (e.g. a `custom_shader` whose `path`
    /// changed) propagates up so the caller can fall back to a full
    /// rebuild.
    pub fn update_params(&mut self, project: &Project) -> Result<()> {
        for (name, spec) in &project.nodes {
            let node = self
                .nodes
                .get_mut(name)
                .expect("topology_matches confirmed presence");
            node.update_params(spec)
                .with_context(|| format!("updating params for `{name}`"))?;
        }
        Ok(())
    }

    /// Evaluate every node for the current frame, in topological order.
    pub fn cook_frame(&mut self, ctx: &mut FrameContext) -> Result<()> {
        // We need to feed each node references to its input textures. To
        // satisfy the borrow checker we look up textures via the read-only
        // `textures` map while mutably borrowing one node at a time.
        for id in &self.eval_order {
            let node = self
                .nodes
                .get_mut(id)
                .expect("eval_order references existing node");

            // Gather input textures by name.
            let input_refs: Vec<(String, &wgpu::Texture)> = node
                .input_refs()
                .into_iter()
                .map(|name| {
                    let tex = self.textures.get(&name).expect("validated at graph build");
                    (name, tex)
                })
                .collect();

            let output_tex = self
                .textures
                .get(id)
                .expect("output texture allocated for every node");

            node.cook(ctx, &input_refs, output_tex)
                .with_context(|| format!("cooking node `{id}`"))?;
        }
        Ok(())
    }

    /// Read the output node's texture back to CPU as RGBA8 pixels for encoding,
    /// applying the requested tone map to the engine's HDR-range floats.
    pub fn read_output_pixels(&self, ctx: &FrameContext, tone_map: ToneMap) -> Result<Vec<u8>> {
        let texture = self.output_texture();
        crate::engine::graph::readback::texture_to_rgba8(
            ctx.gpu, texture, ctx.width, ctx.height, tone_map,
        )
    }

    /// Borrow the texture this graph writes into for the configured output
    /// node. Valid after `cook_frame`; sample it for live preview, copy it
    /// for offline render readback, etc.
    pub fn output_texture(&self) -> &wgpu::Texture {
        self.textures
            .get(&self.output_id)
            .expect("output texture exists")
    }
}

fn topo_sort(nodes: &IndexMap<NodeId, BoxedNode>, sink: &str) -> Result<Vec<NodeId>> {
    use std::collections::HashSet;

    // First, find every node reachable from the sink by walking inputs.
    // Unreachable nodes are silently dropped — useful for keeping commented-out
    // alternatives in a project file.
    let mut reachable: HashSet<NodeId> = HashSet::new();
    let mut stack = vec![sink.to_string()];
    while let Some(n) = stack.pop() {
        if reachable.insert(n.clone()) {
            if let Some(node) = nodes.get(&n) {
                for input in node.input_refs() {
                    stack.push(input);
                }
            }
        }
    }

    // Kahn's algorithm over the reachable subgraph.
    // indegree[n] = number of inputs n has (i.e. edges coming in).
    let mut indegree: HashMap<NodeId, usize> = HashMap::new();
    for name in &reachable {
        let inputs = nodes
            .get(name)
            .expect("reachable contains only valid nodes")
            .input_refs();
        indegree.insert(name.clone(), inputs.len());
    }

    // Reverse adjacency: for each input X, who depends on X?
    let mut dependents: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for name in &reachable {
        for input in nodes.get(name).unwrap().input_refs() {
            dependents.entry(input).or_default().push(name.clone());
        }
    }

    // Seed queue with zero-indegree nodes (sources).
    let mut queue: Vec<NodeId> = indegree
        .iter()
        .filter_map(|(k, &v)| if v == 0 { Some(k.clone()) } else { None })
        .collect();
    queue.sort(); // determinism

    let mut order = Vec::with_capacity(reachable.len());
    while let Some(n) = queue.pop() {
        order.push(n.clone());
        if let Some(deps) = dependents.get(&n) {
            for d in deps {
                let entry = indegree.get_mut(d).unwrap();
                *entry -= 1;
                if *entry == 0 {
                    queue.push(d.clone());
                }
            }
        }
    }

    if order.len() != reachable.len() {
        return Err(anyhow!("graph has a cycle"));
    }
    Ok(order)
}

pub(crate) mod readback {
    use anyhow::Result;

    use crate::engine::GpuContext;
    use crate::project::ToneMap;

    /// Copy a GPU texture into a Vec<u8> of RGBA8 pixels, applying the
    /// requested tone map and an sRGB-ish 2.2 gamma so HDR-range values land
    /// in the 0..255 range FFmpeg expects.
    pub(crate) fn texture_to_rgba8(
        gpu: &GpuContext,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        tone_map: ToneMap,
    ) -> Result<Vec<u8>> {
        // wgpu requires buffer rows aligned to COPY_BYTES_PER_ROW_ALIGNMENT (256).
        // Derive bytes-per-pixel from the format so this stays correct if the
        // engine's working format changes (e.g. Rgba32Float for deeper HDR).
        let bytes_per_pixel = texture
            .format()
            .block_copy_size(None)
            .expect("uncompressed format has a fixed copy size");
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

        let buffer_size = (padded_bytes_per_row * height) as wgpu::BufferAddress;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        gpu.device.poll(wgpu::Maintain::Wait);
        receiver.recv()??;

        let raw = slice.get_mapped_range();

        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let row_start = (y * padded_bytes_per_row) as usize;
            for x in 0..width {
                let px_start = row_start + (x * bytes_per_pixel) as usize;
                let f16_to_f32 =
                    |b: &[u8]| -> f32 { half::f16::from_le_bytes([b[0], b[1]]).to_f32() };
                let r = f16_to_f32(&raw[px_start..px_start + 2]);
                let g = f16_to_f32(&raw[px_start + 2..px_start + 4]);
                let b = f16_to_f32(&raw[px_start + 4..px_start + 6]);
                let a = f16_to_f32(&raw[px_start + 6..px_start + 8]);

                let mapped = match tone_map {
                    ToneMap::Aces => [aces(r), aces(g), aces(b)],
                    ToneMap::Reinhard => [reinhard(r), reinhard(g), reinhard(b)],
                    ToneMap::None => [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)],
                };
                out.push(to_u8_gamma(mapped[0]));
                out.push(to_u8_gamma(mapped[1]));
                out.push(to_u8_gamma(mapped[2]));
                out.push((a.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }

        drop(raw);
        buffer.unmap();
        Ok(out)
    }

    /// ACES filmic tone-map approximation (Krzysztof Narkowicz). Operates
    /// per-channel; preserves highlight saturation better than Reinhard at
    /// the cost of a slightly punchier rendering of mid-greys.
    fn aces(x: f32) -> f32 {
        let a = 2.51_f32;
        let b = 0.03_f32;
        let c = 2.43_f32;
        let d = 0.59_f32;
        let e = 0.14_f32;
        ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
    }

    /// Simple Reinhard `x / (1 + x)`. Cheap, smooth, but desaturates bright
    /// RGB.
    fn reinhard(x: f32) -> f32 {
        let m = x / (1.0 + x);
        m.clamp(0.0, 1.0)
    }

    /// Apply the standard 2.2 display gamma and pack to 8-bit.
    fn to_u8_gamma(x: f32) -> u8 {
        (x.powf(1.0 / 2.2) * 255.0).round() as u8
    }
}
