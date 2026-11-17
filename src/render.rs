use crate::sim::Simulation;
use bytemuck::{Pod, Zeroable};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use std::ptr::NonNull;
use wayland_client::{Connection, Proxy, protocol::wl_surface};

const PARTICLE_SHADER: &str = r#"
struct Screen {
    size: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> screen: Screen;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) opacity: f32,
    @location(3) shape: f32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) shape: f32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(
        input.position.x / screen.size.x * 2.0 - 1.0,
        1.0 - input.position.y / screen.size.y * 2.0,
        0.0,
        1.0
    );
    output.uv = input.uv;
    output.opacity = input.opacity;
    output.shape = input.shape;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let distance = length(input.uv);
    let edge = max(fwidth(distance), 0.015);
    let coverage = 1.0 - smoothstep(1.0 - edge, 1.0 + edge, distance);
    let softness = select(0.45, 0.65, input.shape > 0.5);
    let core = 1.0 - smoothstep(0.0, 1.0, distance);
    let alpha = coverage * (softness + (1.0 - softness) * core) * input.opacity;
    return vec4<f32>(0.94 * alpha, 0.97 * alpha, alpha, alpha);
}
"#;

const BALL_SHADER: &str = r#"
struct Screen { size: vec2<f32> }
@group(0) @binding(0) var<uniform> screen: Screen;
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) opacity: f32,
    @location(3) shape: f32,
}
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) radius: f32,
}
@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position.x / screen.size.x * 2.0 - 1.0,
        1.0 - input.position.y / screen.size.y * 2.0, 0.0, 1.0);
    output.uv = input.uv;
    output.radius = input.shape;
    return output;
}
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}
@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let d = length(input.uv);
    let aa = max(fwidth(d), 0.001);
    let coverage = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, d);
    let normal = vec3<f32>(input.uv.x, input.uv.y, sqrt(max(1.0 - d * d, 0.0)));
    let light = normalize(vec3<f32>(-0.42, -0.55, 0.72));
    let diffuse = max(dot(normal, light), 0.0);
    let grain = (hash(floor((input.uv + 1.0) * max(input.radius, 4.0))) - 0.5) * 0.055;
    let brightness = clamp(0.78 + 0.20 * diffuse + grain, 0.0, 1.0);
    let color = vec3<f32>(brightness * 0.96, brightness * 0.98, brightness);
    let alpha = coverage * 0.98;
    return vec4<f32>(color * alpha, alpha);
}
"#;

