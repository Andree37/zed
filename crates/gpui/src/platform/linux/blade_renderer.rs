// Doing `if let` gives you nice scoping with passes/encoders
#![allow(irrefutable_let_patterns)]

use super::{BladeBelt, BladeBeltDescriptor};
use crate::{
    AtlasTextureKind, AtlasTile, BladeAtlas, Bounds, ContentMask, Hsla, MonochromeSprite, Path,
    PathId, PathVertex, PolychromeSprite, PrimitiveBatch, Quad, ScaledPixels, Scene, Shadow,
    Underline, PATH_TEXTURE_FORMAT,
};
use bytemuck::{Pod, Zeroable};
use collections::HashMap;

use blade_graphics as gpu;
use std::{mem, sync::Arc};

const SURFACE_FRAME_COUNT: u32 = 3;
const MAX_FRAME_TIME_MS: u32 = 1000;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    pad: [u32; 2],
}

#[derive(blade_macros::ShaderData)]
struct ShaderQuadsData {
    globals: GlobalParams,
    b_quads: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderShadowsData {
    globals: GlobalParams,
    b_shadows: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderPathRasterizationData {
    globals: GlobalParams,
    b_path_vertices: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderPathsData {
    globals: GlobalParams,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_path_sprites: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderUnderlinesData {
    globals: GlobalParams,
    b_underlines: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderMonoSpritesData {
    globals: GlobalParams,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_mono_sprites: gpu::BufferPiece,
}

#[derive(blade_macros::ShaderData)]
struct ShaderPolySpritesData {
    globals: GlobalParams,
    t_sprite: gpu::TextureView,
    s_sprite: gpu::Sampler,
    b_poly_sprites: gpu::BufferPiece,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
struct PathSprite {
    bounds: Bounds<ScaledPixels>,
    color: Hsla,
    tile: AtlasTile,
}

struct BladePipelines {
    quads: gpu::RenderPipeline,
    shadows: gpu::RenderPipeline,
    path_rasterization: gpu::RenderPipeline,
    paths: gpu::RenderPipeline,
    underlines: gpu::RenderPipeline,
    mono_sprites: gpu::RenderPipeline,
    poly_sprites: gpu::RenderPipeline,
}

impl BladePipelines {
    fn new(gpu: &gpu::Context, surface_format: gpu::TextureFormat) -> Self {
        use gpu::ShaderData as _;

        let shader = gpu.create_shader(gpu::ShaderDesc {
            source: include_str!("shaders.wgsl"),
        });
        shader.check_struct_size::<Quad>();
        shader.check_struct_size::<Shadow>();
        assert_eq!(
            mem::size_of::<PathVertex<ScaledPixels>>(),
            shader.get_struct_size("PathVertex") as usize,
        );
        shader.check_struct_size::<PathSprite>();
        shader.check_struct_size::<Underline>();
        shader.check_struct_size::<MonochromeSprite>();
        shader.check_struct_size::<PolychromeSprite>();

        Self {
            quads: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "quads",
                data_layouts: &[&ShaderQuadsData::layout()],
                vertex: shader.at("vs_quad"),
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: shader.at("fs_quad"),
                color_targets: &[gpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(gpu::BlendState::ALPHA_BLENDING),
                    write_mask: gpu::ColorWrites::default(),
                }],
            }),
            shadows: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "shadows",
                data_layouts: &[&ShaderShadowsData::layout()],
                vertex: shader.at("vs_shadow"),
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: shader.at("fs_shadow"),
                color_targets: &[gpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(gpu::BlendState::ALPHA_BLENDING),
                    write_mask: gpu::ColorWrites::default(),
                }],
            }),
            path_rasterization: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "path_rasterization",
                data_layouts: &[&ShaderPathRasterizationData::layout()],
                vertex: shader.at("vs_path_rasterization"),
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: shader.at("fs_path_rasterization"),
                color_targets: &[gpu::ColorTargetState {
                    format: PATH_TEXTURE_FORMAT,
                    blend: Some(gpu::BlendState::ALPHA_BLENDING),
                    write_mask: gpu::ColorWrites::default(),
                }],
            }),
            paths: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "paths",
                data_layouts: &[&ShaderPathsData::layout()],
                vertex: shader.at("vs_path"),
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: shader.at("fs_path"),
                color_targets: &[gpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(gpu::BlendState::ALPHA_BLENDING),
                    write_mask: gpu::ColorWrites::default(),
                }],
            }),
            underlines: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "underlines",
                data_layouts: &[&ShaderUnderlinesData::layout()],
                vertex: shader.at("vs_underline"),
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: shader.at("fs_underline"),
                color_targets: &[gpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(gpu::BlendState::ALPHA_BLENDING),
                    write_mask: gpu::ColorWrites::default(),
                }],
            }),
            mono_sprites: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "mono-sprites",
                data_layouts: &[&ShaderMonoSpritesData::layout()],
                vertex: shader.at("vs_mono_sprite"),
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: shader.at("fs_mono_sprite"),
                color_targets: &[gpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(gpu::BlendState::ALPHA_BLENDING),
                    write_mask: gpu::ColorWrites::default(),
                }],
            }),
            poly_sprites: gpu.create_render_pipeline(gpu::RenderPipelineDesc {
                name: "poly-sprites",
                data_layouts: &[&ShaderPolySpritesData::layout()],
                vertex: shader.at("vs_poly_sprite"),
                primitive: gpu::PrimitiveState {
                    topology: gpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                fragment: shader.at("fs_poly_sprite"),
                color_targets: &[gpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(gpu::BlendState::ALPHA_BLENDING),
                    write_mask: gpu::ColorWrites::default(),
                }],
            }),
        }
    }
}

