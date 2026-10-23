//! Drawing MilkDrop's frames with wgpu: two pictures that take turns being the previous frame
//! and the new one, the blur textures, the noise textures and presets' own images, and the
//! pipelines for each preset's shaders (translated by [`hlsl`]) and for the drawing over each
//! frame. [`Gpu::draw`] carries out an [`Engine`] frame and leaves the picture in
//! [`Gpu::output`].

use std::collections::HashMap;

use pixels::wgpu;

use super::draw::{Blend, Kind};
use super::engine::{Engine, Frame, Pass, PassBlend, PresetId, ShaderInputs};
use super::hlsl::{self, Filter, Stage, Texture};
use super::noise::noise;
use super::preset::Preset;

/// The pictures' format: eight bits a channel, as MilkDrop's were.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Blur texture sizes as fractions of the picture: each level's horizontal and vertical pass
/// (butterchurn's `blurRatios`).
const BLUR_RATIOS: [(f32, f32); 3] = [(0.5, 0.25), (0.125, 0.125), (0.0625, 0.0625)];

/// MilkDrop's blur weights (`BlurPasses`).
const BLUR_W: [f32; 8] = [4.0, 3.8, 3.5, 2.9, 1.9, 1.2, 0.7, 0.3];

/// Loads a preset's texture by name (`worms`, `clouds`...): RGBA rows and the size.
pub type TextureLoader = Box<dyn FnMut(&str) -> Option<(Vec<u8>, u32, u32)>>;

/// A texture that can be drawn into and sampled.
#[derive(Debug)]
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

impl Target {
    fn new(device: &wgpu::Device, size: (u32, u32), label: &str) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Target {
            texture,
            view,
            size,
        }
    }
}

/// A texture made from pixels: noise, or a preset's image.
fn image_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    data: &[u8],
    size: (u32, u32, u32),
    label: &str,
) -> wgpu::TextureView {
    let (w, h, d) = size;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: d,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: if d > 1 {
            wgpu::TextureDimension::D3
        } else {
            wgpu::TextureDimension::D2
        },
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * w),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: d,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The size of a blur texture for a fraction of the picture (butterchurn's
/// `getTextureSize`).
fn blur_size(size: (u32, u32), ratio: f32) -> (u32, u32) {
    let x = (size.0 as f32 * ratio).max(16.0);
    let y = (size.1 as f32 * ratio).max(16.0);
    // Positive and modest, so the casts are exact.
    (
        ((x + 3.0) / 16.0).floor() as u32 * 16,
        ((y + 3.0) / 4.0).floor() as u32 * 4,
    )
}

/// Write `values` as little-endian bytes onto `out`.
fn push_f32s(out: &mut Vec<u8>, values: impl IntoIterator<Item = f32>) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// The uniform block a preset's shaders read (laid out as [`hlsl::INPUTS_SIZE`] says).
fn inputs_bytes(i: &ShaderInputs) -> Vec<u8> {
    let mut b = Vec::with_capacity(hlsl::INPUTS_SIZE);
    for c in &i.c {
        push_f32s(&mut b, *c);
    }
    push_f32s(&mut b, i.q);
    let flag = |f: bool| if f { 1.0 } else { 0.0 };
    push_f32s(&mut b, [i.decay, i.gamma, i.echo_zoom, i.echo_alpha]);
    push_f32s(
        &mut b,
        [
            i.echo_orient,
            i.hue_shader,
            flag(i.brighten),
            flag(i.darken),
        ],
    );
    push_f32s(
        &mut b,
        [flag(i.solarize), flag(i.invert), flag(i.wrap), 0.0],
    );
    push_f32s(&mut b, i.rand_frame);
    push_f32s(&mut b, i.rand_preset);
    for m in &i.rot {
        for row in m {
            push_f32s(&mut b, [row[0], row[1], row[2], 0.0]);
        }
    }
    b
}

/// One preset's warp or composite shader, ready to draw.
#[derive(Debug)]
struct Program {
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    user: Option<wgpu::BindGroup>,
    blur: u8,
    pipelines: HashMap<PassBlend, wgpu::RenderPipeline>,
}

/// A vertex of the warp and composite meshes: position, texture coordinate, colour.
const MESH_STRIDE: u64 = 32;

const DRAW_SHADER: &str = "
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>, @location(2) uv: vec2<f32>) -> VOut {
    var o: VOut;
    // MilkDrop's 2D space runs down the screen; wgpu's up.
    o.pos = vec4<f32>(pos.x, -pos.y, 0.0, 1.0);
    o.color = color;
    o.uv = uv;
    return o;
}

@fragment
fn fs_plain(i: VOut) -> @location(0) vec4<f32> {
    return i.color;
}

@fragment
fn fs_textured(i: VOut) -> @location(0) vec4<f32> {
    // Direct3D's modulate: colour times texture, alpha from the colour alone.
    return vec4<f32>(textureSample(tex, samp, i.uv).rgb * i.color.rgb, i.color.a);
}
";

const BLUR_SHADER: &str = "
struct Blur {
    texsize: vec4<f32>,
    ws: vec4<f32>,
    ds: vec4<f32>,
    // scale, bias, w_div
    misc: vec4<f32>,
    // The edge darkening: ed1, ed2, ed3.
    ed: vec4<f32>,
}

@group(0) @binding(0) var<uniform> b: Blur;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    return o;
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv, 0.0).xyz;
}

