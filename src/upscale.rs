//! Presents a window's canvas stretched over the whole window: sharp-bilinear when enlarging,
//! averaged when shrinking.
//!
//! `pixels`' own renderer keeps the aspect ratio and clips to whole screen pixels, so a skin
//! 275 pixels wide shown at 1.5x (412.5 screen pixels, in a 413-pixel window) loses its last
//! column. This renderer maps the canvas edges exactly onto the window edges instead; with
//! window sizes rounded to whole screen pixels, one axis may stretch by under a pixel more than
//! the other.
//!
//! When the canvas is coarser than the screen, the shader samples each canvas pixel's centre,
//! and within one screen pixel of a canvas-pixel edge it blends linearly into the neighbour.
//! Nearest-neighbour at a fractional scale would make some rows and columns a screen pixel
//! wider than others; this keeps them even with a one-pixel soft edge. At whole-number scales
//! every sample lands on a texel centre, so the output is exactly nearest-neighbour.
//!
//! A canvas drawn finer than the screen (a density rounded up from a fractional scale, such as
//! 2 at 1.5x) is shrunk instead: each screen pixel averages the canvas pixels under it.
//!
//! The canvas is premultiplied (masked-out pixels are all zero, everything else is opaque),
//! so filtered samples are written as they are.

use pixels::{Pixels, wgpu};

const SHADER: &str = r"
@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var canvas_sampler: sampler;

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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(canvas));
    let texel = in.uv * size;
    // Canvas pixels per screen pixel along each axis.
    let step = vec2<f32>(abs(dpdx(texel.x)), abs(dpdy(texel.y)));
    if (step.x > 1.0 || step.y > 1.0) {
        // Shrinking: four bilinear taps a quarter of the screen pixel's footprint from its
        // centre together weigh each canvas pixel by about how much of the footprint it covers.
        let d = max(step, vec2<f32>(1.0)) * 0.25;
        let sum = textureSampleLevel(canvas, canvas_sampler, (texel - d) / size, 0.0)
            + textureSampleLevel(canvas, canvas_sampler, (texel + vec2<f32>(d.x, -d.y)) / size, 0.0)
            + textureSampleLevel(canvas, canvas_sampler, (texel + vec2<f32>(-d.x, d.y)) / size, 0.0)
            + textureSampleLevel(canvas, canvas_sampler, (texel + d) / size, 0.0);
        return sum * 0.25;
    }
    let f = fract(texel);
    let offset = clamp(f / step, vec2<f32>(0.0), vec2<f32>(0.5))
        + clamp((f - 1.0) / step + 0.5, vec2<f32>(0.0), vec2<f32>(0.5));
    return textureSampleLevel(canvas, canvas_sampler, (floor(texel) + offset) / size, 0.0);
}
";

/// The GPU state for drawing one window's canvas.
#[derive(Debug)]
pub(crate) struct Upscaler {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
}

impl Upscaler {
    /// Build the pipeline on `pixels`' GPU device, drawing to its surface format and reading
    /// its current canvas texture.
    #[must_use]
    pub(crate) fn new(pixels: &Pixels<'_>) -> Self {
        let device = pixels.device();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("upscale_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("upscale_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 1.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscale_bind_group_layout"),
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
            label: Some("upscale_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("upscale_pipeline"),
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
                    format: pixels.render_texture_format(),
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bind_group = bind(device, &layout, &sampler, pixels);
        Self {
            pipeline,
            layout,
            sampler,
            bind_group,
        }
    }

    /// Point at the current canvas texture. [`Pixels::resize_buffer`] replaces the texture, so
    /// call this after every buffer resize.
    pub(crate) fn rebind(&mut self, pixels: &Pixels<'_>) {
        self.bind_group = bind(pixels.device(), &self.layout, &self.sampler, pixels);
    }

    /// Upload the frame and draw it over the whole window.
    ///
    /// # Errors
    /// Fails like [`Pixels::render`], e.g. when the surface cannot be acquired.
    pub(crate) fn render(&self, pixels: &Pixels<'_>) -> Result<(), pixels::Error> {
        pixels.render_with(|encoder, target, _| {
            self.draw(encoder, target);
            Ok(())
        })
    }

    /// Draw the uploaded frame over the whole of `target`, for [`Pixels::render_with`].
    pub(crate) fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("upscale_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    pixels: &Pixels<'_>,
) -> wgpu::BindGroup {
    let view = pixels
        .texture()
        .create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("upscale_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// The canvas pixel under a window position, for a canvas stretched over the whole window as
/// [`Upscaler`] draws it. Positions outside the window clamp to the nearest edge pixel.
#[must_use]
pub(crate) fn window_to_canvas(
    pos: (f64, f64),
    window: (u32, u32),
    canvas: (i32, i32),
) -> (i32, i32) {
    let axis = |p: f64, win: u32, len: i32| {
        let last = len.saturating_sub(1).max(0);
        let v = (p * f64::from(len) / f64::from(win.max(1))).floor();
        if v.is_nan() || v < 0.0 {
            0
        } else if v >= f64::from(last) {
            last
        } else {
            // A whole number in 0..last, so the cast is exact.
            v as i32
        }
    };
    (
        axis(pos.0, window.0, canvas.0),
        axis(pos.1, window.1, canvas.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_positions_map_onto_the_stretched_canvas() {
        // At 1.5x skin pixel 0 covers window positions 0 to 1.5, skin pixel 1 the next 1.5.
        assert_eq!(window_to_canvas((1.9, 0.0), (413, 174), (275, 116)), (1, 0));
        assert_eq!(window_to_canvas((1.4, 1.4), (413, 174), (275, 116)), (0, 0));
        assert_eq!(
            window_to_canvas((412.9, 173.9), (413, 174), (275, 116)),
            (274, 115)
        );
        // Whole-number scales divide exactly.
        assert_eq!(
            window_to_canvas((81.0, 81.0), (550, 232), (275, 116)),
            (40, 40)
        );
        // Degenerate input stays in bounds.
        assert_eq!(
            window_to_canvas((f64::NAN, 3.0), (413, 174), (0, 0)),
            (0, 0)
        );
        // Outside the window clamps to the edges.
        assert_eq!(
            window_to_canvas((-5.0, 900.0), (413, 174), (275, 116)),
            (0, 115)
        );
    }
}
