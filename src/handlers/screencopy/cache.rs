// SPDX-License-Identifier: MIT
//! Bounded capture scenes shared by clients with identical output parameters.
use super::*;
use smithay::backend::renderer::{Blit, ErasedContextId, TextureFilter};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Instant,
};

type Rect = Rectangle<i32, BufferCoords>;
type Error = Box<dyn std::error::Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    scene: u64,
    revision: u64,
}

/// Last successful image for an ext session or wlr manager. Failed requests
/// never advance it. A different scene/evicted history conservatively damages
/// the full buffer, including when a manager switches outputs or regions.
#[derive(Clone, Default)]
pub struct CaptureHistory(Arc<Mutex<Option<Stamp>>>);

#[derive(Clone, PartialEq)]
pub(super) struct Key {
    pub output: Output,
    pub geometry: CaptureGeometry,
    pub output_geo: Rectangle<i32, Logical>,
    pub origin: Point<i32, Logical>,
    pub size: Size<i32, Physical>,
    pub cursor: bool,
    pub context: ErasedContextId,
}

#[derive(Default)]
pub struct CaptureCache {
    scenes: VecDeque<Scene>,
    next_id: u64,
}

impl CaptureCache {
    pub(super) fn take(&mut self, key: Key, scale: Scale<f64>) -> Scene {
        if let Some(index) = self.scenes.iter().position(|scene| scene.key == key) {
            return self.scenes.remove(index).unwrap();
        }
        self.next_id += 1;
        Scene {
            tracker: OutputDamageTracker::new(key.size, scale, Transform::Normal),
            key,
            stamp: Stamp {
                scene: self.next_id,
                revision: 0,
            },
            damage: VecDeque::new(),
            buffer: None,
            pixels: Vec::new(),
            unread: None,
            borders: BorderBuffers::default(),
        }
    }

    pub(super) fn put(&mut self, scene: Scene) {
        self.scenes.push_back(scene);
        // At most four variants and 128 MiB of image storage. Retain one
        // scene even when a single large output exceeds that byte budget.
        while self.scenes.len() > 1
            && (self.scenes.len() > 4
                || self
                    .scenes
                    .iter()
                    .map(|s| s.key.size.w as usize * s.key.size.h as usize * 4 + s.pixels.len())
                    .sum::<usize>()
                    > 128 * 1024 * 1024)
        {
            self.scenes.pop_front();
        }
    }

    pub fn clear(&mut self, output: Option<&Output>) {
        self.scenes
            .retain(|scene| output.is_some_and(|o| o != &scene.key.output));
    }
}

pub(super) struct Scene {
    pub key: Key,
    pub borders: BorderBuffers,
    tracker: OutputDamageTracker,
    stamp: Stamp,
    damage: VecDeque<(u64, Rect)>,
    buffer: Option<GlesRenderbuffer>,
    pixels: Vec<u8>,
    unread: Option<Rect>,
}

fn full(size: Size<i32, Physical>) -> Rect {
    Rectangle::from_size((size.w, size.h).into())
}

/// Clip client-supplied rectangles without overflowing their i32 extents.
pub(super) fn clipped(rect: Rect, size: Size<i32, Physical>) -> Option<Rect> {
    let x = i64::from(rect.loc.x).max(0);
    let y = i64::from(rect.loc.y).max(0);
    let right = (i64::from(rect.loc.x) + i64::from(rect.size.w)).min(i64::from(size.w));
    let bottom = (i64::from(rect.loc.y) + i64::from(rect.size.h)).min(i64::from(size.h));
    (right > x && bottom > y).then(|| {
        Rectangle::new(
            (x as i32, y as i32).into(),
            ((right - x) as i32, (bottom - y) as i32).into(),
        )
    })
}

impl Scene {
    pub fn changed_since(&self, history: Option<&CaptureHistory>) -> Vec<Rect> {
        let previous = history.and_then(|history| *history.0.lock().unwrap());
        let Some(previous) = previous.filter(|s| s.scene == self.stamp.scene) else {
            return vec![full(self.key.size)];
        };
        if previous.revision == self.stamp.revision {
            return Vec::new();
        }
        if self
            .damage
            .front()
            .is_none_or(|(revision, _)| previous.revision + 1 < *revision)
        {
            return vec![full(self.key.size)];
        }
        self.damage
            .iter()
            .filter(|(revision, _)| *revision > previous.revision)
            .map(|(_, rect)| *rect)
            .reduce(Rect::merge)
            .into_iter()
            .collect()
    }