@fragment
fn fs_horizontal(i: VOut) -> @location(0) vec4<f32> {
    let dx = b.texsize.z;
    var blur = (tap(i.uv + vec2<f32>(b.ds.x * dx, 0.0)) + tap(i.uv - vec2<f32>(b.ds.x * dx, 0.0))) * b.ws.x
        + (tap(i.uv + vec2<f32>(b.ds.y * dx, 0.0)) + tap(i.uv - vec2<f32>(b.ds.y * dx, 0.0))) * b.ws.y
        + (tap(i.uv + vec2<f32>(b.ds.z * dx, 0.0)) + tap(i.uv - vec2<f32>(b.ds.z * dx, 0.0))) * b.ws.z
        + (tap(i.uv + vec2<f32>(b.ds.w * dx, 0.0)) + tap(i.uv - vec2<f32>(b.ds.w * dx, 0.0))) * b.ws.w;
    blur = blur * b.misc.z;
    blur = blur * b.misc.x + b.misc.y;
    return vec4<f32>(blur, 1.0);
}

@fragment
fn fs_vertical(i: VOut) -> @location(0) vec4<f32> {
    let dy = b.texsize.w;
    var blur = (tap(i.uv + vec2<f32>(0.0, b.ws.z * dy)) + tap(i.uv - vec2<f32>(0.0, b.ws.z * dy))) * b.ws.x
        + (tap(i.uv + vec2<f32>(0.0, b.ws.w * dy)) + tap(i.uv - vec2<f32>(0.0, b.ws.w * dy))) * b.ws.y;
    blur = blur * b.misc.z;
    var t = min(min(i.uv.x, i.uv.y), 1.0 - max(i.uv.x, i.uv.y));
    t = sqrt(max(t, 0.0));
    t = b.ed.x + b.ed.y * clamp(t * b.ed.z, 0.0, 1.0);
    return vec4<f32>(blur * t, 1.0);
}
";

/// Everything needed to draw MilkDrop on one wgpu device.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: (u32, u32),
    frames: [Target; 2],
    /// Which of `frames` holds the previous frame.
    current: usize,
    /// Each blur level's horizontal and vertical texture.
    blur: [(Target, Target); 3],
    display: Target,
    noise: HashMap<Texture, wgpu::TextureView>,
    samplers: HashMap<Filter, wgpu::Sampler>,
    group0: wgpu::BindGroupLayout,
    user_layouts: HashMap<usize, wgpu::BindGroupLayout>,
    user_textures: HashMap<String, wgpu::TextureView>,
    loader: Option<TextureLoader>,
    programs: HashMap<(PresetId, Stage), Program>,
    defaults: HashMap<Stage, Program>,
    uniforms: [wgpu::Buffer; 4],
    mesh_index: (wgpu::Buffer, u32),
    comp_index: (wgpu::Buffer, u32),
    mesh_vertices: wgpu::Buffer,
    comp_vertices: wgpu::Buffer,
    draw_vertices: (wgpu::Buffer, u64),
    motion_vertices: (wgpu::Buffer, u64),
    draw_layout: wgpu::BindGroupLayout,
    draw_pipelines: HashMap<(Kind, Blend), wgpu::RenderPipeline>,
    blur_layout: wgpu::BindGroupLayout,
    blur_pipelines: [wgpu::RenderPipeline; 2],
    blur_uniforms: Vec<wgpu::Buffer>,
    scratch: Vec<u8>,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gpu")
            .field("size", &self.size)
            .field("programs", &self.programs.len())
            .finish_non_exhaustive()
    }
}

fn grid_indices(grid: (usize, usize)) -> Vec<u8> {
    let (gx, gy) = grid;
    let mut out = Vec::with_capacity(gx * gy * 6 * 4);
    for y in 0..gy {
        for x in 0..gx {
            let a = y * (gx + 1) + x;
            let b = a + gx + 1;
            for i in [a, b, a + 1, b, b + 1, a + 1] {
                // The grids are far smaller than 2^32 vertices.
                out.extend_from_slice(&(i as u32).to_le_bytes());
            }
        }
    }
    out
}

fn buffer(
    device: &wgpu::Device,
    size: u64,
    usage: wgpu::BufferUsages,
    label: &str,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn mesh_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x2, 1 => Float32x2, 2 => Float32x4
    ];
    wgpu::VertexBufferLayout {
        array_stride: MESH_STRIDE,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    }
}

fn blend_state(b: PassBlend) -> Option<wgpu::BlendState> {
    let color = |src, dst| wgpu::BlendComponent {
        src_factor: src,
        dst_factor: dst,
        operation: wgpu::BlendOperation::Add,
    };
    let state = |c| Some(wgpu::BlendState { color: c, alpha: c });
    match b {
        PassBlend::Replace => None,
        PassBlend::Alpha => state(color(
            wgpu::BlendFactor::SrcAlpha,
            wgpu::BlendFactor::OneMinusSrcAlpha,
        )),
        PassBlend::FlippedAlpha => state(color(
            wgpu::BlendFactor::OneMinusSrcAlpha,
            wgpu::BlendFactor::SrcAlpha,
        )),
    }
}

/// The colour channels drawn: the pictures keep alpha at 1, like MilkDrop's alpha-less ones.
const RGB: wgpu::ColorWrites = wgpu::ColorWrites::COLOR;