const SNOW_SHADER: &str = r#"
struct Screen {
    size: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> screen: Screen;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(
        input.position.x / screen.size.x * 2.0 - 1.0,
        1.0 - input.position.y / screen.size.y * 2.0,
        0.0,
        1.0
    );
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color.rgb * input.color.a, input.color.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleVertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
    shape: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SnowVertex {
    position: [f32; 2],
    color: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShovelDirection {
    Left,
    Right,
}

impl ShovelDirection {
    pub fn after_motion(self, previous_x: f32, current_x: f32) -> Self {
        if current_x < previous_x {
            Self::Left
        } else if current_x > previous_x {
            Self::Right
        } else {
            self
        }
    }

    fn sign(self) -> f32 {
        match self {
            Self::Left => -1.0,
            Self::Right => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Shovel {
    pub position: (f32, f32),
    pub direction: ShovelDirection,
    pub scale: f32,
}

pub struct GpuRenderer {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    screen_buffer: wgpu::Buffer,
    screen_bind_group: wgpu::BindGroup,
    particle_pipeline: wgpu::RenderPipeline,
    ball_pipeline: wgpu::RenderPipeline,
    snow_pipeline: wgpu::RenderPipeline,
    particle_buffer: wgpu::Buffer,
    particle_capacity: usize,
    particle_vertices: Vec<ParticleVertex>,
    ball_buffer: wgpu::Buffer,
    ball_capacity: usize,
    ball_vertices: Vec<ParticleVertex>,
    snow_buffer: wgpu::Buffer,
    snow_capacity: usize,
    snow_vertices: Vec<SnowVertex>,
}

impl GpuRenderer {
    pub fn new(
        connection: &Connection,
        wl_surface: &wl_surface::WlSurface,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let display = NonNull::new(connection.backend().display_ptr() as *mut _)
            .ok_or("Wayland connection has a null display handle")?;
        let surface_handle = NonNull::new(wl_surface.id().as_ptr() as *mut _)
            .ok_or("Wayland surface has a null handle")?;
        let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display));
        let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(surface_handle));
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle,
                raw_window_handle,
            })
        }
        .map_err(|error| format!("cannot create Wayland GPU surface: {error}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|error| {
            format!("cannot find a GPU compatible with the Wayland output: {error}")
        })?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("christmas-snow-device"),
            ..Default::default()
        }))
        .map_err(|error| format!("cannot initialize the GPU device: {error}"))?;

        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(capabilities.formats[0]);
        let alpha_mode = capabilities
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| {
                matches!(
                    mode,
                    wgpu::CompositeAlphaMode::PreMultiplied
                        | wgpu::CompositeAlphaMode::PostMultiplied
                )
            })
            .unwrap_or(wgpu::CompositeAlphaMode::Auto);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![format],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("christmas-snow-screen-size"),
            size: 8,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &screen_buffer,
            0,
            bytemuck::cast_slice(&[width.max(1) as f32, height.max(1) as f32]),
        );
        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("christmas-snow-screen-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("christmas-snow-screen-bind-group"),
            layout: &screen_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_buffer.as_entire_binding(),
            }],
        });
        let particle_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("christmas-snow-particle-layout"),
                bind_group_layouts: &[&screen_layout],
                push_constant_ranges: &[],
            });
        let particle_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("christmas-snow-particle-shader"),
            source: wgpu::ShaderSource::Wgsl(PARTICLE_SHADER.into()),
        });
        let particle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("christmas-snow-particle-pipeline"),
            layout: Some(&particle_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &particle_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ParticleVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32,
                        3 => Float32
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &particle_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let ball_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("christmas-snow-ball-shader"),
            source: wgpu::ShaderSource::Wgsl(BALL_SHADER.into()),
        });
        let ball_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("christmas-snow-ball-pipeline"),
            layout: Some(&particle_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &ball_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ParticleVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &ball_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let snow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("christmas-snow-surface-shader"),
            source: wgpu::ShaderSource::Wgsl(SNOW_SHADER.into()),
        });
        let snow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("christmas-snow-surface-layout"),
            bind_group_layouts: &[&screen_layout],
            push_constant_ranges: &[],
        });
        let snow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("christmas-snow-surface-pipeline"),
            layout: Some(&snow_layout),
            vertex: wgpu::VertexState {
                module: &snow_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SnowVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &snow_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let particle_buffer = create_buffer(&device, "christmas-snow-particles", 4096);
        let particle_capacity = 4096 / std::mem::size_of::<ParticleVertex>();
        let ball_buffer = create_buffer(&device, "christmas-snow-balls", 4096);
        let ball_capacity = 4096 / std::mem::size_of::<ParticleVertex>();
        let snow_buffer = create_buffer(&device, "christmas-snow-accumulation", 4096);
        let snow_capacity = 4096 / std::mem::size_of::<SnowVertex>();
        Ok(Self {
            _instance: instance,
            surface,
            adapter,
            device,
            queue,
            surface_config,
            screen_buffer,
            screen_bind_group,
            particle_pipeline,
            ball_pipeline,
            snow_pipeline,
            particle_buffer,
            particle_capacity,
            particle_vertices: Vec::new(),
            ball_buffer,
            ball_capacity,
            ball_vertices: Vec::new(),
            snow_buffer,
            snow_capacity,
            snow_vertices: Vec::new(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.surface_config.width == width && self.surface_config.height == height {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.queue.write_buffer(
            &self.screen_buffer,
            0,
            bytemuck::cast_slice(&[width as f32, height as f32]),
        );
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub fn clear(&mut self) -> Result<(), String> {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error @ wgpu::SurfaceError::OutOfMemory) => {
                return Err(format!("GPU surface exhausted memory: {error}"));
            }
            Err(error) => return Err(format!("cannot acquire GPU frame: {error}")),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("christmas-snow-clear"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("christmas-snow-clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }

    pub fn render(
        &mut self,
        simulation: &Simulation,
        shovel: Option<Shovel>,
    ) -> Result<(), String> {
        fill_particle_vertices(&mut self.particle_vertices, simulation);
        ensure_capacity(
            &self.device,
            &mut self.particle_buffer,
            &mut self.particle_capacity,
            self.particle_vertices.len(),
            std::mem::size_of::<ParticleVertex>(),
            "christmas-snow-particles",
        );
        if !self.particle_vertices.is_empty() {
            self.queue.write_buffer(
                &self.particle_buffer,
                0,
                bytemuck::cast_slice(&self.particle_vertices),
            );
        }

        fill_ball_vertices(&mut self.ball_vertices, simulation);
        ensure_capacity(
            &self.device,
            &mut self.ball_buffer,
            &mut self.ball_capacity,
            self.ball_vertices.len(),
            std::mem::size_of::<ParticleVertex>(),
            "christmas-snow-balls",
        );
        if !self.ball_vertices.is_empty() {
            self.queue.write_buffer(
                &self.ball_buffer,
                0,
                bytemuck::cast_slice(&self.ball_vertices),
            );
        }
        let background_count = fill_surface_vertices(&mut self.snow_vertices, simulation);
        let decorations_end = self.snow_vertices.len();
        if let Some(shovel) = shovel {
            Self::append_shovel(&mut self.snow_vertices, shovel);
        }
        ensure_capacity(
            &self.device,
            &mut self.snow_buffer,
            &mut self.snow_capacity,
            self.snow_vertices.len(),
            std::mem::size_of::<SnowVertex>(),
            "christmas-snow-accumulation",
        );
        if !self.snow_vertices.is_empty() {
            self.queue.write_buffer(
                &self.snow_buffer,
                0,
                bytemuck::cast_slice(&self.snow_vertices),
            );
        }

        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error @ wgpu::SurfaceError::OutOfMemory) => {
                return Err(format!("GPU surface exhausted memory: {error}"));
            }
            Err(error) => return Err(format!("cannot acquire GPU frame: {error}")),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("christmas-snow-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("christmas-snow-transparent-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.snow_pipeline);
            pass.set_bind_group(0, &self.screen_bind_group, &[]);
            pass.set_vertex_buffer(0, self.snow_buffer.slice(..));
            pass.draw(0..background_count as u32, 0..1);
            pass.set_pipeline(&self.ball_pipeline);
            pass.set_vertex_buffer(0, self.ball_buffer.slice(..));
            pass.draw(0..self.ball_vertices.len() as u32, 0..1);
            pass.set_pipeline(&self.snow_pipeline);
            pass.set_vertex_buffer(0, self.snow_buffer.slice(..));
            pass.draw(background_count as u32..decorations_end as u32, 0..1);
            pass.set_pipeline(&self.particle_pipeline);
            pass.set_bind_group(0, &self.screen_bind_group, &[]);
            pass.set_vertex_buffer(0, self.particle_buffer.slice(..));
            pass.draw(0..self.particle_vertices.len() as u32, 0..1);
            pass.set_pipeline(&self.snow_pipeline);
            pass.set_vertex_buffer(0, self.snow_buffer.slice(..));
            pass.draw(
                decorations_end as u32..self.snow_vertices.len() as u32,
                0..1,
            );
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }

    fn append_shovel(vertices: &mut Vec<SnowVertex>, shovel: Shovel) {
        let point = |x: f32, y: f32| {
            [
                shovel.position.0 + x * shovel.direction.sign() * shovel.scale,
                shovel.position.1 + y * shovel.scale,
            ]
        };
        let dark = [0.09, 0.13, 0.18, 1.0];
        let shaft = [0.68, 0.40, 0.19, 1.0];
        let blue = [0.12, 0.43, 0.70, 1.0];
        let highlight = [0.42, 0.76, 0.95, 1.0];

        line(
            vertices,
            point(-6.0, -9.0),
            point(-28.0, -43.0),
            3.8 * shovel.scale,
            dark,
        );
        line(
            vertices,
            point(-6.0, -9.0),
            point(-28.0, -43.0),
            2.4 * shovel.scale,
            shaft,
        );
        line(
            vertices,
            point(-24.0, -41.0),
            point(-31.0, -46.0),
            3.2 * shovel.scale,
            dark,
        );
        line(
            vertices,
            point(-24.0, -41.0),
            point(-31.0, -46.0),
            1.7 * shovel.scale,
            highlight,
        );

        let a = point(-13.0, -17.0);
        let b = point(9.0, -17.0);
        let c = point(15.0, -4.0);
        let d = point(-9.0, 0.0);
        triangle(vertices, a, b, d, dark);
        triangle(vertices, b, c, d, dark);
        let a = point(-10.0, -15.0);
        let b = point(7.0, -15.0);
        let c = point(12.0, -5.0);
        let d = point(-7.0, -2.0);
        triangle(vertices, a, b, d, blue);
        triangle(vertices, b, c, d, blue);
        line(
            vertices,
            point(-7.0, -3.0),
            point(12.0, -6.0),
            1.3 * shovel.scale,
            highlight,
        );
    }

    pub fn adapter_name(&self) -> String {
        self.adapter.get_info().name
    }
}

fn fill_particle_vertices(vertices: &mut Vec<ParticleVertex>, simulation: &Simulation) {
    vertices.clear();
    vertices.reserve(simulation.flakes.len().saturating_mul(6));
    for flake in &simulation.flakes {
        let radius = flake.radius * simulation.particle_size();
        let cosine = flake.phase.cos();
        let sine = flake.phase.sin();
        let corners = [
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0],
        ];
        for corner in corners {
            let rotated_x = corner[0] * cosine - corner[1] * sine;
            let rotated_y = corner[0] * sine + corner[1] * cosine;
            vertices.push(ParticleVertex {
                position: [flake.x + rotated_x * radius, flake.y + rotated_y * radius],
                uv: corner,
                opacity: flake.opacity,
                shape: f32::from(flake.shape),
            });
        }
    }
}

fn fill_ball_vertices(vertices: &mut Vec<ParticleVertex>, simulation: &Simulation) {
    vertices.clear();
    vertices.reserve(simulation.snowballs.len().saturating_mul(6));
    for ball in &simulation.snowballs {
        let radius = ball.radius();
        for uv in [
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0],
        ] {
            vertices.push(ParticleVertex {
                position: [ball.x + uv[0] * radius, ball.y + uv[1] * radius],
                uv,
                opacity: 1.0,
                shape: radius,
            });
        }
    }
}

fn fill_surface_vertices(vertices: &mut Vec<SnowVertex>, simulation: &Simulation) -> usize {
    vertices.clear();
    vertices.reserve(simulation.snow_depth.len().saturating_mul(3));
    append_surface(vertices, &simulation.snow_depth, 0.0, |_| {
        simulation.height()
    });
    for shelf in &simulation.window_shelves {
        append_surface(vertices, &shelf.snow_depth, shelf.x, |x| shelf.baseline(x));
    }
    for man in &simulation.snowmen {
        let body = &simulation.snowballs[man.balls[man.balls.len() - 2]];
        let r = body.radius();
        let stick = [0.31, 0.21, 0.13, 1.0];
        for side in [-1.0, 1.0] {
            line(
                vertices,
                [body.x + side * r * 0.65, body.y],
                [body.x + side * r * 1.75, body.y - r * 0.65],
                (r * 0.12).max(1.0),
                stick,
            );
            line(
                vertices,
                [body.x + side * r * 1.45, body.y - r * 0.47],
                [body.x + side * r * 1.42, body.y - r * 0.95],
                (r * 0.07).max(0.7),
                stick,
            );
        }
    }
    let background_count = vertices.len();
    for man in &simulation.snowmen {
        let head = &simulation.snowballs[*man.balls.last().expect("snowman has balls")];
        let body = &simulation.snowballs[man.balls[man.balls.len() - 2]];
        let r = head.radius();
        let dark = [0.08, 0.10, 0.13, 1.0];
        let hat = [0.12, 0.14, 0.18, 1.0];
        let red = [0.68, 0.08, 0.12, 1.0];
        let green = [0.07, 0.38, 0.20, 1.0];
        let band = if man.red_hat_band { red } else { green };
        let scarf = if man.red_scarf { red } else { green };
        for side in [-1.0, 1.0] {
            circle(
                vertices,
                head.x + side * r * 0.32,
                head.y - r * 0.15,
                r * 0.105,
                dark,
            );
        }
        for offset in -2..=2 {
            let x = offset as f32 * r * 0.15;
            circle(
                vertices,
                head.x + x,
                head.y + r * (0.44 - 0.15 * (offset as f32 / 2.0).powi(2)),
                r * 0.055,
                dark,
            );
        }
        triangle(
            vertices,
            [head.x - r * 0.06, head.y + r * 0.02],
            [head.x + r * 0.68, head.y + r * 0.14],
            [head.x - r * 0.06, head.y + r * 0.20],
            [0.95, 0.39, 0.08, 1.0],
        );
        let brim_y = head.y - r * 0.78;
        rect(
            vertices,
            head.x - r * 0.65,
            brim_y - r * 0.8,
            r * 1.3,
            r * 0.8,
            hat,
        );
        rect(
            vertices,
            head.x - r * 0.92,
            brim_y - r * 0.08,
            r * 1.84,
            r * 0.19,
            hat,
        );
        rect(
            vertices,
            head.x - r * 0.65,
            brim_y - r * 0.31,
            r * 1.3,
            r * 0.18,
            band,
        );
        let neck = (head.y + r + body.y - body.radius()) * 0.5;
        rect(
            vertices,
            head.x - r * 0.67,
            neck - r * 0.13,
            r * 1.34,
            r * 0.28,
            scarf,
        );
        rect(vertices, head.x + r * 0.25, neck, r * 0.25, r * 0.65, scarf);
        for offset in [-0.28, 0.05, 0.38] {
            circle(
                vertices,
                body.x,
                body.y + body.radius() * offset,
                (r * 0.085).max(1.0),
                dark,
            );
        }
    }
    background_count
}

#[cfg(test)]
fn ball_vertices(simulation: &Simulation) -> Vec<ParticleVertex> {
    let mut vertices = Vec::new();
    fill_ball_vertices(&mut vertices, simulation);
    vertices
}

#[cfg(test)]
fn surface_vertices(simulation: &Simulation) -> (Vec<SnowVertex>, usize) {
    let mut vertices = Vec::new();
    let background_count = fill_surface_vertices(&mut vertices, simulation);
    (vertices, background_count)
}

fn triangle(
    vertices: &mut Vec<SnowVertex>,
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
    color: [f32; 4],
) {
    vertices.extend([
        SnowVertex { position: a, color },
        SnowVertex { position: b, color },
        SnowVertex { position: c, color },
    ]);
}

fn rect(vertices: &mut Vec<SnowVertex>, x: f32, y: f32, width: f32, height: f32, color: [f32; 4]) {
    triangle(vertices, [x, y], [x + width, y], [x, y + height], color);
    triangle(
        vertices,
        [x + width, y],
        [x + width, y + height],
        [x, y + height],
        color,
    );
}

fn line(vertices: &mut Vec<SnowVertex>, a: [f32; 2], b: [f32; 2], width: f32, color: [f32; 4]) {
    let length = (b[0] - a[0]).hypot(b[1] - a[1]).max(0.001);
    let normal = [
        (b[1] - a[1]) * width / length,
        -(b[0] - a[0]) * width / length,
    ];
    let p = |v: [f32; 2], sign: f32| [v[0] + normal[0] * sign, v[1] + normal[1] * sign];
    triangle(vertices, p(a, 1.0), p(a, -1.0), p(b, 1.0), color);
    triangle(vertices, p(a, -1.0), p(b, -1.0), p(b, 1.0), color);
}

fn circle(vertices: &mut Vec<SnowVertex>, x: f32, y: f32, radius: f32, color: [f32; 4]) {
    for i in 0..12 {
        let point = |angle: f32| [x + radius * angle.cos(), y + radius * angle.sin()];
        triangle(
            vertices,
            [x, y],
            point(i as f32 * std::f32::consts::TAU / 12.0),
            point((i + 1) as f32 * std::f32::consts::TAU / 12.0),
            color,
        );
    }
}

fn append_surface(
    vertices: &mut Vec<SnowVertex>,
    depths: &[f32],
    x_offset: f32,
    baseline: impl Fn(f32) -> f32,
) {
    if depths.len() < 2 {
        return;
    }
    for x in 0..depths.len() - 1 {
        let next = x + 1;
        if depths[x] <= 0.0 && depths[next] <= 0.0 {
            continue;
        }
        let left_baseline = baseline(x_offset + x as f32);
        let right_baseline = baseline(x_offset + next as f32);
        let left_y = left_baseline - depths[x];
        let right_y = right_baseline - depths[next];
        let left_bottom = vertex(x_offset + x as f32, left_baseline, [0.76, 0.84, 0.93, 0.97]);
        let right_bottom = vertex(
            x_offset + next as f32,
            right_baseline,
            [0.76, 0.84, 0.93, 0.97],
        );
        let left_top = vertex(x_offset + x as f32, left_y, [0.99, 1.0, 1.0, 0.98]);
        let right_top = vertex(x_offset + next as f32, right_y, [0.99, 1.0, 1.0, 0.98]);
        vertices.extend_from_slice(&[
            left_top,
            right_top,
            left_bottom,
            right_top,
            right_bottom,
            left_bottom,
        ]);
    }
}

fn vertex(x: f32, y: f32, color: [f32; 4]) -> SnowVertex {
    SnowVertex {
        position: [x, y],
        color,
    }
}

fn create_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn ensure_capacity(
    device: &wgpu::Device,
    buffer: &mut wgpu::Buffer,
    capacity: &mut usize,
    length: usize,
    element_size: usize,
    label: &str,
) {
    if length <= *capacity {
        return;
    }
    *capacity = length.next_power_of_two();
    *buffer = create_buffer(device, label, (*capacity * element_size) as u64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Flake, Snowball, Snowman};

    #[test]
    fn shovel_faces_the_latest_horizontal_motion() {
        let direction = ShovelDirection::Right;
        assert_eq!(direction.after_motion(10.0, 9.0), ShovelDirection::Left);
        assert_eq!(
            ShovelDirection::Left.after_motion(9.0, 9.0),
            ShovelDirection::Left
        );
        assert_eq!(
            ShovelDirection::Left.after_motion(9.0, 10.0),
            ShovelDirection::Right
        );
    }

    #[test]
    fn shovel_mesh_mirrors_around_the_pointer_and_scales() {
        let mut right = Vec::new();
        let mut left = Vec::new();
        let mut doubled = Vec::new();
        let shovel = Shovel {
            position: (100.0, 150.0),
            direction: ShovelDirection::Right,
            scale: 1.0,
        };
        GpuRenderer::append_shovel(&mut right, shovel);
        GpuRenderer::append_shovel(
            &mut left,
            Shovel {
                direction: ShovelDirection::Left,
                ..shovel
            },
        );
        GpuRenderer::append_shovel(
            &mut doubled,
            Shovel {
                scale: 2.0,
                ..shovel
            },
        );
        assert!(!right.is_empty());
        assert_eq!(right.len(), left.len());
        for vertex in &right {
            assert!(left.iter().any(|other| {
                (other.position[0] - (200.0 - vertex.position[0])).abs() < 0.001
                    && (other.position[1] - vertex.position[1]).abs() < 0.001
                    && other.color == vertex.color
            }));
        }
        for (one, two) in right.iter().zip(&doubled) {
            assert!((two.position[0] - (100.0 + (one.position[0] - 100.0) * 2.0)).abs() < 0.001);
            assert!((two.position[1] - (150.0 + (one.position[1] - 150.0) * 2.0)).abs() < 0.001);
        }
        assert!(
            right
                .iter()
                .any(|vertex| vertex.color == [0.12, 0.43, 0.70, 1.0])
        );
        assert!(right.iter().any(|vertex| vertex.position[1] < 110.0));
    }

    #[test]
    fn particle_vertex_buffer_reuses_capacity_and_preserves_geometry() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        sim.flakes.push(Flake {
            x: 30.0,
            y: 40.0,
            vx: 0.0,
            vy: 0.0,
            radius: 2.0,
            opacity: 0.8,
            phase: 0.0,
            shape: 1,
            snow_mass: None,
        });
        let mut vertices = Vec::new();

        fill_particle_vertices(&mut vertices, &sim);
        let capacity = vertices.capacity();
        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[0].position, [28.0, 38.0]);
        assert_eq!(vertices[0].uv, [-1.0, -1.0]);
        assert_eq!(vertices[0].opacity, 0.8);

        fill_particle_vertices(&mut vertices, &sim);

        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices.capacity(), capacity);
        assert_eq!(vertices[0].position, [28.0, 38.0]);
    }

    #[test]
    fn snowball_quads_follow_simulation_geometry_with_no_extra_click_area() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        sim.snowballs.push(Snowball {
            x: 80.0,
            y: 90.0,
            vx: 0.0,
            vy: 0.0,
            mass: std::f32::consts::PI * 400.0,
        });
        let vertices = ball_vertices(&sim);
        assert_eq!(vertices.len(), 6);
        assert!(vertices.iter().all(|v| v.shape == 20.0));
        assert!(vertices.iter().any(|v| v.position == [60.0, 70.0]));
        assert!(vertices.iter().any(|v| v.position == [100.0, 110.0]));
    }

    #[test]
    fn accumulation_shading_stays_within_the_simulated_depth() {
        let mut vertices = Vec::new();
        append_surface(&mut vertices, &[0.0, 8.0, 8.0, 0.0], 10.0, |_| 100.0);
        assert!(
            vertices
                .iter()
                .all(|v| v.position[1] >= 92.0 && v.position[1] <= 100.0)
        );
        assert!(vertices.iter().any(|v| v.color == [0.99, 1.0, 1.0, 0.98]));
        assert!(vertices.iter().any(|v| v.color == [0.76, 0.84, 0.93, 0.97]));
        assert!(
            surface_vertices(&Simulation::new(100, 100, 0.0, 0.0, 20.0))
                .0
                .is_empty()
        );
    }

    #[test]
    fn rounded_shelf_mesh_uses_curved_window_baseline() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        sim.set_window_rounding(10.0);
        sim.set_windows(
            &[crate::sim::WindowShelfInput {
                id: "window".to_owned(),
                x: 20.0,
                y: 80.0,
                width: 100.0,
                height: 50.0,
            }],
            true,
        );
        sim.window_shelves[0].snow_depth.fill(3.0);
        let (vertices, _) = surface_vertices(&sim);
        assert!(vertices.iter().any(|v| v.position == [20.0, 90.0]));
        assert!(vertices.iter().any(|v| v.position == [30.0, 80.0]));
    }

    #[test]
    fn snowman_mesh_contains_decorations_and_stable_independent_colors() {
        let mut sim = Simulation::new(200, 200, 0.0, 0.0, 20.0);
        sim.snowballs = [140.0, 100.0]
            .into_iter()
            .map(|y| Snowball {
                x: 100.0,
                y,
                vx: 0.0,
                vy: 0.0,
                mass: std::f32::consts::PI * 400.0,
            })
            .collect();
        sim.snowmen.push(Snowman {
            balls: vec![0, 1],
            red_hat_band: true,
            red_scarf: false,
        });
        let (first, background_count) = surface_vertices(&sim);
        assert!(background_count > 0);
        assert!(
            first[..background_count]
                .iter()
                .any(|v| v.color == [0.31, 0.21, 0.13, 1.0])
        );
        assert!(
            first[background_count..]
                .iter()
                .any(|v| v.color == [0.95, 0.39, 0.08, 1.0])
        );
        let (second, _) = surface_vertices(&sim);
        assert_eq!(first.len(), second.len());
        assert!(
            first
                .iter()
                .zip(&second)
                .all(|(a, b)| a.position == b.position && a.color == b.color)
        );
        for color in [
            [0.68, 0.08, 0.12, 1.0],
            [0.07, 0.38, 0.20, 1.0],
            [0.95, 0.39, 0.08, 1.0],
            [0.31, 0.21, 0.13, 1.0],
            [0.08, 0.10, 0.13, 1.0],
        ] {
            assert!(first.iter().any(|vertex| vertex.color == color));
        }
        assert!(first.iter().any(|vertex| vertex.position[1] < 100.0 - 20.0));
        sim.snowmen[0].red_hat_band = false;
        sim.snowmen[0].red_scarf = true;
        let (swapped, _) = surface_vertices(&sim);
        assert_ne!(
            first.iter().map(|vertex| vertex.color).collect::<Vec<_>>(),
            swapped
                .iter()
                .map(|vertex| vertex.color)
                .collect::<Vec<_>>()
        );
    }
}
