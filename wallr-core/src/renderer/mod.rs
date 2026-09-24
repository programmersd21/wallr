use image::GenericImageView;
use wgpu::util::DeviceExt;

use crate::video::{GpuSelection, VideoFrameData, YuvColorInfo, YuvMatrix, YuvRange};

const MAX_TEXTURE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_STATIC_DECODE_BYTES: u64 = 512 * 1024 * 1024;

pub struct Renderer {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub bind_group_layout_tex: wgpu::BindGroupLayout,
    pub bind_group_layout_uni: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    /// Lazily created pipelines keyed by surface format. Key space is the
    /// finite `wgpu::TextureFormat` enum; in practice 1-2 entries (one per
    /// distinct surface format across outputs). Never created in the frame
    /// hot path more than once per format.
    pipeline:
        std::sync::Mutex<std::collections::HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>>,
    nv12_bind_group_layout: wgpu::BindGroupLayout,
    nv12_pipeline: wgpu::RenderPipeline,
    /// Shared samplers. Previously a new sampler was created per texture
    /// upload; samplers are immutable and cheap to share, so one per use
    /// lives as long as the renderer.
    static_sampler: wgpu::Sampler,
    video_plane_sampler: wgpu::Sampler,
    video_output_sampler: wgpu::Sampler,
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

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// True when an incoming frame can reuse the allocated Y/UV/output
    /// textures, bind groups, and conversion buffer instead of recreating
    /// GPU resources. Only a real resolution change requires recreation.
    pub fn matches_dimensions(&self, width: u32, height: u32) -> bool {
        self.width == width && self.height == height
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
    pub async fn new(preference: &GpuSelection) -> anyhow::Result<Self> {
        // A Wayland wallpaper daemon on Linux only needs the native Linux GPU
        // backends. `Backends::all()` also probes browser/mobile/Apple
        // backends that cannot produce a Wayland surface here, increasing
        // startup work and sometimes loading unnecessary driver state.
        let (instance, adapter) = 'select: {
            // Try Vulkan first without initializing the GL backend: probing
            // GL loads its whole driver stack (~15 ms measured) for a
            // fallback most systems never need.
            #[cfg(target_os = "linux")]
            {
                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::VULKAN,
                    ..Default::default()
                });
                if let Ok(adapter) = crate::video::select_adapter(&instance, preference).await {
                    break 'select (instance, adapter);
                }
                tracing::info!("No Vulkan adapter found, falling back to GL");
            }
            // Keep Vulkan as the primary path and OpenGL as a compatibility
            // fallback (plus everything elsewhere).
            #[cfg(target_os = "linux")]
            let backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
            #[cfg(not(target_os = "linux"))]
            let backends = wgpu::Backends::all();
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends,
                ..Default::default()
            });
            let adapter = crate::video::select_adapter(&instance, preference)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            (instance, adapter)
        };

        let adapter_limits = adapter.limits();
        let required_limits = wgpu::Limits {
            max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
            ..wgpu::Limits::default()
        };

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    // Wallr is a persistent background process, not a game:
                    // prefer smaller allocations and lower residency over
                    // speculative throughput. Frame pacing remains governed
                    // by FIFO presentation and the compositor.
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
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

        let static_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wallr static sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let video_plane_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Video Plane Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let video_output_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Video Output Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
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
            pipeline: std::sync::Mutex::new(std::collections::HashMap::new()),
            nv12_bind_group_layout,
            nv12_pipeline,
            static_sampler,
            video_plane_sampler,
            video_output_sampler,
        })
    }

    pub fn validate_static_decode(
        width: u32,
        height: u32,
        decoded_bytes: u64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            width > 0 && height > 0,
            "decoded image dimensions must be non-zero, got {width}x{height}"
        );
        anyhow::ensure!(
            decoded_bytes <= MAX_STATIC_DECODE_BYTES,
            "decoded image for {width}x{height} requires approximately {:.1} MiB, exceeding the {:.0} MiB safety limit",
            decoded_bytes as f64 / (1024.0 * 1024.0),
            MAX_STATIC_DECODE_BYTES as f64 / (1024.0 * 1024.0)
        );
        Ok(())
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
        // Fast path: reuse an already-created pipeline for this format.
        // Only the first frame per format pays for pipeline creation, and
        // creation never happens more than once per distinct format.
        if let Ok(cache) = self.pipeline.lock().as_deref()
            && let Some(pipeline) = cache.get(&format)
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
                    cull_mode: None, // Don't cull - fullscreen quad
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

        // Insert under lock; a concurrent first-frame for the same format
        // may have inserted first, in which case keep the existing entry.
        if let Ok(mut cache) = self.pipeline.lock() {
            cache.entry(format).or_insert_with(|| pipeline.clone());
            if let Some(existing) = cache.get(&format) {
                return existing.clone();
            }
        }
        pipeline
    }

    pub fn create_texture(
        &self,
        width: u32,
        height: u32,
    ) -> anyhow::Result<(wgpu::Texture, wgpu::BindGroup)> {
        validate_texture_dimensions(width, height, self.device.limits().max_texture_dimension_2d)?;
        validate_texture_memory(width, height, 4)?;
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
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.bind_group_layout_tex,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.static_sampler),
                },
            ],
            label: None,
        });

        Ok((texture, bind_group))
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

    pub fn create_video_texture(&self, width: u32, height: u32) -> anyhow::Result<VideoTexture> {
        self.validate_video_texture(width, height)?;
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
                    resource: wgpu::BindingResource::Sampler(&self.video_plane_sampler),
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
                    resource: wgpu::BindingResource::Sampler(&self.video_output_sampler),
                },
            ],
        });

        Ok(VideoTexture {
            output,
            output_view,
            effects_bind_group,
            luma,
            chroma,
            conversion_buffer,
            conversion_bind_group,
            width,
            height,
        })
    }

    pub fn validate_video_texture(&self, width: u32, height: u32) -> anyhow::Result<()> {
        validate_texture_dimensions(width, height, self.device.limits().max_texture_dimension_2d)?;
        // RGBA output plus NV12 luma/chroma planes use about 5.5 bytes per
        // pixel. Round up so all allocations stay within a conservative cap.
        validate_texture_memory(width, height, 6)
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
        output_width: u32,
        output_height: u32,
        scaling_mode: u32,
    ) -> anyhow::Result<(wgpu::Texture, wgpu::BindGroup, u32, u32)> {
        // Share the output-aware sizing policy and the SIMD resize path with
        // the wl_shm fast path so large static uploads pay one efficient
        // downscale instead of uploading a huge texture for the GPU to scale.
        let (rgba, width, height) =
            self.prepare_image_rgba(image, output_width, output_height, scaling_mode)?;
        let (texture, bind_group) = self.create_texture(width, height)?;
        self.update_texture(&texture, &rgba, width, height);
        Ok((texture, bind_group, width, height))
    }

    /// Prepare pixels for a compositor-owned shared-memory buffer. This is
    /// the static-image counterpart to `load_texture`: it applies the same
    /// output-aware sizing policy without allocating a GPU texture.
    pub fn prepare_image_rgba(
        &self,
        image: &image::DynamicImage,
        output_width: u32,
        output_height: u32,
        scaling_mode: u32,
    ) -> anyhow::Result<(image::RgbaImage, u32, u32)> {
        prepare_image_rgba(
            image,
            output_width,
            output_height,
            scaling_mode,
            self.device.limits().max_texture_dimension_2d,
        )
    }
}

