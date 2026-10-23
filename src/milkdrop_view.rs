//! MilkDrop's picture in its window: drawn by [`crate::milkdrop::gpu`] on the window's own GPU
//! device, at the size it covers on the screen (up to a limit), then laid over the window
//! after the skin's frame, stretched to fit.

use std::time::Instant;

use pixels::{Pixels, wgpu};

use crate::canvas::Rect;
use crate::milkdrop::gpu::{Gpu, TextureLoader};
use crate::milkdrop::visualizer::Visualizer;

/// The most pixels MilkDrop draws, about 1920 x 1200; a larger picture is enlarged from that.
const MAX_PIXELS: u64 = 2_304_000;

/// Draws the picture into the viewport. MilkDrop's colours are as the screen shows them, so
/// they are converted to linear light for a surface that encodes to sRGB itself.
const SHADER: &str = r"
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One triangle covering the viewport; uv runs 0..1 across it, top-left to bottom-right.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VertexOutput {
    let corner = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(picture, picture_sampler, in.uv, 0.0).rgb;
    return vec4<f32>(select(c, to_linear(c), SRGB_TARGET), 1.0);
}
";

/// MilkDrop drawing in one window.
pub(crate) struct MilkdropView {
    gpu: Gpu,
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl std::fmt::Debug for MilkdropView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MilkdropView")
            .field("gpu", &self.gpu)
            .finish_non_exhaustive()
    }
}

/// The size MilkDrop draws at for a picture `w` x `h` screen pixels: the same, or smaller in
/// the same shape when that is more than [`MAX_PIXELS`].
fn picture_size(w: u32, h: u32) -> (u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    let area = u64::from(w) * u64::from(h);
    if area <= MAX_PIXELS {
        return (w, h);
    }
    let k = (MAX_PIXELS as f64 / area as f64).sqrt();
    // Smaller than the sides given, so the casts cannot overflow.
    let scale = |v: u32| ((f64::from(v) * k).floor() as u32).max(1);
    (scale(w), scale(h))
}

impl MilkdropView {
    /// Set up on `pixels`' GPU device, drawing to its surface, with presets' pictures from
    /// `loader`.
    pub(crate) fn new(pixels: &Pixels<'_>, loader: TextureLoader) -> MilkdropView {
        let device = pixels.device().clone();
        let mut gpu = Gpu::new(&device, pixels.queue(), (16, 16));
        gpu.set_texture_loader(loader);
        let format = pixels.render_texture_format();
        let source = format!("const SRGB_TARGET: bool = {};\n{SHADER}", format.is_srgb());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("milkdrop_present_shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("milkdrop_present_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("milkdrop_present_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("milkdrop_present_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("milkdrop_present_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        MilkdropView {
            gpu,
            device,
            pipeline,
            layout,
            sampler,
        }
    }

    /// Work out MilkDrop's frame at `now` and draw it, for a picture covering `area` screen
    /// pixels.
    pub(crate) fn draw(&mut self, vis: &mut Visualizer, now: Instant, area: (u32, u32)) {
        vis.render(now, picture_size(area.0, area.1));
        self.gpu.draw(vis.engine());
    }

    /// Lay the last frame drawn over `area` of `target` (in its pixels), for
    /// [`Pixels::render_with`]. An area outside the target is cut to it.
    pub(crate) fn present(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        area: Rect,
    ) {
        let size = target.texture().size();
        let bounds = Rect::new(
            0,
            0,
            i32::try_from(size.width).unwrap_or(i32::MAX),
            i32::try_from(size.height).unwrap_or(i32::MAX),
        );
        let area = area.intersect(&bounds);
        if area.is_empty() {
            return;
        }
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("milkdrop_present_bind_group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(self.gpu.output()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("milkdrop_present_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            area.x as f32,
            area.y as f32,
            area.w as f32,
            area.h as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_pictures_are_drawn_smaller_in_the_same_shape() {
        assert_eq!(picture_size(640, 480), (640, 480));
        assert_eq!(picture_size(0, 0), (1, 1));
        let (w, h) = picture_size(3840, 2160);
        assert!(u64::from(w) * u64::from(h) <= MAX_PIXELS);
        assert!((f64::from(w) / f64::from(h) - 16.0 / 9.0).abs() < 0.01);
        assert!(w > 1900);
    }
}
