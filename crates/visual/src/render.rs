//! Rendu hors écran d'un effet WGSL et relecture des pixels.

use std::time::Duration;

use crate::{BANDS, EFFECTS, Inputs, PRELUDE, VisualError};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
/// Taille du bloc de paramètres : 8 flottants puis les bandes.
const PARAMS_SIZE: usize = (8 + BANDS) * 4;

struct Target {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    /// Octets par ligne dans le tampon de relecture (aligné sur 256).
    padded_row: u32,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Vec<wgpu::RenderPipeline>,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    target: Option<Target>,
}

impl Renderer {
    /// Ouvre la carte graphique et compile tous les effets.
    pub fn new() -> Result<Self, VisualError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| VisualError::NoAdapter(e.to_string()))?;
        let info = adapter.get_info();
        tracing::info!(gpu = %info.name, backend = ?info.backend, "visualiseur");
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("ondes-visual"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits())
                    .using_alignment(adapter.limits()),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }),
        )
        .map_err(|e| VisualError::Device(e.to_string()))?;

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: PARAMS_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("params"),
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
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("params"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effets"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        // Une erreur de compilation WGSL est signalée par la portée d'erreur.
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = EFFECTS
            .iter()
            .map(|(name, source)| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(format!("{PRELUDE}\n{source}").into()),
                });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(name),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
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
            .collect();
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(VisualError::Render(error.to_string()));
        }

        Ok(Self {
            device,
            queue,
            pipelines,
            params,
            bind_group,
            target: None,
        })
    }

    fn target(&mut self, width: u32, height: u32) -> &Target {
        let current = self
            .target
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height);
        if !current {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cible"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("relecture"),
                size: u64::from(padded_row) * u64::from(height),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.target = Some(Target {
                width,
                height,
                texture,
                view,
                readback,
                padded_row,
            });
        }
        self.target.as_ref().expect("cible créée")
    }

    /// Rend l'effet `effect` et renvoie les pixels (BGRA, lignes contiguës).
    pub fn render(
        &mut self,
        effect: usize,
        width: u32,
        height: u32,
        inputs: &Inputs,
    ) -> Result<Vec<u8>, VisualError> {
        let mut params = Vec::with_capacity(PARAMS_SIZE);
        for v in [
            width as f32,
            height as f32,
            inputs.time,
            inputs.level,
            inputs.beat,
            inputs.bass,
            inputs.mid,
            inputs.treble,
        ]
        .iter()
        .chain(&inputs.bands)
        {
            params.extend_from_slice(&v.to_le_bytes());
        }
        self.queue.write_buffer(&self.params, 0, &params);

        let pipeline = &self.pipelines[effect % self.pipelines.len()];
        let bind_group = self.bind_group.clone();
        let pipeline = pipeline.clone();
        let device = self.device.clone();
        let queue = self.queue.clone();
        let target = self.target(width, height);

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effet"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = queue.submit(Some(encoder.finish()));

        let (tx, rx) = std::sync::mpsc::channel();
        target
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(5)),
            })
            .map_err(|e| VisualError::Render(e.to_string()))?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|e| VisualError::Render(e.to_string()))?
            .map_err(|e| VisualError::Render(e.to_string()))?;

        let row = (width * 4) as usize;
        let mut pixels = Vec::with_capacity(row * height as usize);
        {
            let mapped = target
                .readback
                .slice(..)
                .get_mapped_range()
                .map_err(|e| VisualError::Render(e.to_string()))?;
            for line in mapped.chunks_exact(target.padded_row as usize) {
                pixels.extend_from_slice(&line[..row]);
            }
        }
        target.readback.unmap();
        Ok(pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile et rend chaque effet ; ignoré sans carte graphique (CI).
    #[test]
    fn renders_every_effect() {
        let mut renderer = match Renderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("test ignoré : {e}");
                return;
            }
        };
        let mut inputs = Inputs {
            time: 3.,
            level: 0.7,
            beat: 0.8,
            bass: 0.9,
            mid: 0.5,
            treble: 0.3,
            ..Default::default()
        };
        for (i, b) in inputs.bands.iter_mut().enumerate() {
            *b = 1. - i as f32 / BANDS as f32;
        }
        for (i, (name, _)) in EFFECTS.iter().enumerate() {
            let pixels = renderer.render(i, 160, 90, &inputs).unwrap();
            assert_eq!(pixels.len(), 160 * 90 * 4);
            let lit = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60)
                .count();
            assert!(
                lit > 160 * 90 / 50,
                "{name} : image presque noire ({lit} pixels)"
            );
            assert!(
                pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
                "{name}"
            );
        }
    }
}