/// Prepare compositor-owned static pixels without requiring a live GPU
/// renderer. Keeping this operation independent is the first boundary for
/// lazy GPU initialization: `wl_shm` outputs can decode and size static
/// wallpapers before any wgpu device is created.
pub fn prepare_image_rgba(
    image: &image::DynamicImage,
    output_width: u32,
    output_height: u32,
    scaling_mode: u32,
    max_texture_dimension: u32,
) -> anyhow::Result<(image::RgbaImage, u32, u32)> {
    let (width, height) = prepared_image_dimensions(
        image.width(),
        image.height(),
        output_width,
        output_height,
        scaling_mode,
        max_texture_dimension,
    );
    let rgba = if (width, height) == image.dimensions() {
        image.to_rgba8()
    } else {
        // Use the SIMD-capable resizer for the expensive downscale path. It
        // preserves the existing Lanczos3 quality while avoiding the slower
        // scalar imageops implementation for 4K-to-output conversions.
        let source = image.to_rgba8();
        let source_view = fast_image_resize::images::ImageRef::new(
            source.width(),
            source.height(),
            source.as_raw(),
            fast_image_resize::PixelType::U8x4,
        )?;
        let mut destination = fast_image_resize::images::Image::new(
            width,
            height,
            fast_image_resize::PixelType::U8x4,
        );
        let mut resizer = fast_image_resize::Resizer::new();
        resizer.resize(&source_view, &mut destination, None)?;
        image::RgbaImage::from_raw(width, height, destination.into_vec())
            .ok_or_else(|| anyhow::anyhow!("resizer returned an invalid RGBA buffer"))?
    };
    Ok((rgba, width, height))
}