    pub fn delivered(&self, history: Option<&CaptureHistory>) {
        if let Some(history) = history {
            *history.0.lock().unwrap() = Some(self.stamp);
        }
    }

    pub fn render<R>(
        &mut self,
        renderer: &mut R,
        elements: &[MindeRenderElements<R>],
        background: [f32; 4],
    ) -> Result<(), Error>
    where
        R: Renderer + ImportAll + ImportMem + Offscreen<GlesRenderbuffer>,
        R::TextureId: Send + Clone + 'static,
        R::Error: Send + Sync + 'static,
    {
        if self.buffer.is_none() {
            self.buffer = Some(renderer.create_buffer(Fourcc::Argb8888, full(self.key.size).size)?);
        }
        let mut framebuffer = renderer.bind(self.buffer.as_mut().unwrap())?;
        let start = Instant::now();
        let result =
            self.tracker
                .render_output(renderer, &mut framebuffer, 1, elements, background)?;
        let damaged = result
            .damage
            .and_then(|rects| rects.iter().copied().reduce(Rectangle::merge));
        result.sync.wait()?;
        if let Some(rect) = damaged {
            crate::timing::record(crate::timing::Probe::CaptureRender, start);
            let rect = Rect::new(
                (rect.loc.x, rect.loc.y).into(),
                (rect.size.w, rect.size.h).into(),
            );
            self.stamp.revision += 1;
            self.damage.push_back((self.stamp.revision, rect));
            while self.damage.len() > 16 {
                self.damage.pop_front();
            }
            // Keep the newest changes, including damage accumulated while
            // only DMA clients were reading the GPU image.
            self.unread = Some(self.unread.map_or(rect, |old| old.merge(rect)));
        }
        Ok(())
    }