impl Gpu {
    /// Set up on `device`, for a picture `size` pixels across and down.
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: (u32, u32)) -> Gpu {
        let size = (size.0.max(1), size.1.max(1));
        let device = device.clone();
        let queue = queue.clone();
        let frames = [
            Target::new(&device, size, "milkdrop_frame_a"),
            Target::new(&device, size, "milkdrop_frame_b"),
        ];
        let blur = Self::blur_targets(&device, size);
        let display = Target::new(&device, size, "milkdrop_display");

        let mut noise_views = HashMap::new();
        for (t, side, dims, zoom, seed) in [
            (Texture::NoiseLq, 256, 2, 1, 11),
            (Texture::NoiseLqLite, 32, 2, 1, 12),
            (Texture::NoiseMq, 256, 2, 4, 13),
            (Texture::NoiseHq, 256, 2, 8, 14),
            (Texture::NoiseVolLq, 32, 3, 1, 15),
            (Texture::NoiseVolHq, 32, 3, 4, 16),
        ] {
            let data = noise(side, dims, zoom, seed);
            let depth = if dims == 3 { side as u32 } else { 1 };
            let view = image_texture(
                &device,
                &queue,
                &data,
                (side as u32, side as u32, depth),
                "milkdrop_noise",
            );
            noise_views.insert(t, view);
        }

        let sampler = |filter: wgpu::FilterMode, address: wgpu::AddressMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("milkdrop_sampler"),
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..wgpu::SamplerDescriptor::default()
            })
        };
        let samplers = HashMap::from([
            (
                Filter::LinearWrap,
                sampler(wgpu::FilterMode::Linear, wgpu::AddressMode::Repeat),
            ),
            (
                Filter::LinearClamp,
                sampler(wgpu::FilterMode::Linear, wgpu::AddressMode::ClampToEdge),
            ),
            (
                Filter::PointWrap,
                sampler(wgpu::FilterMode::Nearest, wgpu::AddressMode::Repeat),
            ),
            (
                Filter::PointClamp,
                sampler(wgpu::FilterMode::Nearest, wgpu::AddressMode::ClampToEdge),
            ),
        ]);

        let fragment = wgpu::ShaderStages::FRAGMENT;
        let tex_entry = |binding: u32, volume: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: if volume {
                    wgpu::TextureViewDimension::D3
                } else {
                    wgpu::TextureViewDimension::D2
                },
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let uniform_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let mut entries = vec![uniform_entry(hlsl::INPUTS_BINDING)];
        let builtins = [
            Texture::Main,
            Texture::Blur(1),
            Texture::Blur(2),
            Texture::Blur(3),
            Texture::NoiseLq,
            Texture::NoiseLqLite,
            Texture::NoiseMq,
            Texture::NoiseHq,
            Texture::NoiseVolLq,
            Texture::NoiseVolHq,
        ];
        for t in &builtins {
            if let Some(b) = hlsl::binding(t) {
                entries.push(tex_entry(b, t.is_volume()));
            }
        }
        for f in [
            Filter::LinearWrap,
            Filter::LinearClamp,
            Filter::PointWrap,
            Filter::PointClamp,
        ] {
            entries.push(sampler_entry(hlsl::sampler_binding(f)));
        }
        let group0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("milkdrop_group0"),
            entries: &entries,
        });

        let uniforms = std::array::from_fn(|_| {
            buffer(
                &device,
                hlsl::INPUTS_SIZE as u64,
                wgpu::BufferUsages::UNIFORM,
                "milkdrop_inputs",
            )
        });
        let index = |grid, label: &str| {
            let data = grid_indices(grid);
            let b = buffer(&device, data.len() as u64, wgpu::BufferUsages::INDEX, label);
            queue.write_buffer(&b, 0, &data);
            (b, (data.len() / 4) as u32)
        };
        let mesh_index = index(super::engine::GRID, "milkdrop_mesh_index");
        let comp_index = index(super::engine::COMP_GRID, "milkdrop_comp_index");
        let vertices = |grid: (usize, usize), label: &str| {
            buffer(
                &device,
                ((grid.0 + 1) * (grid.1 + 1)) as u64 * MESH_STRIDE,
                wgpu::BufferUsages::VERTEX,
                label,
            )
        };
        let mesh_vertices = vertices(super::engine::GRID, "milkdrop_mesh");
        let comp_vertices = vertices(super::engine::COMP_GRID, "milkdrop_comp");

        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("milkdrop_draw_layout"),
            entries: &[tex_entry(0, false), sampler_entry(1)],
        });
        let draw_pipelines = Self::draw_pipelines(&device, &draw_layout);
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("milkdrop_blur_layout"),
            entries: &[uniform_entry(0), tex_entry(1, false), sampler_entry(2)],
        });
        let blur_pipelines = Self::blur_pipelines(&device, &blur_layout);
        let blur_uniforms = (0..6)
            .map(|_| buffer(&device, 80, wgpu::BufferUsages::UNIFORM, "milkdrop_blur"))
            .collect();

        let mut gpu = Gpu {
            draw_vertices: (
                buffer(&device, 4096, wgpu::BufferUsages::VERTEX, "milkdrop_draw"),
                4096,
            ),
            motion_vertices: (
                buffer(&device, 4096, wgpu::BufferUsages::VERTEX, "milkdrop_motion"),
                4096,
            ),
            device,
            queue,
            size,
            frames,
            current: 0,
            blur,
            display,
            noise: noise_views,
            samplers,
            group0,
            user_layouts: HashMap::new(),
            user_textures: HashMap::new(),
            loader: None,
            programs: HashMap::new(),
            defaults: HashMap::new(),
            uniforms,
            mesh_index,
            comp_index,
            mesh_vertices,
            comp_vertices,
            draw_layout,
            draw_pipelines,
            blur_layout,
            blur_pipelines,
            blur_uniforms,
            scratch: Vec::new(),
        };
        for (stage, src) in [
            (Stage::Warp, hlsl::DEFAULT_WARP),
            (Stage::Comp, hlsl::DEFAULT_COMP),
        ] {
            match gpu.program(src, stage) {
                Ok(p) => {
                    gpu.defaults.insert(stage, p);
                }
                Err(e) => log::error!("milkdrop: the built-in {stage:?} shader failed: {e}"),
            }
        }
        gpu
    }

    /// Where presets' textures come from.
    pub fn set_texture_loader(&mut self, loader: TextureLoader) {
        self.loader = Some(loader);
        self.user_textures.clear();
    }

    fn blur_targets(device: &wgpu::Device, size: (u32, u32)) -> [(Target, Target); 3] {
        BLUR_RATIOS.map(|(h, v)| {
            (
                Target::new(device, blur_size(size, h), "milkdrop_blur_h"),
                Target::new(device, blur_size(size, v), "milkdrop_blur_v"),
            )
        })
    }

    fn draw_pipelines(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> HashMap<(Kind, Blend), wgpu::RenderPipeline> {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("milkdrop_draw"),
            source: wgpu::ShaderSource::Wgsl(DRAW_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("milkdrop_draw"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        const ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x4, 2 => Float32x2
        ];
        let mut out = HashMap::new();
        for kind in [Kind::Lines, Kind::Triangles, Kind::Textured] {
            for blend in [Blend::Alpha, Blend::Add] {
                let dst = match blend {
                    Blend::Alpha => wgpu::BlendFactor::OneMinusSrcAlpha,
                    Blend::Add => wgpu::BlendFactor::One,
                };
                let component = wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: dst,
                    operation: wgpu::BlendOperation::Add,
                };
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("milkdrop_draw"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_main"),
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: 32,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &ATTRS,
                        }],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: if kind == Kind::Lines {
                            wgpu::PrimitiveTopology::LineList
                        } else {
                            wgpu::PrimitiveTopology::TriangleList
                        },
                        ..wgpu::PrimitiveState::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(if kind == Kind::Textured {
                            "fs_textured"
                        } else {
                            "fs_plain"
                        }),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: FORMAT,
                            blend: Some(wgpu::BlendState {
                                color: component,
                                alpha: component,
                            }),
                            write_mask: RGB,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                out.insert((kind, blend), pipeline);
            }
        }
        out
    }

    fn blur_pipelines(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
    ) -> [wgpu::RenderPipeline; 2] {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("milkdrop_blur"),
            source: wgpu::ShaderSource::Wgsl(BLUR_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("milkdrop_blur"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        ["fs_horizontal", "fs_vertical"].map(|entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("milkdrop_blur"),
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
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// Translate and compile a shader, with its preset textures bound.
    fn program(&mut self, src: &str, stage: Stage) -> Result<Program, String> {
        let t = hlsl::translate(src, stage).map_err(|e| e.to_string())?;
        hlsl::validate(&t.wgsl)?;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("milkdrop_preset"),
                source: wgpu::ShaderSource::Wgsl(t.wgsl.as_str().into()),
            });
        let n = t.user_textures.len();
        let user_layout = if n > 0 {
            let fragment = wgpu::ShaderStages::FRAGMENT;
            let layout = self.user_layouts.entry(n).or_insert_with(|| {
                let entries: Vec<wgpu::BindGroupLayoutEntry> = (0..n)
                    .map(|i| wgpu::BindGroupLayoutEntry {
                        binding: i as u32,
                        visibility: fragment,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    })
                    .collect();
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("milkdrop_user_textures"),
                        entries: &entries,
                    })
            });
            Some(layout.clone())
        } else {
            None
        };
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("milkdrop_preset"),
                bind_group_layouts: &[Some(&self.group0), user_layout.as_ref()],
                immediate_size: 0,
            });
        let user = match &user_layout {
            Some(l) => {
                let views: Vec<wgpu::TextureView> = t
                    .user_textures
                    .iter()
                    .map(|name| self.user_texture(name))
                    .collect();
                let entries: Vec<wgpu::BindGroupEntry> = views
                    .iter()
                    .enumerate()
                    .map(|(i, v)| wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource: wgpu::BindingResource::TextureView(v),
                    })
                    .collect();
                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("milkdrop_user_textures"),
                    layout: l,
                    entries: &entries,
                }))
            }
            None => None,
        };
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(e.to_string());
        }
        Ok(Program {
            module,
            layout,
            user,
            blur: t.blur,
            pipelines: HashMap::new(),
        })
    }

    /// A preset's texture, loaded once; missing ones show noise.
    fn user_texture(&mut self, name: &str) -> wgpu::TextureView {
        if let Some(v) = self.user_textures.get(name) {
            return v.clone();
        }
        let loaded = self.loader.as_mut().and_then(|l| l(name));
        let view = match loaded {
            Some((data, w, h)) if w > 0 && h > 0 && data.len() == (w * h * 4) as usize => {
                image_texture(&self.device, &self.queue, &data, (w, h, 1), "milkdrop_user")
            }
            _ => {
                log::info!("milkdrop: no texture '{name}', using noise");
                self.noise[&Texture::NoiseMq].clone()
            }
        };
        self.user_textures.insert(name.to_owned(), view.clone());
        view
    }

    fn pipeline(
        &mut self,
        key: Option<(PresetId, Stage)>,
        stage: Stage,
        blend: PassBlend,
    ) -> Option<wgpu::RenderPipeline> {
        let device = self.device.clone();
        let program = match key {
            Some(k) => self.programs.get_mut(&k)?,
            None => self.defaults.get_mut(&stage)?,
        };
        if let Some(p) = program.pipelines.get(&blend) {
            return Some(p.clone());
        }
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("milkdrop_preset"),
            layout: Some(&program.layout),
            vertex: wgpu::VertexState {
                module: &program.module,
                entry_point: Some("vs_main"),
                buffers: &[mesh_vertex_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &program.module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: blend_state(blend),
                    write_mask: if stage == Stage::Warp {
                        RGB
                    } else {
                        wgpu::ColorWrites::ALL
                    },
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        program.pipelines.insert(blend, p.clone());
        Some(p)
    }

    /// Make sure a preset's shaders are compiled, falling back to MilkDrop 1's built-in ones
    /// for a stage the preset has no shader for, or one that does not translate.
    fn prepare(&mut self, id: PresetId, preset: &Preset) {
        for (stage, src, ps) in [
            (Stage::Warp, &preset.warp, preset.warp_ps),
            (Stage::Comp, &preset.comp, preset.comp_ps),
        ] {
            if self.programs.contains_key(&(id, stage)) {
                continue;
            }
            let Some(src) = src.as_ref().filter(|_| ps > 0) else {
                continue;
            };
            match self.program(src, stage) {
                Ok(p) => {
                    self.programs.insert((id, stage), p);
                }
                Err(e) => log::warn!("milkdrop: {stage:?} shader not used: {e}"),
            }
        }
    }

    /// The picture after the last [`Gpu::draw`].
    #[must_use]
    pub fn output(&self) -> &wgpu::TextureView {
        &self.display.view
    }

    /// The texture behind [`Gpu::output`], for reading back.
    #[must_use]
    pub fn output_texture(&self) -> &wgpu::Texture {
        &self.display.texture
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    fn resize(&mut self, size: (u32, u32)) {
        self.size = size;
        self.frames = [
            Target::new(&self.device, size, "milkdrop_frame_a"),
            Target::new(&self.device, size, "milkdrop_frame_b"),
        ];
        self.blur = Self::blur_targets(&self.device, size);
        self.display = Target::new(&self.device, size, "milkdrop_display");
    }

    fn write_grow(&mut self, which: bool, data: &[u8]) {
        let (buf, cap) = if which {
            &mut self.draw_vertices
        } else {
            &mut self.motion_vertices
        };
        let need = data.len() as u64;
        if need > *cap {
            let cap2 = need.next_power_of_two();
            *buf = buffer(
                &self.device,
                cap2,
                wgpu::BufferUsages::VERTEX,
                "milkdrop_draw",
            );
            *cap = cap2;
        }
        if !data.is_empty() {
            self.queue.write_buffer(buf, 0, data);
        }
    }

    fn group0(&self, uniform: &wgpu::Buffer, main: &wgpu::TextureView) -> wgpu::BindGroup {
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: hlsl::INPUTS_BINDING,
            resource: uniform.as_entire_binding(),
        }];
        let blur_views = [
            &self.blur[0].1.view,
            &self.blur[1].1.view,
            &self.blur[2].1.view,
        ];
        let textures: [(Texture, &wgpu::TextureView); 10] = [
            (Texture::Main, main),
            (Texture::Blur(1), blur_views[0]),
            (Texture::Blur(2), blur_views[1]),
            (Texture::Blur(3), blur_views[2]),
            (Texture::NoiseLq, &self.noise[&Texture::NoiseLq]),
            (Texture::NoiseLqLite, &self.noise[&Texture::NoiseLqLite]),
            (Texture::NoiseMq, &self.noise[&Texture::NoiseMq]),
            (Texture::NoiseHq, &self.noise[&Texture::NoiseHq]),
            (Texture::NoiseVolLq, &self.noise[&Texture::NoiseVolLq]),
            (Texture::NoiseVolHq, &self.noise[&Texture::NoiseVolHq]),
        ];
        for (t, view) in textures {
            if let Some(b) = hlsl::binding(&t) {
                entries.push(wgpu::BindGroupEntry {
                    binding: b,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
        }
        for (f, s) in &self.samplers {
            entries.push(wgpu::BindGroupEntry {
                binding: hlsl::sampler_binding(*f),
                resource: wgpu::BindingResource::Sampler(s),
            });
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("milkdrop_group0"),
            layout: &self.group0,
            entries: &entries,
        })
    }

    /// Draw the frame `engine` last worked out, into [`Gpu::output`].
    pub fn draw(&mut self, engine: &Engine) {
        let frame = engine.frame();
        let Some(canvas) = frame.canvas else {
            return;
        };
        if canvas.size != self.size {
            self.resize(canvas.size);
        }
        // Compile shaders for presets seen for the first time; forget presets gone.
        let presets = [engine.current(), engine.previous()];
        for (id, p) in presets.iter().flatten() {
            self.prepare(*id, p);
        }
        self.programs
            .retain(|(id, _), _| presets.iter().flatten().any(|(p, _)| p == id));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("milkdrop"),
            });
        let prev = self.current;
        let next = 1 - prev;
        if frame.clear {
            for f in &self.frames {
                clear(&mut encoder, &f.view);
            }
        }

        // Meshes.
        self.scratch.clear();
        let (gx, gy) = frame.grid;
        for j in 0..=gy {
            for i in 0..=gx {
                let n = j * (gx + 1) + i;
                let pos = [
                    i as f32 / gx as f32 * 2.0 - 1.0,
                    j as f32 / gy as f32 * 2.0 - 1.0,
                ];
                let uv = frame.warp_uv.get(n).copied().unwrap_or([0.0; 2]);
                let a = frame.warp_alpha.get(n).copied().unwrap_or(1.0);
                push_f32s(
                    &mut self.scratch,
                    [pos[0], pos[1], uv[0], uv[1], 1.0, 1.0, 1.0, a],
                );
            }
        }
        self.queue
            .write_buffer(&self.mesh_vertices, 0, &self.scratch);
        self.scratch.clear();
        let (cx, cy) = frame.comp_grid;
        for j in 0..=cy {
            for i in 0..=cx {
                let n = j * (cx + 1) + i;
                let (x, y) = (
                    i as f32 / cx as f32 * 2.0 - 1.0,
                    j as f32 / cy as f32 * 2.0 - 1.0,
                );
                let c = frame.comp_color.get(n).copied().unwrap_or([1.0; 4]);
                push_f32s(
                    &mut self.scratch,
                    [x, y, x * 0.5 + 0.5, 0.5 - y * 0.5, c[0], c[1], c[2], c[3]],
                );
            }
        }
        self.queue
            .write_buffer(&self.comp_vertices, 0, &self.scratch);

        // Uniforms for each pass.
        for (slot, pass) in frame.warp.iter().chain(&frame.comp).enumerate().take(4) {
            let bytes = inputs_bytes(&pass.inputs);
            self.queue.write_buffer(&self.uniforms[slot], 0, &bytes);
        }

        // 1. Motion vectors, into the previous frame.
        if !frame.motion.vertices.is_empty() {
            let bytes = vertex_bytes(&frame.motion.vertices);
            self.write_grow(false, &bytes);
            // Nothing here is textured, so the bound texture only needs to be another one.
            let target = self.frames[prev].view.clone();
            let unused = self.noise[&Texture::NoiseLqLite].clone();
            self.draw_calls(&mut encoder, &frame.motion.calls, false, &target, &unused);
        }

        // 2. The warp, previous frame into the new one.
        let warp_views = (
            self.frames[prev].view.clone(),
            self.frames[next].view.clone(),
        );
        self.mesh_passes(
            &mut encoder,
            frame,
            &frame.warp,
            0,
            Stage::Warp,
            &warp_views.0,
            &warp_views.1,
        );

        // 3. The blur textures, from the previous frame as MilkDrop made them.
        let blur = frame
            .warp
            .iter()
            .chain(&frame.comp)
            .filter_map(|p| {
                [Stage::Warp, Stage::Comp]
                    .iter()
                    .filter_map(|s| self.programs.get(&(p.preset, *s)).map(|pr| pr.blur))
                    .max()
            })
            .max()
            .unwrap_or(0);
        self.blur_passes(&mut encoder, frame, blur, &warp_views.0);

        // 4. Shapes, waves and borders, over the new frame.
        if !frame.drawing.vertices.is_empty() {
            let bytes = vertex_bytes(&frame.drawing.vertices);
            self.write_grow(true, &bytes);
            self.draw_calls(
                &mut encoder,
                &frame.drawing.calls,
                true,
                &warp_views.1,
                &warp_views.0,
            );
        }

        // 5. The composite, to the display.
        let display = self.display.view.clone();
        let first = frame.warp.len().min(2);
        self.mesh_passes(
            &mut encoder,
            frame,
            &frame.comp,
            first,
            Stage::Comp,
            &warp_views.1,
            &display,
        );

        self.queue.submit([encoder.finish()]);
        self.current = next;
    }

    /// Warp or composite passes over a mesh, sampling `main` and drawing into `target`.
    #[allow(clippy::too_many_arguments)]
    fn mesh_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        passes: &[Pass],
        first_slot: usize,
        stage: Stage,
        main: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        let (vertices, index, count) = match stage {
            Stage::Warp => (&self.mesh_vertices, &self.mesh_index.0, self.mesh_index.1),
            Stage::Comp => (&self.comp_vertices, &self.comp_index.0, self.comp_index.1),
        };
        let (vertices, index) = (vertices.clone(), index.clone());
        let _ = frame;
        for (k, pass) in passes.iter().enumerate().take(2) {
            let key = (pass.preset, stage);
            let key = self.programs.contains_key(&key).then_some(key);
            let Some(pipeline) = self.pipeline(key, stage, pass.blend) else {
                continue;
            };
            let user = key
                .and_then(|k| self.programs.get(&k))
                .and_then(|p| p.user.clone());
            let group0 = self.group0(&self.uniforms[(first_slot + k).min(3)], main);
            let load = if k == 0 {
                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
            } else {
                wgpu::LoadOp::Load
            };
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("milkdrop_mesh"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&pipeline);
            rp.set_bind_group(0, &group0, &[]);
            if let Some(u) = &user {
                rp.set_bind_group(1, u, &[]);
            }
            rp.set_vertex_buffer(0, vertices.slice(..));
            rp.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..count, 0, 0..1);
        }
    }

    /// The drawing's calls into `target`; textured ones sample `previous`.
    fn draw_calls(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        calls: &[super::draw::Call],
        drawing: bool,
        target: &wgpu::TextureView,
        previous: &wgpu::TextureView,
    ) {
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("milkdrop_draw"),
            layout: &self.draw_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(previous),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.samplers[&Filter::LinearWrap]),
                },
            ],
        });
        let buf = if drawing {
            &self.draw_vertices.0
        } else {
            &self.motion_vertices.0
        };
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("milkdrop_draw"),
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
        rp.set_bind_group(0, &group, &[]);
        rp.set_vertex_buffer(0, buf.slice(..));
        for call in calls {
            if let Some(p) = self.draw_pipelines.get(&(call.kind, call.blend)) {
                rp.set_pipeline(p);
                rp.draw(call.range.clone(), 0..1);
            }
        }
    }

    /// MilkDrop's `BlurPasses`: each level blurred across, then down, from `source`.
    fn blur_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        levels: u8,
        source: &wgpu::TextureView,
    ) {
        if levels == 0 {
            return;
        }
        let b = frame.blur;
        let scale_bias = blur_scale_bias(&b);
        let ws = BLUR_W;
        for (level, &(scale, bias)) in scale_bias.iter().enumerate().take(usize::from(levels)) {
            let (src_view, src_size) = if level == 0 {
                (source.clone(), self.size)
            } else {
                (
                    self.blur[level - 1].1.view.clone(),
                    self.blur[level - 1].1.size,
                )
            };
            for pass in 0..2 {
                let i = level * 2 + pass;
                let mut u = Vec::with_capacity(80);
                let (input, input_size) = if pass == 0 {
                    (src_view.clone(), src_size)
                } else {
                    (self.blur[level].0.view.clone(), self.blur[level].0.size)
                };
                let (w, h) = (input_size.0 as f32, input_size.1 as f32);
                push_f32s(&mut u, [w, h, 1.0 / w, 1.0 / h]);
                if pass == 0 {
                    let w1 = ws[0] + ws[1];
                    let w2 = ws[2] + ws[3];
                    let w3 = ws[4] + ws[5];
                    let w4 = ws[6] + ws[7];
                    let d = [
                        2.0 * ws[1] / w1,
                        2.0 + 2.0 * ws[3] / w2,
                        4.0 + 2.0 * ws[5] / w3,
                        6.0 + 2.0 * ws[7] / w4,
                    ];
                    let w_div = 0.5 / (w1 + w2 + w3 + w4);
                    push_f32s(&mut u, [w1, w2, w3, w4]);
                    push_f32s(&mut u, d);
                    push_f32s(&mut u, [scale, bias, w_div, 0.0]);
                    push_f32s(&mut u, [1.0, 0.0, 0.0, 0.0]);
                } else {
                    let w1 = ws[0] + ws[1] + ws[2] + ws[3];
                    let w2 = ws[4] + ws[5] + ws[6] + ws[7];
                    let d1 = 2.0 * ((ws[2] + ws[3]) / w1);
                    let d2 = 2.0 + 2.0 * ((ws[6] + ws[7]) / w2);
                    let w_div = 1.0 / ((w1 + w2) * 2.0);
                    push_f32s(&mut u, [w1, w2, d1, d2]);
                    push_f32s(&mut u, [0.0; 4]);
                    push_f32s(&mut u, [1.0, 0.0, w_div, 0.0]);
                    // Only the first level darkens its edges.
                    if level == 0 {
                        push_f32s(&mut u, [1.0 - b.edge_darken, b.edge_darken, 5.0, 0.0]);
                    } else {
                        push_f32s(&mut u, [1.0, 0.0, 5.0, 0.0]);
                    }
                }
                self.queue.write_buffer(&self.blur_uniforms[i], 0, &u);
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("milkdrop_blur"),
                    layout: &self.blur_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.blur_uniforms[i].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&input),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(
                                &self.samplers[&Filter::LinearClamp],
                            ),
                        },
                    ],
                });
                let target = if pass == 0 {
                    &self.blur[level].0.view
                } else {
                    &self.blur[level].1.view
                };
                let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("milkdrop_blur"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_pipeline(&self.blur_pipelines[pass]);
                rp.set_bind_group(0, &group, &[]);
                rp.draw(0..3, 0..1);
            }
        }
    }
}