impl Renderer {
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
            Err(wgpu::SurfaceError::Outdated) => return Ok(FrameStatus::Outdated),
            Err(wgpu::SurfaceError::Lost) => return Ok(FrameStatus::Lost),
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

/// Whether a frame was presented or the surface needs recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStatus {
    Presented,
    TimedOut,
    Outdated,
    Lost,
}

fn validate_texture_dimensions(width: u32, height: u32, limit: u32) -> anyhow::Result<()> {
    anyhow::ensure!(
        width > 0 && height > 0,
        "texture dimensions must be non-zero, got {width}x{height}"
    );
    anyhow::ensure!(
        width <= limit && height <= limit,
        "texture dimensions {width}x{height} exceed the GPU limit of {limit}"
    );
    Ok(())
}

fn validate_texture_memory(width: u32, height: u32, bytes_per_pixel: u64) -> anyhow::Result<()> {
    validate_image_memory(
        width,
        height,
        bytes_per_pixel,
        MAX_TEXTURE_BYTES,
        "texture allocation",
    )
}

fn validate_image_memory(
    width: u32,
    height: u32,
    bytes_per_pixel: u64,
    limit: u64,
    label: &str,
) -> anyhow::Result<()> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| anyhow::anyhow!("{label} size overflow for {width}x{height}"))?;
    anyhow::ensure!(
        bytes <= limit,
        "{label} for {width}x{height} requires approximately {:.1} MiB, exceeding the {:.0} MiB safety limit",
        bytes as f64 / (1024.0 * 1024.0),
        limit as f64 / (1024.0 * 1024.0)
    );
    Ok(())
}