    fn read_pixels<R>(&mut self, renderer: &mut R) -> Result<(), Error>
    where
        R: Renderer + ExportMem + Offscreen<GlesRenderbuffer>,
        R::Error: Send + Sync + 'static,
    {
        let region = if self.pixels.is_empty() {
            full(self.key.size)
        } else if let Some(region) = self.unread {
            region
        } else {
            return Ok(());
        };
        let _measurement = crate::timing::Measurement::start(crate::timing::Probe::CaptureReadback);
        let framebuffer = renderer.bind(self.buffer.as_mut().ok_or("capture not rendered")?)?;
        let mapping = renderer.copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)?;
        drop(framebuffer);
        let pixels = renderer.map_texture(&mapping)?;
        let stride = self.key.size.w as usize * 4;
        let width = region.size.w as usize * 4;
        if pixels.len() < width * region.size.h as usize {
            return Err("short capture readback".into());
        }
        self.pixels.resize(stride * self.key.size.h as usize, 0);
        for row in 0..region.size.h as usize {
            let to = (row + region.loc.y as usize) * stride + region.loc.x as usize * 4;
            self.pixels[to..to + width].copy_from_slice(&pixels[row * width..(row + 1) * width]);
        }
        self.unread = None;
        Ok(())
    }

    pub fn copy<R>(&mut self, renderer: &mut R, buffer: &WlBuffer, repair: &[Rect]) -> FillOutcome
    where
        R: Renderer + ExportMem + Offscreen<GlesRenderbuffer> + Bind<Dmabuf> + Blit,
        R::Error: Send + Sync + 'static,
    {
        use smithay::backend::renderer::{BufferType, buffer_dimensions, buffer_type};
        if !buffer_dimensions(buffer)
            .is_some_and(|size| size.w == self.key.size.w && size.h == self.key.size.h)
        {
            return FillOutcome::UnsupportedBuffer;
        }
        let region = repair
            .iter()
            .filter_map(|r| clipped(*r, self.key.size))
            .reduce(Rect::merge);
        let result = match buffer_type(buffer) {
            Some(BufferType::Dma) => self.copy_dma(renderer, buffer, region),
            Some(BufferType::Shm) => self.copy_shm(renderer, buffer, region),
            _ => return FillOutcome::UnsupportedBuffer,
        };
        match result {
            Ok(()) => FillOutcome::Ok(Vec::new()),
            Err(err) => {
                tracing::warn!(%err, "screencopy: copy failed");
                FillOutcome::RenderFailed
            }
        }
    }

    fn copy_dma<R>(
        &mut self,
        renderer: &mut R,
        buffer: &WlBuffer,
        region: Option<Rect>,
    ) -> Result<(), Error>
    where
        R: Renderer + Offscreen<GlesRenderbuffer> + Bind<Dmabuf> + Blit,
        R::Error: Send + Sync + 'static,
    {
        let Some(region) = region else {
            return Ok(());
        };
        let mut dma = smithay::wayland::dmabuf::get_dmabuf(buffer)?.clone();
        let source = renderer.bind(self.buffer.as_mut().ok_or("capture not rendered")?)?;
        let mut target = renderer.bind(&mut dma)?;
        let rect = Rectangle::new(
            (region.loc.x, region.loc.y).into(),
            (region.size.w, region.size.h).into(),
        );
        renderer
            .blit(&source, &mut target, rect, rect, TextureFilter::Nearest)?
            .wait()?;
        Ok(())
    }

    fn copy_shm<R>(
        &mut self,
        renderer: &mut R,
        buffer: &WlBuffer,
        region: Option<Rect>,
    ) -> Result<(), Error>
    where
        R: Renderer + ExportMem + Offscreen<GlesRenderbuffer>,
        R::Error: Send + Sync + 'static,
    {
        let format = with_buffer_contents_mut(buffer, |_, _, data| data.format)?;
        if !matches!(
            shm_format_to_fourcc(format),
            Some(Fourcc::Argb8888 | Fourcc::Xrgb8888)
        ) {
            return Err("unsupported capture format".into());
        }
        let Some(region) = region else {
            return Ok(());
        };
        self.read_pixels(renderer)?;
        with_buffer_contents_mut(buffer, |ptr, len, data| -> Result<(), Error> {
            let stride = self.key.size.w as usize * 4;
            if data.stride < self.key.size.w * 4 || data.offset < 0 {
                return Err("invalid capture stride".into());
            }
            let width = region.size.w as usize * 4;
            for row in region.loc.y as usize..(region.loc.y + region.size.h) as usize {
                let from = row * stride + region.loc.x as usize * 4;
                let to =
                    data.offset as usize + row * data.stride as usize + region.loc.x as usize * 4;
                if to.checked_add(width).is_none_or(|end| end > len) {
                    return Err("short capture buffer".into());
                }
                // Safety: source and pool bounds are validated; the Wayland
                // shm helper catches clients truncating their backing file.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        self.pixels.as_ptr().add(from),
                        ptr.add(to),
                        width,
                    );
                }
            }
            Ok(())
        })??;
        Ok(())
    }

    pub fn rgba<R>(&mut self, renderer: &mut R) -> Result<Vec<u8>, Error>
    where
        R: Renderer + ExportMem + Offscreen<GlesRenderbuffer>,
        R::Error: Send + Sync + 'static,
    {
        self.read_pixels(renderer)?;
        let mut pixels = self.pixels.clone();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        Ok(pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_damage_extents_cannot_overflow_or_escape_the_buffer() {
        let size = (640, 480).into();
        assert_eq!(
            clipped(
                Rect::new((630, 470).into(), (i32::MAX, i32::MAX).into()),
                size
            ),
            Some(Rect::new((630, 470).into(), (10, 10).into()))
        );
        assert_eq!(
            clipped(Rect::new((i32::MAX, 0).into(), (i32::MAX, 10).into()), size),
            None
        );
        assert_eq!(
            clipped(Rect::new((-10, -10).into(), (20, 20).into()), size),
            Some(Rect::from_size((10, 10).into()))
        );
        assert_eq!(
            clipped(Rect::new((0, 0).into(), (0, 480).into()), size),
            None
        );
    }
}