pub struct BladeRenderer {
    gpu: Arc<gpu::Context>,
    command_encoder: gpu::CommandEncoder,
    last_sync_point: Option<gpu::SyncPoint>,
    pipelines: BladePipelines,
    instance_belt: BladeBelt,
    viewport_size: gpu::Extent,
    path_tiles: HashMap<PathId, AtlasTile>,
    atlas: Arc<BladeAtlas>,
    atlas_sampler: gpu::Sampler,
}

impl BladeRenderer {
    pub fn new(gpu: Arc<gpu::Context>, size: gpu::Extent) -> Self {
        let surface_format = gpu.resize(gpu::SurfaceConfig {
            size,
            usage: gpu::TextureUsage::TARGET,
            frame_count: SURFACE_FRAME_COUNT,
        });
        let command_encoder = gpu.create_command_encoder(gpu::CommandEncoderDesc {
            name: "main",
            buffer_count: 2,
        });
        let pipelines = BladePipelines::new(&gpu, surface_format);
        let instance_belt = BladeBelt::new(BladeBeltDescriptor {
            memory: gpu::Memory::Shared,
            min_chunk_size: 0x1000,
            alignment: 0x40, // Vulkan `minStorageBufferOffsetAlignment` on Intel Xe
        });
        let atlas = Arc::new(BladeAtlas::new(&gpu));
        let atlas_sampler = gpu.create_sampler(gpu::SamplerDesc {
            name: "atlas",
            mag_filter: gpu::FilterMode::Linear,
            min_filter: gpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            gpu,
            command_encoder,
            last_sync_point: None,
            pipelines,
            instance_belt,
            viewport_size: size,
            path_tiles: HashMap::default(),
            atlas,
            atlas_sampler,
        }
    }

    fn wait_for_gpu(&mut self) {
        if let Some(last_sp) = self.last_sync_point.take() {
            if !self.gpu.wait_for(&last_sp, MAX_FRAME_TIME_MS) {
                panic!("GPU hung");
            }
        }
    }

    pub fn destroy(&mut self) {
        self.wait_for_gpu();
        self.atlas.destroy();
        self.instance_belt.destroy(&self.gpu);
        self.gpu.destroy_command_encoder(&mut self.command_encoder);
    }

    pub fn resize(&mut self, size: gpu::Extent) {
        self.wait_for_gpu();
        self.gpu.resize(gpu::SurfaceConfig {
            size,
            usage: gpu::TextureUsage::TARGET,
            frame_count: SURFACE_FRAME_COUNT,
        });
        self.viewport_size = size;
    }

    pub fn viewport_size(&self) -> gpu::Extent {
        self.viewport_size
    }

    pub fn atlas(&self) -> &Arc<BladeAtlas> {
        &self.atlas
    }

    fn rasterize_paths(&mut self, paths: &[Path<ScaledPixels>]) {
        self.path_tiles.clear();
        let mut vertices_by_texture_id = HashMap::default();

        for path in paths {
            let clipped_bounds = path.bounds.intersect(&path.content_mask.bounds);
            let tile = self
                .atlas
                .allocate(clipped_bounds.size.map(Into::into), AtlasTextureKind::Path);
            vertices_by_texture_id
                .entry(tile.texture_id)
                .or_insert(Vec::new())
                .extend(path.vertices.iter().map(|vertex| PathVertex {
                    xy_position: vertex.xy_position - clipped_bounds.origin
                        + tile.bounds.origin.map(Into::into),
                    st_position: vertex.st_position,
                    content_mask: ContentMask {
                        bounds: tile.bounds.map(Into::into),
                    },
                }));
            self.path_tiles.insert(path.id, tile);
        }

        for (texture_id, vertices) in vertices_by_texture_id {
            let tex_info = self.atlas.get_texture_info(texture_id);
            let globals = GlobalParams {
                viewport_size: [tex_info.size.width as f32, tex_info.size.height as f32],
                pad: [0; 2],
            };

            let vertex_buf = self.instance_belt.alloc_data(&vertices, &self.gpu);
            let mut pass = self.command_encoder.render(gpu::RenderTargetSet {
                colors: &[gpu::RenderTarget {
                    view: tex_info.raw_view,
                    init_op: gpu::InitOp::Clear(gpu::TextureColor::OpaqueBlack),
                    finish_op: gpu::FinishOp::Store,
                }],
                depth_stencil: None,
            });

            let mut encoder = pass.with(&self.pipelines.path_rasterization);
            encoder.bind(
                0,
                &ShaderPathRasterizationData {
                    globals,
                    b_path_vertices: vertex_buf,
                },
            );
            encoder.draw(0, vertices.len() as u32, 0, 1);
        }
    }

    pub fn draw(&mut self, scene: &Scene) {
        let frame = self.gpu.acquire_frame();
        self.command_encoder.start();
        self.command_encoder.init_texture(frame.texture());

        self.atlas.before_frame(&mut self.command_encoder);
        self.rasterize_paths(scene.paths());

        let globals = GlobalParams {
            viewport_size: [
                self.viewport_size.width as f32,
                self.viewport_size.height as f32,
            ],
            pad: [0; 2],
        };

        if let mut pass = self.command_encoder.render(gpu::RenderTargetSet {
            colors: &[gpu::RenderTarget {
                view: frame.texture_view(),
                init_op: gpu::InitOp::Clear(gpu::TextureColor::TransparentBlack),
                finish_op: gpu::FinishOp::Store,
            }],
            depth_stencil: None,
        }) {
            for batch in scene.batches() {
                match batch {
                    PrimitiveBatch::Quads(quads) => {
                        let instance_buf = self.instance_belt.alloc_data(quads, &self.gpu);
                        let mut encoder = pass.with(&self.pipelines.quads);
                        encoder.bind(
                            0,
                            &ShaderQuadsData {
                                globals,
                                b_quads: instance_buf,
                            },
                        );
                        encoder.draw(0, 4, 0, quads.len() as u32);
                    }
                    PrimitiveBatch::Shadows(shadows) => {
                        let instance_buf = self.instance_belt.alloc_data(shadows, &self.gpu);
                        let mut encoder = pass.with(&self.pipelines.shadows);
                        encoder.bind(
                            0,
                            &ShaderShadowsData {
                                globals,
                                b_shadows: instance_buf,
                            },
                        );
                        encoder.draw(0, 4, 0, shadows.len() as u32);
                    }
                    PrimitiveBatch::Paths(paths) => {
                        let mut encoder = pass.with(&self.pipelines.paths);
                        //todo!(linux): group by texture ID
                        for path in paths {
                            let tile = &self.path_tiles[&path.id];
                            let tex_info = self.atlas.get_texture_info(tile.texture_id);
                            let origin = path.bounds.intersect(&path.content_mask.bounds).origin;
                            let sprites = [PathSprite {
                                bounds: Bounds {
                                    origin: origin.map(|p| p.floor()),
                                    size: tile.bounds.size.map(Into::into),
                                },
                                color: path.color,
                                tile: (*tile).clone(),
                            }];

                            let instance_buf = self.instance_belt.alloc_data(&sprites, &self.gpu);
                            encoder.bind(
                                0,
                                &ShaderPathsData {
                                    globals,
                                    t_sprite: tex_info.raw_view,
                                    s_sprite: self.atlas_sampler,
                                    b_path_sprites: instance_buf,
                                },
                            );
                            encoder.draw(0, 4, 0, sprites.len() as u32);
                        }
                    }
                    PrimitiveBatch::Underlines(underlines) => {
                        let instance_buf = self.instance_belt.alloc_data(underlines, &self.gpu);
                        let mut encoder = pass.with(&self.pipelines.underlines);
                        encoder.bind(
                            0,
                            &ShaderUnderlinesData {
                                globals,
                                b_underlines: instance_buf,
                            },
                        );
                        encoder.draw(0, 4, 0, underlines.len() as u32);
                    }
                    PrimitiveBatch::MonochromeSprites {
                        texture_id,
                        sprites,
                    } => {
                        let tex_info = self.atlas.get_texture_info(texture_id);
                        let instance_buf = self.instance_belt.alloc_data(&sprites, &self.gpu);
                        let mut encoder = pass.with(&self.pipelines.mono_sprites);
                        encoder.bind(
                            0,
                            &ShaderMonoSpritesData {
                                globals,
                                t_sprite: tex_info.raw_view,
                                s_sprite: self.atlas_sampler,
                                b_mono_sprites: instance_buf,
                            },
                        );
                        encoder.draw(0, 4, 0, sprites.len() as u32);
                    }
                    PrimitiveBatch::PolychromeSprites {
                        texture_id,
                        sprites,
                    } => {
                        let tex_info = self.atlas.get_texture_info(texture_id);
                        let instance_buf = self.instance_belt.alloc_data(&sprites, &self.gpu);
                        let mut encoder = pass.with(&self.pipelines.poly_sprites);
                        encoder.bind(
                            0,
                            &ShaderPolySpritesData {
                                globals,
                                t_sprite: tex_info.raw_view,
                                s_sprite: self.atlas_sampler,
                                b_poly_sprites: instance_buf,
                            },
                        );
                        encoder.draw(0, 4, 0, sprites.len() as u32);
                    }
                    PrimitiveBatch::Surfaces { .. } => {
                        unimplemented!()
                    }
                }
            }
        }

        self.command_encoder.present(frame);
        let sync_point = self.gpu.submit(&mut self.command_encoder);

        self.instance_belt.flush(&sync_point);
        self.atlas.after_frame(&sync_point);
        self.atlas.clear_textures(AtlasTextureKind::Path);

        self.wait_for_gpu();
        self.last_sync_point = Some(sync_point);
    }
}
