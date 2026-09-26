//! Bounded RGBA atlas for rasterized Markdown images.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use ui::core::paint::RasterImage;

pub const IMAGE_ATLAS_SIZE: u32 = 4096;
const CELL_SIZE: u32 = 64;
const CELL_COUNT: u32 = IMAGE_ATLAS_SIZE / CELL_SIZE;
const EDGE_GUTTER: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageAtlasError {
    TooLarge,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageSlot {
    /// Normalized UV of the image content, excluding its duplicated edge gutter.
    pub uv: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct CellRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct ImageEntry {
    cells: CellRect,
    slot: ImageSlot,
    image: Weak<RasterImage>,
    last_use: u64,
    pinned_frame: u64,
}

pub struct ImageAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    occupancy: Vec<Option<u64>>,
    entries: HashMap<u64, ImageEntry>,
    frame: u64,
    use_sequence: u64,
}

impl ImageAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RGBA image atlas"),
            size: wgpu::Extent3d {
                width: IMAGE_ATLAS_SIZE,
                height: IMAGE_ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            occupancy: vec![None; (CELL_COUNT * CELL_COUNT) as usize],
            entries: HashMap::new(),
            frame: 1,
            use_sequence: 0,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Call once before any DrawList is drained for a frame.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        if self.frame == 0 {
            self.frame = 1;
            for entry in self.entries.values_mut() {
                entry.pinned_frame = 0;
            }
        }
    }

    pub fn get_or_upload(
        &mut self,
        image: &Arc<RasterImage>,
        queue: &wgpu::Queue,
    ) -> Result<ImageSlot, ImageAtlasError> {
        self.use_sequence = self.use_sequence.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&image.id()) {
            entry.last_use = self.use_sequence;
            entry.pinned_frame = self.frame;
            return Ok(entry.slot);
        }

        let padded_width =
            image.width().checked_add(EDGE_GUTTER * 2).ok_or(ImageAtlasError::TooLarge)?;
        let padded_height =
            image.height().checked_add(EDGE_GUTTER * 2).ok_or(ImageAtlasError::TooLarge)?;
        if padded_width > IMAGE_ATLAS_SIZE || padded_height > IMAGE_ATLAS_SIZE {
            return Err(ImageAtlasError::TooLarge);
        }
        let needed_width = padded_width.div_ceil(CELL_SIZE);
        let needed_height = padded_height.div_ceil(CELL_SIZE);
        let cells = loop {
            if let Some(cells) = self.find_free_cells(needed_width, needed_height) {
                break cells;
            }
            let Some(eviction_id) = self.eviction_candidate() else {
                return Err(ImageAtlasError::Full);
            };
            self.remove(eviction_id);
        };

        self.upload_pixels(image, queue, cells);
        self.mark(cells, Some(image.id()));
        let left = (cells.x * CELL_SIZE + EDGE_GUTTER) as f32 / IMAGE_ATLAS_SIZE as f32;
        let top = (cells.y * CELL_SIZE + EDGE_GUTTER) as f32 / IMAGE_ATLAS_SIZE as f32;
        let right =
            (cells.x * CELL_SIZE + EDGE_GUTTER + image.width()) as f32 / IMAGE_ATLAS_SIZE as f32;
        let bottom =
            (cells.y * CELL_SIZE + EDGE_GUTTER + image.height()) as f32 / IMAGE_ATLAS_SIZE as f32;
        let slot = ImageSlot { uv: [left, top, right, bottom] };
        self.entries.insert(
            image.id(),
            ImageEntry {
                cells,
                slot,
                image: Arc::downgrade(image),
                last_use: self.use_sequence,
                pinned_frame: self.frame,
            },
        );
        Ok(slot)
    }

    fn find_free_cells(&self, width: u32, height: u32) -> Option<CellRect> {
        for y in 0..=CELL_COUNT - height {
            for x in 0..=CELL_COUNT - width {
                let cells = CellRect { x, y, width, height };
                if self.cells_are_free(cells) {
                    return Some(cells);
                }
            }
        }
        None
    }

    fn cells_are_free(&self, cells: CellRect) -> bool {
        (cells.y..cells.y + cells.height).all(|y| {
            (cells.x..cells.x + cells.width)
                .all(|x| self.occupancy[(y * CELL_COUNT + x) as usize].is_none())
        })
    }

    fn mark(&mut self, cells: CellRect, owner: Option<u64>) {
        for y in cells.y..cells.y + cells.height {
            for x in cells.x..cells.x + cells.width {
                self.occupancy[(y * CELL_COUNT + x) as usize] = owner;
            }
        }
    }

    fn eviction_candidate(&self) -> Option<u64> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.pinned_frame != self.frame)
            .min_by_key(|(id, entry)| (entry.image.strong_count() > 0, entry.last_use, **id))
            .map(|(id, _)| *id)
    }

    fn remove(&mut self, id: u64) {
        if let Some(entry) = self.entries.remove(&id) {
            self.mark(entry.cells, None);
        }
    }

    fn upload_pixels(&self, image: &RasterImage, queue: &wgpu::Queue, cells: CellRect) {
        let width = image.width() as usize;
        let height = image.height() as usize;
        let upload_width = width + EDGE_GUTTER as usize * 2;
        let upload_height = height + EDGE_GUTTER as usize * 2;
        let mut padded = vec![0; upload_width * upload_height * 4];
        for y in 0..upload_height {
            let source_y = y.saturating_sub(EDGE_GUTTER as usize).min(height - 1);
            for x in 0..upload_width {
                let source_x = x.saturating_sub(EDGE_GUTTER as usize).min(width - 1);
                let source_offset = (source_y * width + source_x) * 4;
                let target_offset = (y * upload_width + x) * 4;
                padded[target_offset..target_offset + 4]
                    .copy_from_slice(&image.pixels()[source_offset..source_offset + 4]);
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: cells.x * CELL_SIZE, y: cells.y * CELL_SIZE, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &padded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((upload_width * 4) as u32),
                rows_per_image: Some(upload_height as u32),
            },
            wgpu::Extent3d {
                width: upload_width as u32,
                height: upload_height as u32,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_image_is_not_evicted_until_next_frame() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
            backends: wgpu::Backends::PRIMARY,
        });
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: None,
                force_fallback_adapter: false,
                ..Default::default()
            }))
        else {
            eprintln!("skipping image atlas GPU test: no adapter");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("image atlas test device"),
            ..Default::default()
        }))
        .expect("image atlas test device must initialize");
        let mut atlas = ImageAtlas::new(&device);
        atlas.begin_frame();
        let oversized_width = IMAGE_ATLAS_SIZE - EDGE_GUTTER;
        let oversized = Arc::new(
            RasterImage::new(oversized_width, 1, vec![255; oversized_width as usize * 4])
                .expect("oversized atlas input is still a valid RGBA image"),
        );
        assert_eq!(atlas.get_or_upload(&oversized, &queue), Err(ImageAtlasError::TooLarge));
        let first = Arc::new(
            RasterImage::new(1, 1, vec![128, 64, 32, 255])
                .expect("one RGBA pixel must be accepted"),
        );
        let first_slot = atlas.get_or_upload(&first, &queue).expect("first image must fit");
        assert_eq!(atlas.get_or_upload(&first, &queue), Ok(first_slot));

        for cell in &mut atlas.occupancy {
            if cell.is_none() {
                *cell = Some(u64::MAX);
            }
        }
        let second = Arc::new(
            RasterImage::new(1, 1, vec![32, 64, 128, 255])
                .expect("one RGBA pixel must be accepted"),
        );
        assert_eq!(atlas.get_or_upload(&second, &queue), Err(ImageAtlasError::Full));
        assert!(atlas.entries.contains_key(&first.id()));

        atlas.begin_frame();
        assert_eq!(atlas.get_or_upload(&second, &queue), Ok(first_slot));
        assert!(!atlas.entries.contains_key(&first.id()));
        assert!(atlas.entries.contains_key(&second.id()));
    }
}
