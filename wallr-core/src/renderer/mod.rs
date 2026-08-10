use image::GenericImageView;
use wgpu::util::DeviceExt;

use crate::video::{VideoFrameData, YuvColorInfo, YuvMatrix, YuvRange};

pub struct Renderer {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub bind_group_layout_tex: wgpu::BindGroupLayout,
    pub bind_group_layout_uni: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    /// Cached pipeline for a specific surface format. Created lazily.
    pipeline: std::sync::Mutex<Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>>,
    nv12_bind_group_layout: wgpu::BindGroupLayout,
    nv12_pipeline: wgpu::RenderPipeline,
}

pub struct VideoTexture {
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    effects_bind_group: wgpu::BindGroup,
    luma: wgpu::Texture,
    chroma: wgpu::Texture,
    conversion_buffer: wgpu::Buffer,
    conversion_bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

impl VideoTexture {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.output
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.effects_bind_group
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct YuvConversion {
    range: [f32; 4],
    red: [f32; 4],
    green: [f32; 4],
    blue: [f32; 4],
}

impl YuvConversion {
    fn new(color: YuvColorInfo) -> Self {
        let range = match color.range {
            YuvRange::Limited => [16.0 / 255.0, 255.0 / 219.0, 128.0 / 255.0, 255.0 / 224.0],
            YuvRange::Full => [0.0, 1.0, 128.0 / 255.0, 1.0],
        };
        let (red_cr, green_cb, green_cr, blue_cb) = match color.matrix {
            YuvMatrix::Bt601 => (1.402, -0.344_136, -0.714_136, 1.772),
            YuvMatrix::Bt709 => (1.5748, -0.187_324, -0.468_124, 1.8556),
            YuvMatrix::Bt2020 => (1.4746, -0.164_553, -0.571_353, 1.8814),
        };
        Self {
            range,
            red: [1.0, 0.0, red_cr, 0.0],
            green: [1.0, green_cb, green_cr, 0.0],
            blue: [1.0, blue_cb, 0.0, 0.0],
        }
    }
}

/// Per-output uniform buffer and bind group. Each output gets its own so
/// concurrent renders never race on shared GPU state.
pub struct PerOutputUniforms {
    pub buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
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
    pub scaling_mode: u32,
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
            scaling_mode: 0,
        }
    }
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
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
            scaling_mode: 0,
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

        let nv12_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("NV12 Conversion Bind Group Layout"),
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
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
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

