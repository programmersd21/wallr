use image::GenericImageView;
use wgpu::util::DeviceExt;

pub struct Renderer {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub bind_group_layout_tex: wgpu::BindGroupLayout,
    pub bind_group_layout_uni: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
    pub uniform_bind_group: wgpu::BindGroup,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    /// Cached pipeline for a specific surface format. Created lazily.
    pipeline: std::sync::Mutex<Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>>,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub time: f32,
    pub progress: f32,
    pub effect_type: u32,
    pub padding: u32,
    pub resolution: [f32; 2],
    pub image_resolution: [f32; 2],
    pub old_image_resolution: [f32; 2],
    pub param_a: f32,
    pub param_b: f32,
    pub param_c: f32,
    pub param_d: f32,
    pub origin: [f32; 2],
    pub direction: [f32; 2],
    pub easing: u32,
    pub padding2: u32,
}

impl Uniforms {
    pub fn from_effect(effect: &crate::animation::EffectUniforms) -> Self {
        Self {
            time: effect.progress,
            progress: effect.progress,
            effect_type: effect.effect_type,
            padding: 0,
            resolution: [1920.0, 1080.0],
            image_resolution: [1920.0, 1080.0],
            old_image_resolution: [1920.0, 1080.0],
            param_a: effect.param_a,
            param_b: effect.param_b,
            param_c: effect.param_c,
            param_d: effect.param_d,
            origin: effect.origin,
            direction: effect.direction,
            easing: effect.easing,
            padding2: 0,
        }
    }
}

impl Renderer {
    pub async fn new() -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("Failed to find suitable adapter"))?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Effects Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                crate::shader::EFFECTS_SHADER,
            )),
        });

        let bind_group_layout_tex =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
                label: Some("texture_bind_group_layout"),
            });

        let bind_group_layout_uni =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
                label: Some("uniform_bind_group_layout"),
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[
                &bind_group_layout_tex,
                &bind_group_layout_tex,
                &bind_group_layout_uni,
            ],
            push_constant_ranges: &[],
        });

        let uniforms = Uniforms {
            time: 0.0,
            progress: 0.0,
            effect_type: 0,
            padding: 0,
            resolution: [1920.0, 1080.0],
            image_resolution: [1920.0, 1080.0],
            old_image_resolution: [1920.0, 1080.0],
            param_a: 0.0,
            param_b: 0.0,
            param_c: 0.0,
            param_d: 0.0,
            origin: [0.5, 0.5],
            direction: [0.0, 0.0],
            easing: 3,
            padding2: 0,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout_uni,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
            label: Some("uniform_bind_group"),
        });

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            pipeline_layout,
            shader,
            bind_group_layout_tex,
            bind_group_layout_uni,
            uniform_buffer,
            uniform_bind_group,
            pipeline: std::sync::Mutex::new(None),
        })
    }

    fn get_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let mut cache = self.pipeline.lock().unwrap();
        if let Some((cached_fmt, ref pipeline)) = *cache
            && cached_fmt == format
        {
            return pipeline.clone();
        }

        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Render Pipeline"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None, // Don't cull — fullscreen quad
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: 1,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: None,
                cache: None,
            });

        *cache = Some((format, pipeline.clone()));
        pipeline
    }

    pub fn create_texture(&self, width: u32, height: u32) -> (wgpu::Texture, wgpu::BindGroup) {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.bind_group_layout_tex,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
            label: None,
        });

        (texture, bind_group)
    }

    pub fn update_texture(&self, texture: &wgpu::Texture, rgba: &[u8], width: u32, height: u32) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn load_texture(
        &self,
        image: &image::DynamicImage,
    ) -> anyhow::Result<(wgpu::Texture, wgpu::BindGroup)> {
        let rgba = image.to_rgba8();
        let (width, height) = image.dimensions();
        let (texture, bind_group) = self.create_texture(width, height);
        self.update_texture(&texture, &rgba, width, height);
        Ok((texture, bind_group))
    }

    pub fn update_uniforms(&self, uniforms: Uniforms) {
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    pub fn render_frame(&self, request: FrameRequest) -> anyhow::Result<FrameStatus> {
        let FrameRequest {
            surface,
            format,
            bg_bind,
            new_bind,
            effect,
            width,
            height,
            img_width,
            img_height,
            old_img_width,
            old_img_height,
        } = request;
        let mut uniforms = Uniforms::from_effect(effect);
        uniforms.resolution = [width as f32, height as f32];
        uniforms.image_resolution = [img_width as f32, img_height as f32];
        uniforms.old_image_resolution = [old_img_width as f32, old_img_height as f32];
        self.update_uniforms(uniforms);

        let pipeline = self.get_pipeline(format);
        // `get_current_texture` blocks until the compositor presents (Fifo),
        // which paces rendering to the refresh rate. A stalled compositor
        // parks this call, which is safe because transition loops always run
        // on detached tasks that never block the daemon's IPC loop.
        let output = match surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Timeout) => return Ok(FrameStatus::TimedOut),
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "failed to acquire swapchain texture: {err:?}"
                ));
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Wallpaper Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&pipeline);
            render_pass.set_bind_group(0, bg_bind, &[]);
            render_pass.set_bind_group(1, new_bind, &[]);
            render_pass.set_bind_group(2, &self.uniform_bind_group, &[]);
            // The vertex shader generates a fullscreen triangle from vertex_index 0..3
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(FrameStatus::Presented)
    }
}

/// Whether a frame was actually presented to the surface. A `TimedOut` frame
/// means the compositor is not requesting frames right now (e.g. the monitor
/// is off), and callers should stop rendering instead of spinning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStatus {
    Presented,
    TimedOut,
}

/// Everything needed to present one transition frame to a surface.
pub struct FrameRequest<'a> {
    pub surface: &'a wgpu::Surface<'a>,
    pub format: wgpu::TextureFormat,
    /// Outgoing wallpaper frame, normally the daemon's previous wallpaper.
    pub bg_bind: &'a wgpu::BindGroup,
    /// New wallpaper.
    pub new_bind: &'a wgpu::BindGroup,
    /// Effect uniforms for this frame (progress, params, origin, easing...).
    pub effect: &'a crate::animation::EffectUniforms,
    pub width: u32,
    pub height: u32,
    pub img_width: u32,
    pub img_height: u32,
    pub old_img_width: u32,
    pub old_img_height: u32,
}