fn prepared_image_dimensions(
    image_width: u32,
    image_height: u32,
    output_width: u32,
    output_height: u32,
    scaling_mode: u32,
    texture_limit: u32,
) -> (u32, u32) {
    if image_width == 0 || image_height == 0 {
        return (image_width, image_height);
    }

    let limit_scale = (texture_limit as f64 / image_width as f64)
        .min(texture_limit as f64 / image_height as f64)
        .min(1.0);
    let output_scale = match scaling_mode {
        // Fill retains enough pixels to cover the output; fit retains enough
        // to fit inside it. The shader still owns the final crop/letterbox.
        0 if output_width > 0 && output_height > 0 => (output_width as f64 / image_width as f64)
            .max(output_height as f64 / image_height as f64)
            .min(1.0),
        1 if output_width > 0 && output_height > 0 => (output_width as f64 / image_width as f64)
            .min(output_height as f64 / image_height as f64)
            .min(1.0),
        2 if output_width > 0 && output_height > 0 => {
            return (
                image_width.min(output_width).min(texture_limit).max(1),
                image_height.min(output_height).min(texture_limit).max(1),
            );
        }
        // Center and tile have pixel-sensitive semantics. Reject an impossible
        // native texture later rather than silently changing its apparent size.
        _ => return (image_width, image_height),
    };
    let scale = limit_scale.min(output_scale);

    (
        ((image_width as f64 * scale).round() as u32).max(1),
        ((image_height as f64 * scale).round() as u32).max(1),
    )
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

    /// Renders a real fade transition offscreen (no compositor needed) and
    /// checks the blended pixels. Skips gracefully where no GPU exists.
    /// This exercises the full path the unit tests cannot: pipeline lookup,
    /// bind groups, uniform upload, and the WGSL blend for the renumbered
    /// effect arms.
    #[test]
    fn transition_renders_mid_fade_blend_offscreen() {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return,
        };
        let renderer = match runtime.block_on(Renderer::new(&GpuSelection::Auto)) {
            Ok(renderer) => renderer,
            Err(_) => {
                eprintln!("SKIP: no GPU adapter for offscreen transition test");
                return;
            }
        };
        const SIZE: u32 = 64;
        let red = [255u8, 0, 0, 255].repeat((SIZE * SIZE) as usize);
        let blue = [0u8, 0, 255, 255].repeat((SIZE * SIZE) as usize);
        let (tex_old, bind_old) = renderer.create_texture(SIZE, SIZE).expect("old texture");
        let (tex_new, bind_new) = renderer.create_texture(SIZE, SIZE).expect("new texture");
        renderer.update_texture(&tex_old, &red, SIZE, SIZE);
        renderer.update_texture(&tex_new, &blue, SIZE, SIZE);

        let per_output = renderer.create_per_output_uniforms();
        let render_at = |effect: &crate::animation::Effect, progress: f32| -> Vec<u8> {
            let effect = crate::animation::compute_effect_uniforms(effect, progress);
            let mut uniforms = Uniforms::from_effect(&effect);
            uniforms.resolution = [SIZE as f32, SIZE as f32];
            uniforms.image_resolution = [SIZE as f32, SIZE as f32];
            uniforms.old_image_resolution = [SIZE as f32, SIZE as f32];
            uniforms.scaling_mode = 0;
            renderer.update_uniforms(&per_output.buffer, uniforms);

            let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("offscreen transition target"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let pipeline = renderer.get_pipeline(wgpu::TextureFormat::Rgba8UnormSrgb);
            let mut encoder = renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("offscreen transition"),
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
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_old, &[]);
                pass.set_bind_group(1, &bind_new, &[]);
                pass.set_bind_group(2, &per_output.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("offscreen readback"),
                size: (SIZE * SIZE * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SIZE * 4),
                        rows_per_image: Some(SIZE),
                    },
                },
                wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
            );
            renderer.queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            renderer.device.poll(wgpu::Maintain::Wait);
            let pixels = {
                let mapped = slice.get_mapped_range();
                mapped.to_vec()
            };
            readback.unmap();
            pixels
        };
        let pixel_at = |frame: &[u8], x: u32, y: u32| -> [u8; 4] {
            let i = ((y * SIZE + x) * 4) as usize;
            [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
        };

        let fade = crate::animation::Effect::Fade(crate::animation::FadeParams::default());
        // Fade endpoints are exact (no blending artifacts at start/end).
        let start = render_at(&fade, 0.0);
        let start = pixel_at(&start, SIZE / 2, SIZE / 2);
        assert!(start[0] > 240 && start[2] < 15, "start pixel: {start:?}");
        let end = render_at(&fade, 1.0);
        let end = pixel_at(&end, SIZE / 2, SIZE / 2);
        assert!(end[2] > 240 && end[0] < 15, "end pixel: {end:?}");
        // Mid-fade blends in linear light (sRGB textures decode on sample
        // and re-encode on store). The default Bezier easing runs ahead at
        // midpoint (awww's fast-start character), so blue leads red here
        // while their sum still matches a uniform mix.
        let mid = render_at(&fade, 0.5);
        let mid = pixel_at(&mid, SIZE / 2, SIZE / 2);
        assert!(
            (160..=215).contains(&mid[0])
                && (160..=215).contains(&mid[2])
                && mid[2] > mid[0]
                && mid[1] < 20,
            "mid pixel: {mid:?}"
        );

        // A mid-wipe is sharp: well outside the feathered edge the pixels
        // are exactly old or new, with only a narrow blend band between.
        // (Default wipe enters from the LEFT, so the left edge is new/blue.)
        let wipe = crate::animation::Effect::Wipe(crate::animation::WipeParams::default());
        let frame = render_at(&wipe, 0.5);
        let left = pixel_at(&frame, 4, SIZE / 2);
        assert!(
            left[2] > 240 && left[0] < 15,
            "wipe revealed pixel: {left:?}"
        );
        let right = pixel_at(&frame, SIZE - 5, SIZE / 2);
        assert!(
            right[0] > 240 && right[2] < 15,
            "wipe unrevealed pixel: {right:?}"
        );

        // Every cardinal enters from its named edge and must be fully shown
        // at p=1.0 (regression: a reversed threshold left the image mostly
        // old, then snapped).
        let at = |direction: crate::animation::WipeDirection, progress, x, y| {
            let e = crate::animation::Effect::Wipe(crate::animation::WipeParams {
                direction,
                ..crate::animation::WipeParams::default()
            });
            let f = render_at(&e, progress);
            pixel_at(&f, x, y)
        };
        let top = at(crate::animation::WipeDirection::Up, 1.0, SIZE / 2, 2);
        assert!(top[2] > 240 && top[0] < 15, "up wipe end pixel: {top:?}");
        let bottom = at(
            crate::animation::WipeDirection::Down,
            1.0,
            SIZE / 2,
            SIZE - 2,
        );
        assert!(
            bottom[2] > 240 && bottom[0] < 15,
            "down wipe end pixel: {bottom:?}"
        );
        let mid_top = at(
            crate::animation::WipeDirection::Down,
            0.5,
            SIZE / 2,
            SIZE - 2,
        );
        assert!(
            mid_top[2] > 240 && mid_top[0] < 15,
            "down wipe reveals bottom first: {mid_top:?}"
        );
        let far_top = at(crate::animation::WipeDirection::Down, 0.5, SIZE / 2, 2);
        assert!(
            far_top[0] > 240 && far_top[2] < 15,
            "down wipe keeps top old: {far_top:?}"
        );

        // Sweep must be monotonic: a boundary column switches old->new once,
        // never back. This directly catches the old "new, old, new" symptom.
        // For a left-entering wipe, a column sweeps in left-to-right, so a
        // fixed middle column must be old early and new from halfway on.
        let mid_col = SIZE / 2;
        let sweep = [
            at(
                crate::animation::WipeDirection::Left,
                0.2,
                mid_col,
                SIZE / 2,
            ),
            at(
                crate::animation::WipeDirection::Left,
                0.4,
                mid_col,
                SIZE / 2,
            ),
            at(
                crate::animation::WipeDirection::Left,
                0.6,
                mid_col,
                SIZE / 2,
            ),
            at(
                crate::animation::WipeDirection::Left,
                0.8,
                mid_col,
                SIZE / 2,
            ),
        ];
        assert!(
            sweep[0][0] > 240 && sweep[0][2] < 15,
            "early old: {:?}",
            sweep[0]
        );
        assert!(
            sweep[1][0] > 240 && sweep[1][2] < 15,
            "early old: {:?}",
            sweep[1]
        );
        assert!(
            sweep[2][2] > 240 && sweep[2][0] < 15,
            "late new: {:?}",
            sweep[2]
        );
        assert!(
            sweep[3][2] > 240 && sweep[3][0] < 15,
            "late new: {:?}",
            sweep[3]
        );
        // Entry edges for the remaining cardinals.
        let right_entry = at(
            crate::animation::WipeDirection::Right,
            0.5,
            SIZE - 2,
            SIZE / 2,
        );
        assert!(
            right_entry[2] > 240 && right_entry[0] < 15,
            "right wipe reveals right first: {right_entry:?}"
        );
        let up_entry = at(crate::animation::WipeDirection::Up, 0.5, SIZE / 2, 2);
        assert!(
            up_entry[2] > 240 && up_entry[0] < 15,
            "up wipe reveals top first: {up_entry:?}"
        );

        // A mid-grow is sharp too: the center is exactly new while the
        // far corner is still exactly old.
        let grow = crate::animation::Effect::Grow(crate::animation::GrowParams::default());
        let frame = render_at(&grow, 0.5);
        let center = pixel_at(&frame, SIZE / 2, SIZE / 2);
        assert!(
            center[2] > 240 && center[0] < 15,
            "grow revealed pixel: {center:?}"
        );
        let corner = pixel_at(&frame, 2, 2);
        assert!(
            corner[0] > 240 && corner[2] < 15,
            "grow unrevealed pixel: {corner:?}"
        );
    }

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

    #[test]
    fn validates_texture_dimensions_before_wgpu() {
        assert!(validate_texture_dimensions(8192, 8192, 8192).is_ok());
        assert!(validate_texture_dimensions(0, 1080, 8192).is_err());
        assert!(validate_texture_dimensions(8193, 1080, 8192).is_err());
        assert!(validate_texture_dimensions(1920, 8193, 8192).is_err());
        assert!(validate_texture_memory(7_680, 4_320, 6).is_ok());
        assert!(validate_texture_memory(16_384, 16_384, 4).is_err());
        assert!(Renderer::validate_static_decode(11_322, 6_192, 210_304_512).is_ok());
        assert!(Renderer::validate_static_decode(32_768, 32_768, u64::MAX).is_err());
    }

    #[test]
    fn fill_reduces_large_images_to_cover_the_output() {
        assert_eq!(
            prepared_image_dimensions(11_322, 6_192, 3_840, 2_160, 0, 32_768),
            (3_950, 2_160)
        );
    }

    #[test]
    fn fit_preserves_aspect_ratio_without_upscaling() {
        assert_eq!(
            prepared_image_dimensions(11_322, 6_192, 3_840, 2_160, 1, 32_768),
            (3_840, 2_100)
        );
        assert_eq!(
            prepared_image_dimensions(1_920, 1_080, 3_840, 2_160, 1, 32_768),
            (1_920, 1_080)
        );
    }

    #[test]
    fn pixel_sensitive_modes_preserve_native_dimensions() {
        assert_eq!(
            prepared_image_dimensions(11_322, 6_192, 3_840, 2_160, 3, 8_192),
            (11_322, 6_192)
        );
        assert_eq!(
            prepared_image_dimensions(3_840, 2_160, 2_560, 1_440, 4, 8_192),
            (3_840, 2_160)
        );
    }

    #[test]
    fn stretch_does_not_upscale_small_images() {
        assert_eq!(
            prepared_image_dimensions(1, 1, 3_840, 2_160, 2, 32_768),
            (1, 1)
        );
        assert_eq!(
            prepared_image_dimensions(5_000, 1_000, 3_840, 2_160, 2, 32_768),
            (3_840, 1_000)
        );
    }
}