        let nv12_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("NV12 to RGB Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                crate::shader::NV12_TO_RGB_SHADER,
            )),
        });
        let nv12_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("NV12 Conversion Pipeline Layout"),
            bind_group_layouts: &[&nv12_bind_group_layout],
            push_constant_ranges: &[],
        });
        let nv12_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("NV12 Conversion Pipeline"),
            layout: Some(&nv12_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &nv12_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &nv12_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
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
            pipeline: std::sync::Mutex::new(None),
            nv12_bind_group_layout,
            nv12_pipeline,
        })
    }

    /// Create a per-output uniform buffer and bind group so each output
    /// renders with its own GPU state, eliminating cross-output races.
    pub fn create_per_output_uniforms(&self) -> PerOutputUniforms {
        let uniforms = Uniforms::default();
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Per-Output Uniform Buffer"),
                contents: bytemuck::cast_slice(&[uniforms]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.bind_group_layout_uni,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
            label: Some("per_output_uniform_bind_group"),
        });
        PerOutputUniforms { buffer, bind_group }
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

    pub fn create_video_texture(&self, width: u32, height: u32) -> VideoTexture {
        let plane_texture = |label, size, format| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let luma = plane_texture(
            "Video NV12 Luma",
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            wgpu::TextureFormat::R8Unorm,
        );
        let chroma = plane_texture(
            "Video NV12 Chroma",
            wgpu::Extent3d {
                width: width.div_ceil(2),
                height: height.div_ceil(2),
                depth_or_array_layers: 1,
            },
            wgpu::TextureFormat::Rg8Unorm,
        );
        let output = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Video RGB Output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Video Plane Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let output_sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Video Output Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let conversion = YuvConversion::new(YuvColorInfo {
            matrix: YuvMatrix::Bt709,
            range: YuvRange::Limited,
        });
        let conversion_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Video YUV Conversion Uniform"),
                contents: bytemuck::bytes_of(&conversion),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let luma_view = luma.create_view(&wgpu::TextureViewDescriptor::default());
        let chroma_view = chroma.create_view(&wgpu::TextureViewDescriptor::default());
        let output_view = output.create_view(&wgpu::TextureViewDescriptor::default());
        let conversion_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Video NV12 Conversion Bind Group"),
            layout: &self.nv12_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&luma_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&chroma_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: conversion_buffer.as_entire_binding(),
                },
            ],
        });
        let effects_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Video Effects Bind Group"),
            layout: &self.bind_group_layout_tex,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&output_sampler),
                },
            ],
        });

        VideoTexture {
            output,
            output_view,
            effects_bind_group,
            luma,
            chroma,
            conversion_buffer,
            conversion_bind_group,
            width,
            height,
        }
    }

    pub fn update_video_texture(
        &self,
        texture: &VideoTexture,
        frame: &VideoFrameData,
    ) -> anyhow::Result<()> {
        match frame {
            VideoFrameData::Rgba(rgba) => {
                anyhow::ensure!(
                    rgba.len() == texture.width as usize * texture.height as usize * 4,
                    "invalid RGBA video frame size"
                );
                self.update_texture(&texture.output, rgba, texture.width, texture.height);
            }
            VideoFrameData::Nv12 {
                y_plane,
                uv_plane,
                color,
            } => {
                let chroma_width = texture.width.div_ceil(2);
                let chroma_height = texture.height.div_ceil(2);
                anyhow::ensure!(
                    y_plane.len() == texture.width as usize * texture.height as usize,
                    "invalid NV12 luma plane size"
                );
                anyhow::ensure!(
                    uv_plane.len() == (chroma_width * chroma_height * 2) as usize,
                    "invalid NV12 chroma plane size"
                );
                self.queue.write_texture(
                    texture.luma.as_image_copy(),
                    y_plane,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(texture.width),
                        rows_per_image: Some(texture.height),
                    },
                    wgpu::Extent3d {
                        width: texture.width,
                        height: texture.height,
                        depth_or_array_layers: 1,
                    },
                );
                self.queue.write_texture(
                    texture.chroma.as_image_copy(),
                    uv_plane,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(chroma_width * 2),
                        rows_per_image: Some(chroma_height),
                    },
                    wgpu::Extent3d {
                        width: chroma_width,
                        height: chroma_height,
                        depth_or_array_layers: 1,
                    },
                );
                self.queue.write_buffer(
                    &texture.conversion_buffer,
                    0,
                    bytemuck::bytes_of(&YuvConversion::new(*color)),
                );

                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("NV12 Conversion Encoder"),
                        });
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("NV12 Conversion Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &texture.output_view,
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
                    pass.set_pipeline(&self.nv12_pipeline);
                    pass.set_bind_group(0, &texture.conversion_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                self.queue.submit([encoder.finish()]);
            }
        }
        Ok(())
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

    pub fn update_uniforms(&self, buffer: &wgpu::Buffer, uniforms: Uniforms) {
        self.queue
            .write_buffer(buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    pub fn render_frame(
        &self,
        request: FrameRequest,
        per_output: &PerOutputUniforms,
    ) -> anyhow::Result<FrameStatus> {
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
            scaling_mode,
        } = request;
        let mut uniforms = Uniforms::from_effect(effect);
        uniforms.resolution = [width as f32, height as f32];
        uniforms.image_resolution = [img_width as f32, img_height as f32];
        uniforms.old_image_resolution = [old_img_width as f32, old_img_height as f32];
        uniforms.scaling_mode = scaling_mode;
        self.update_uniforms(&per_output.buffer, uniforms);

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
            render_pass.set_bind_group(2, &per_output.bind_group, &[]);
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
    /// Scaling mode: 0=Fill, 1=Fit, 2=Stretch, 3=Center, 4=Tile.
    pub scaling_mode: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_yuv_conversion_coefficients_and_range() {
        let limited_709 = YuvConversion::new(YuvColorInfo {
            matrix: YuvMatrix::Bt709,
            range: YuvRange::Limited,
        });
        assert_eq!(limited_709.red, [1.0, 0.0, 1.5748, 0.0]);
        assert_eq!(limited_709.green, [1.0, -0.187_324, -0.468_124, 0.0]);
        assert_eq!(limited_709.range[0], 16.0 / 255.0);
        assert_eq!(limited_709.range[1], 255.0 / 219.0);

        let full_2020 = YuvConversion::new(YuvColorInfo {
            matrix: YuvMatrix::Bt2020,
            range: YuvRange::Full,
        });
        assert_eq!(full_2020.red, [1.0, 0.0, 1.4746, 0.0]);
        assert_eq!(full_2020.blue, [1.0, 1.8814, 0.0, 0.0]);
        assert_eq!(full_2020.range, [0.0, 1.0, 128.0 / 255.0, 1.0]);
    }
}