/// Each blur level's scale and bias, which spread its range over the texture's 0..1
/// (MilkDrop's `fscale` and `fbias`).
fn blur_scale_bias(b: &super::engine::Blur) -> [(f32, f32); 3] {
    let (min, max) = (b.min, b.max);
    let s0 = 1.0 / (max[0] - min[0]);
    let b0 = -min[0] * s0;
    let level = |lo: f32, hi: f32, base_lo: f32, base_hi: f32| {
        let t_min = (lo - base_lo) / (base_hi - base_lo);
        let t_max = (hi - base_lo) / (base_hi - base_lo);
        let s = 1.0 / (t_max - t_min);
        (s, -t_min * s)
    };
    let fix = |(s, b): (f32, f32)| {
        if s.is_finite() && b.is_finite() {
            (s, b)
        } else {
            (1.0, 0.0)
        }
    };
    [
        fix((s0, b0)),
        fix(level(min[1], max[1], min[0], max[0])),
        fix(level(min[2], max[2], min[1], max[1])),
    ]
}

fn vertex_bytes(vertices: &[super::draw::Vertex]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vertices.len() * 32);
    for v in vertices {
        push_f32s(&mut out, [v.pos[0], v.pos[1]]);
        push_f32s(&mut out, v.color);
        push_f32s(&mut out, v.uv);
    }
    out
}

fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
    let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("milkdrop_clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

/// Read a texture back as RGBA rows (for tests and screenshots).
///
/// # Errors
///
/// When the GPU fails to copy or map the texture.
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<(Vec<u8>, u32, u32), String> {
    let (w, h) = (texture.width(), texture.height());
    let row = (4 * w).div_ceil(256) * 256;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("milkdrop_readback"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| e.to_string())?;
    rx.recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        out.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    Ok((out, w, h))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn blur_sizes_round_like_butterchurn() {
        assert_eq!(blur_size((640, 480), 0.5), (320, 240));
        assert_eq!(blur_size((640, 480), 0.25), (160, 120));
        assert_eq!(blur_size((100, 50), 0.0625), (16, 16));
    }

    #[test]
    fn the_inputs_fill_the_uniform_block() {
        let mut i = ShaderInputs::default();
        i.c[2] = [5.0, 6.0, 7.0, 8.0];
        i.rot[23][3] = [1.0, 2.0, 3.0];
        let b = inputs_bytes(&i);
        assert_eq!(b.len(), hlsl::INPUTS_SIZE);
        let f = |k: usize| f32::from_le_bytes(b[k * 4..k * 4 + 4].try_into().unwrap());
        assert_eq!(f(8), 5.0);
        // The last matrix's last row, padded to four.
        let last = b.len() / 4 - 4;
        assert_eq!(
            [f(last), f(last + 1), f(last + 2), f(last + 3)],
            [1.0, 2.0, 3.0, 0.0]
        );
    }

    /// Run by hand with `NEOAMP_MILK_DIR` set to a folder of presets: draws every
    /// `NEOAMP_MILK_EVERY`th preset (default 20th) for a few frames, which panics on any
    /// error wgpu finds.
    #[test]
    #[ignore = "needs a GPU and NEOAMP_MILK_DIR"]
    fn many_presets_draw() {
        use std::time::{Duration, Instant};
        let Some(dir) = std::env::var_os("NEOAMP_MILK_DIR") else {
            return;
        };
        let every: usize = std::env::var("NEOAMP_MILK_EVERY")
            .ok()
            .and_then(|f| f.parse().ok())
            .unwrap_or(20);
        let mut files = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(dir)];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("milk"))
                {
                    files.push(p);
                }
            }
        }
        files.sort();
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..wgpu::DeviceDescriptor::default()
        }))
        .unwrap();
        let size = (320, 240);
        let mut gpu = Gpu::new(&device, &queue, size);
        let mut engine = Engine::new(3);
        let t0 = Instant::now();
        let mut k = 0u64;
        let stereo: Vec<f32> = (0..2940)
            .map(|i| ((i / 2) as f32 * 0.05).sin() * 0.5)
            .collect();
        for (n, f) in files.iter().step_by(every.max(1)).enumerate() {
            eprintln!("{n}: {}", f.display());
            let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).into_owned();
            // Every other one blends in, to draw two presets at once too.
            engine.load(Preset::parse(&text), if n % 2 == 0 { 0.0 } else { 0.5 });
            for _ in 0..8 {
                k += 1;
                engine.push_audio(&stereo);
                engine.render(t0 + Duration::from_millis(33 * k), size);
                gpu.draw(&engine);
            }
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }

    /// Run by hand: renders the preset `NEOAMP_MILK_FILE` for a few seconds to the PNG
    /// `NEOAMP_MILK_PNG`, with a test tone for sound.
    #[test]
    #[ignore = "needs a GPU, NEOAMP_MILK_FILE and NEOAMP_MILK_PNG"]
    fn render_a_preset_to_png() {
        use std::time::{Duration, Instant};
        let (Some(file), Some(png)) = (
            std::env::var_os("NEOAMP_MILK_FILE"),
            std::env::var_os("NEOAMP_MILK_PNG"),
        ) else {
            return;
        };
        let frames: u32 = std::env::var("NEOAMP_MILK_FRAMES")
            .ok()
            .and_then(|f| f.parse().ok())
            .unwrap_or(150);
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..wgpu::DeviceDescriptor::default()
        }))
        .unwrap();
        let text = String::from_utf8_lossy(&std::fs::read(file).unwrap()).into_owned();
        let mut engine = Engine::new(7);
        engine.set_sample_rate(44_100);
        engine.load(Preset::parse(&text), 0.0);
        let size = (640, 480);
        let mut gpu = Gpu::new(&device, &queue, size);
        let t0 = Instant::now();
        let mut signal = vec![0.0f32; 1470];
        let mut stereo = vec![0.0f32; 2940];
        for k in 0..frames {
            crate::vis::demo_signal(f64::from(k) / 30.0, 44_100, &mut signal);
            for (lr, s) in stereo.chunks_exact_mut(2).zip(&signal) {
                lr[0] = *s;
                lr[1] = *s * 0.8;
            }
            engine.push_audio(&stereo);
            engine.render(t0 + Duration::from_millis(33 * u64::from(k)), size);
            gpu.draw(&engine);
        }
        let (data, w, h) = read_texture(&device, &queue, gpu.output_texture()).unwrap();
        image::save_buffer(png, &data, w, h, image::ColorType::Rgba8).unwrap();
    }
}
