// SPDX-License-Identifier: MIT

//! Shared render-element plumbing used by both the winit (nested) and udev
//! (DRM/TTY) backends: the `MindeRenderElements` enum, the
//! focused-window border builder, and a small cursor element helper
//! (adapted from Smithay's `anvil` example; see NOTICE for the exact
//! upstream revision).

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            solid::{SolidColorBuffer, SolidColorRenderElement},
            surface::WaylandSurfaceRenderElement,
        },
    },
    input::pointer::{CursorIcon, CursorImageStatus},
    utils::{IsAlive, Logical, Physical, Point, Rectangle, Scale},
};
use std::collections::HashMap;

// Space window surfaces (built per-window via `Window`'s `AsRenderElements`
// impl, which only needs `R: Renderer + ImportAll` -- we avoid nesting
// `desktop::space::SpaceRenderElements` in here since that drags in a much
// heavier bound set for no benefit at our scale), our solid-color focus
// border, and the default (or client-set) pointer cursor.
smithay::backend::renderer::element::render_elements! {
    pub MindeRenderElements<R> where R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Solid=SolidColorRenderElement,
    Cursor=MemoryRenderBufferRenderElement<R>,
}

/// Default focus-border color; policy may change it for modal states.
pub const BORDER_COLOR: [f32; 4] = [0.84, 0.60, 0.13, 1.0]; // #d79921

pub struct Appearance {
    pub border_width: i32,
    pub background: [f32; 4],
    pub message: MessageStyle,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            border_width: 3,
            background: [0.1, 0.1, 0.1, 1.0],
            message: MessageStyle::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageStyle {
    pub font_size: i32,
    pub padding: i32,
    pub border_width: i32,
    pub foreground: [u8; 4],
    pub background: [u8; 4],
    pub border_color: [u8; 4],
}

impl Default for MessageStyle {
    fn default() -> Self {
        Self {
            font_size: 17,
            padding: 12,
            border_width: 2,
            foreground: [0xeb, 0xdb, 0xb2, 0xff],
            background: [0x28, 0x28, 0x28, 0xff],
            border_color: [0xd7, 0x99, 0x21, 0xff],
        }
    }
}

/// A locked frame contains only the lock tree and its cursor. Share this
/// path between DRM and nested rendering, including cursor ownership checks.
pub(crate) fn lock_elements<R>(
    renderer: &mut R,
    surface: Option<&smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    geometry: Rectangle<i32, Logical>,
    scale: Scale<f64>,
    cursor: &mut CursorState,
    position: Point<f64, Logical>,
) -> Vec<MindeRenderElements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
{
    use smithay::reexports::wayland_server::Resource;
    let Some(surface) = surface else {
        return Vec::new();
    };
    let mut elements = Vec::new();
    let cursor_owned = match cursor.status() {
        CursorImageStatus::Surface(image) => image.is_alive() && image.client() == surface.client(),
        _ => true,
    };
    if cursor_owned && geometry.to_f64().contains(position) {
        let location = (position - geometry.loc.to_f64() - cursor.hotspot().to_f64())
            .to_physical(scale)
            .to_i32_round();
        elements.extend(cursor.render_elements(renderer, location, scale));
    }
    elements.extend(
        smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
            renderer,
            surface,
            (0, 0),
            scale,
            1.0,
            Kind::Unspecified,
        ),
    );
    elements
}

pub(crate) fn send_lock_frames(
    output: &smithay::output::Output,
    geometry: Rectangle<i32, Logical>,
    surface: Option<&smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    cursor: &CursorState,
    position: Point<f64, Logical>,
    time: std::time::Duration,
) {
    use smithay::reexports::wayland_server::Resource;
    let Some(surface) = surface else {
        return;
    };
    let send = |surface| {
        smithay::desktop::utils::send_frames_surface_tree(
            surface,
            output,
            time,
            Some(std::time::Duration::ZERO),
            |_, _| Some(output.clone()),
        )
    };
    send(surface);
    if geometry.to_f64().contains(position)
        && let CursorImageStatus::Surface(image) = cursor.status()
        && image.is_alive()
        && image.client() == surface.client()
    {
        send(image);
    }
}

/// Persistent buffers for the 4 border edges (stable element ids keep
/// damage tracking incremental across frames).
pub struct BorderBuffers {
    buffers: [SolidColorBuffer; 4],
}

impl Default for BorderBuffers {
    fn default() -> Self {
        Self {
            buffers: std::array::from_fn(|_| SolidColorBuffer::new((0, 0), BORDER_COLOR)),
        }
    }
}

impl BorderBuffers {
    /// Builds the 4 border-edge render elements around `geo` (in logical
    /// coordinates, at the given output `scale`), in `color` (changes with
    /// prefix-key state, StumpWM style). `scale` is the output's fractional
    /// scale so the border tracks the same density as window content.
    pub fn elements<R>(
        &mut self,
        geo: Rectangle<i32, Logical>,
        scale: Scale<f64>,
        color: [f32; 4],
        width: i32,
    ) -> Vec<MindeRenderElements<R>>
    where
        R: Renderer + ImportAll + ImportMem,
    {
        let (x, y, w, h) = (geo.loc.x, geo.loc.y, geo.size.w, geo.size.h);
        let t = width.min((w - 1).max(0) / 2).min((h - 1).max(0) / 2);
        if t == 0 {
            return Vec::new();
        }
        // Drawn *inside* the rectangle: a frame filling the whole output
        // would otherwise have its border entirely off-screen.
        let rects = [
            ((x, y), (w, t)),                     // top
            ((x, y + h - t), (w, t)),             // bottom
            ((x, y + t), (t, h - 2 * t)),         // left
            ((x + w - t, y + t), (t, h - 2 * t)), // right
        ];
        let mut out = Vec::with_capacity(4);
        for (buf, (loc, sz)) in self.buffers.iter_mut().zip(rects) {
            buf.update(sz, color);
            out.push(
                SolidColorRenderElement::from_buffer(
                    buf,
                    Point::<i32, Logical>::from(loc).to_physical_precise_round(scale),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                )
                .into(),
            );
        }
        out
    }
}

// ---------------------------------------------------------------------
// Message area (StumpWM-style echo window)
// ---------------------------------------------------------------------

/// Embedded monospace font for the message overlay (DejaVu Sans Mono,
/// from Guix font-dejavu; the DejaVu license permits embedding).
static MESSAGE_FONT_BYTES: &[u8] = include_bytes!("../resources/DejaVuSansMono.ttf");

/// The parsed message font, parsed once: `Font::from_bytes` re-reads the
/// whole TTF, which is far too much for every echo message and overlay label.
fn message_font() -> &'static fontdue::Font {
    static FONT: std::sync::OnceLock<fontdue::Font> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        fontdue::Font::from_bytes(MESSAGE_FONT_BYTES, fontdue::FontSettings::default())
            .expect("embedded font must parse")
    })
}

/// Cache glyphs by character and physical font size, with an 8 MiB budget.
fn rasterize_cached(
    font: &fontdue::Font,
    ch: char,
    px: i32,
) -> (fontdue::Metrics, std::sync::Arc<[u8]>) {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    type Glyph = (fontdue::Metrics, Arc<[u8]>);
    type Cache = (HashMap<(char, i32), Glyph>, usize);
    static GLYPHS: Mutex<Option<Cache>> = Mutex::new(None);
    let mut guard = GLYPHS.lock().unwrap();
    let (cache, bytes) = guard.get_or_insert_with(|| (HashMap::new(), 0));
    if let Some(glyph) = cache.get(&(ch, px)) {
        return glyph.clone();
    }
    let (metrics, bitmap) = font.rasterize(ch, px as f32);
    let len = bitmap.len();
    let glyph = (metrics, Arc::from(bitmap));
    if *bytes + len > 8 * 1024 * 1024 || cache.len() >= 1024 {
        cache.clear();
        *bytes = 0;
    }
    if len <= 8 * 1024 * 1024 {
        *bytes += len;
        cache.insert((ch, px), glyph.clone());
    }
    glyph
}

/// Message content with up to four cached density/style/size variants.
pub struct MessageState {
    text: String,
    limits: (i32, i32),
    buffers: std::cell::RefCell<Vec<(MessageKey, MessageBuffer)>>,
    pub generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MessageKey {
    style: MessageStyle,
    scale: i32,
    bounds: (i32, i32),
}

#[derive(Clone)]
pub struct MessageBuffer {
    pub buffer: MemoryRenderBuffer,
    pub size: (i32, i32),
    bytes: usize,
}

impl MessageState {
    pub fn new(text: &str, generation: u64, limits: (i32, i32)) -> Self {
        Self {
            text: text.into(),
            generation,
            limits,
            buffers: Default::default(),
        }
    }

    pub fn prepare(
        &self,
        style: MessageStyle,
        scale: Scale<f64>,
        bounds: (i32, i32),
    ) -> MessageBuffer {
        let key = MessageKey {
            style,
            scale: cursor_scale(scale),
            bounds: (
                bounds.0.min(self.limits.0).max(1),
                bounds.1.min(self.limits.1).max(1),
            ),
        };
        let mut buffers = self.buffers.borrow_mut();
        if let Some((_, buffer)) = buffers.iter().find(|(existing, _)| *existing == key) {
            return buffer.clone();
        }
        let buffer = rasterize_message(&self.text, key);
        // Keep one oversized image, but do not multiply it across variants.
        while !buffers.is_empty()
            && (buffers.len() >= 4
                || buffers.iter().map(|(_, b)| b.bytes).sum::<usize>() + buffer.bytes
                    > 16 * 1024 * 1024)
        {
            buffers.remove(0);
        }
        buffers.push((key, buffer.clone()));
        buffer
    }
}

fn put_px(data: &mut [u8], stride: usize, x: i32, y: i32, rgba: [u8; 4]) {
    let off = y as usize * stride + x as usize * 4;
    // DRM Argb8888 is packed little-endian: B G R A in memory.
    data[off] = rgba[2];
    data[off + 1] = rgba[1];
    data[off + 2] = rgba[0];
    data[off + 3] = rgba[3];
}

/// Layout in logical pixels, rasterize at integer-ceil output density.
fn rasterize_message(text: &str, key: MessageKey) -> MessageBuffer {
    let font = message_font();
    let (style, scale) = (key.style, key.scale);
    let (max_w, max_h) = key.bounds;
    let font_px = style.font_size as f32;
    let advance = font.metrics('M', font_px).advance_width.ceil() as i32;
    let line_h = (font_px * 1.35).ceil() as i32;
    let baseline = (font_px * 1.05).ceil() as i32;

    // Hard-wrap to the width budget, clamp to the height budget.
    let inset = style.padding + style.border_width;
    let max_cols = (((max_w * 8 / 10) - 2 * inset) / advance.max(1)).max(1) as usize;
    let max_lines = ((((max_h * 8 / 10) - 2 * inset) / line_h.max(1)).max(1)) as usize;
    let mut lines: Vec<String> = Vec::new();
    'outer: for raw in text.split('\n') {
        let chars: Vec<char> = raw.chars().collect();
        if chars.is_empty() {
            lines.push(String::new());
        }
        for chunk in chars.chunks(max_cols) {
            lines.push(chunk.iter().collect());
            if lines.len() >= max_lines {
                break 'outer;
            }
        }
        if lines.len() >= max_lines {
            break;
        }
    }

    let cols = lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(1)
        .max(1) as i32;
    let size = (
        (cols * advance + 2 * inset).clamp(1, max_w),
        (lines.len() as i32 * line_h + 2 * inset).clamp(1, max_h),
    );
    let (w, h) = (size.0 * scale, size.1 * scale);
    let border_width = style.border_width * scale;

    let stride = w as usize * 4;
    let mut data = vec![0u8; stride * h as usize];

    for y in 0..h {
        for x in 0..w {
            let border = x < border_width
                || y < border_width
                || x >= w - border_width
                || y >= h - border_width;
            put_px(
                &mut data,
                stride,
                x,
                y,
                if border {
                    style.border_color
                } else {
                    style.background
                },
            );
        }
    }

    for (row, line) in lines.iter().enumerate() {
        let y0 = inset + row as i32 * line_h;
        for (col, ch) in line.chars().enumerate() {
            let (metrics, bitmap) = rasterize_cached(font, ch, style.font_size * scale);
            let gx = (inset + col as i32 * advance) * scale + metrics.xmin;
            let gy = (y0 + baseline) * scale - metrics.height as i32 - metrics.ymin;
            for by in 0..metrics.height as i32 {
                for bx in 0..metrics.width as i32 {
                    let cov = bitmap[(by * metrics.width as i32 + bx) as usize] as u32;
                    if cov == 0 {
                        continue;
                    }
                    let (px, py) = (gx + bx, gy + by);
                    if px < border_width
                        || py < border_width
                        || px >= w - border_width
                        || py >= h - border_width
                    {
                        continue;
                    }
                    let blend =
                        |f: u8, b: u8| ((f as u32 * cov + b as u32 * (255 - cov)) / 255) as u8;
                    let rgba = [
                        blend(style.foreground[0], style.background[0]),
                        blend(style.foreground[1], style.background[1]),
                        blend(style.foreground[2], style.background[2]),
                        0xff,
                    ];
                    put_px(&mut data, stride, px, py, rgba);
                }
            }
        }
    }

    let buffer = MemoryRenderBuffer::from_slice(
        &data,
        smithay::backend::allocator::Fourcc::Argb8888,
        (w, h),
        scale,
        smithay::utils::Transform::Normal,
        None,
    );

    MessageBuffer {
        buffer,
        size,
        bytes: data.len(),
    }
}

/// Builds an overlay at logical position `loc` relative to the render target,
/// or `None` if import fails.
pub fn overlay_element<R>(
    renderer: &mut R,
    msg: &MessageBuffer,
    loc: Point<i32, Logical>,
    scale: Scale<f64>,
) -> Option<MindeRenderElements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
{
    match MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        loc.to_f64().to_physical(scale),
        &msg.buffer,
        None,
        None,
        None,
        Kind::Unspecified,
    ) {
        Ok(elem) => Some(elem.into()),
        Err(err) => {
            tracing::warn!(?err, "failed to render overlay element");
            None
        }
    }
}

/// Fallback pointer cursor image (a plain 64x64 RGBA arrow), used when no
/// client has set a surface-based cursor and no xcursor theme lookup
/// succeeds (bare test environments with no cursor theme installed).
pub static FALLBACK_CURSOR_RGBA: &[u8] = include_bytes!("../resources/cursor.rgba");
const FALLBACK_CURSOR_SIZE: (i32, i32) = (64, 64);

/// Default Xcursor base size (logical px) when `XCURSOR_SIZE` is unset,
/// matching the freedesktop/GTK convention.
const DEFAULT_CURSOR_SIZE: u32 = 24;
/// Default Xcursor theme name when `XCURSOR_THEME` is unset. The xcursor
/// crate falls back to the "default" theme's inheritance chain.
const DEFAULT_CURSOR_THEME: &str = "default";

/// A decoded, ready-to-render themed cursor image. Animated cursors are
/// loaded as their first frame only (documented limitation); the crate
/// returns frames in file order and we keep the first of the chosen size.
#[derive(Clone)]
struct ThemedCursor {
    buffer: MemoryRenderBuffer,
    /// Logical hotspot; preserve fractions until rounding for the output.
    hotspot: Point<f64, Logical>,
}

/// Loads Xcursor theme images honoring `XCURSOR_THEME`/`XCURSOR_SIZE`, and
/// caches decoded images per (shape name, integer output scale). The size
/// requested from the theme scales with the output's (integer-ceil) scale
/// so cursors stay crisp on HiDPI outputs.
struct XCursorLoader {
    theme: xcursor::CursorTheme,
    base_size: u32,
    cache: HashMap<(&'static str, i32), Option<ThemedCursor>>,
}

impl XCursorLoader {
    fn from_env() -> Self {
        let theme_name = std::env::var("XCURSOR_THEME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_CURSOR_THEME.to_string());
        let base_size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|s| *s > 0)
            .unwrap_or(DEFAULT_CURSOR_SIZE);
        Self {
            theme: xcursor::CursorTheme::load(&theme_name),
            base_size,
            cache: HashMap::new(),
        }
    }

    /// Returns the themed image for `icon` at the given integer output
    /// scale, or `None` when the theme (or the icon within it) is absent.
    fn get(&mut self, icon: CursorIcon, int_scale: i32) -> Option<ThemedCursor> {
        let key = (icon.name(), int_scale);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let loaded = self.load(icon, int_scale);
        self.cache.insert(key, loaded.clone());
        loaded
    }

    fn load(&self, icon: CursorIcon, int_scale: i32) -> Option<ThemedCursor> {
        let target = self.base_size.saturating_mul(int_scale.max(1) as u32);
        // Try the canonical CSS/cursor-shape name first, then the legacy
        // Xcursor aliases (e.g. "left_ptr" for Default).
        let path = std::iter::once(icon.name())
            .chain(icon.alt_names().iter().copied())
            .find_map(|name| self.theme.load_icon(name))?;
        let bytes = std::fs::read(path).ok()?;
        let images = xcursor::parser::parse_xcursor(&bytes)?;
        // Pick the nominal size closest to the target; among equal-size
        // frames `min_by_key` keeps the first (frame 0 of an animation).
        let image = images
            .iter()
            .min_by_key(|img| (img.size as i64 - target as i64).abs())?;
        // Xcursor stores each pixel as a little-endian 0xAARRGGBB u32, so the
        // raw file bytes (`pixels_rgba`) are B,G,R,A in memory -- exactly the
        // DRM Argb8888 layout the rest of the renderer expects.
        let buffer_scale = int_scale.max(1);
        let buffer = MemoryRenderBuffer::from_slice(
            &image.pixels_rgba,
            smithay::backend::allocator::Fourcc::Argb8888,
            (image.width as i32, image.height as i32),
            buffer_scale,
            smithay::utils::Transform::Normal,
            None,
        );
        Some(ThemedCursor {
            buffer,
            hotspot: Point::from((
                f64::from(image.xhot) / f64::from(buffer_scale),
                f64::from(image.yhot) / f64::from(buffer_scale),
            )),
        })
    }
}

/// Selects the integer output scale used to size themed cursors: the
/// ceiling of the (fractional) output scale, at least 1.
pub fn cursor_scale(scale: Scale<f64>) -> i32 {
    (scale.x.max(scale.y).max(1.0).ceil() as i32).max(1)
}

/// Tracks the current pointer cursor: a themed/named cursor (default or
/// requested via `wp_cursor_shape_v1`), or a client-provided surface (set
/// via `SeatHandler::cursor_image`). Named cursors resolve through the
/// Xcursor theme, falling back to the built-in bitmap when no theme is
/// available.
pub struct CursorState {
    status: CursorImageStatus,
    default_buffer: MemoryRenderBuffer,
    loader: XCursorLoader,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            status: CursorImageStatus::default_named(),
            default_buffer: MemoryRenderBuffer::from_slice(
                FALLBACK_CURSOR_RGBA,
                smithay::backend::allocator::Fourcc::Argb8888,
                FALLBACK_CURSOR_SIZE,
                1,
                smithay::utils::Transform::Normal,
                None,
            ),
            loader: XCursorLoader::from_env(),
        }
    }
}

impl CursorState {
    pub fn set_status(&mut self, status: CursorImageStatus) {
        self.status = status;
    }

    pub fn status(&self) -> &CursorImageStatus {
        &self.status
    }

    /// Renders the cursor at physical `location`, honoring
    /// `CursorImageStatus`: hidden while a client requested no cursor,
    /// the client's own surface while `Surface(..)`, and the themed
    /// Xcursor image (or built-in fallback) while `Named(..)`.
    pub fn render_elements<R>(
        &mut self,
        renderer: &mut R,
        location: Point<i32, Physical>,
        scale: Scale<f64>,
    ) -> Vec<MindeRenderElements<R>>
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        match &self.status {
            CursorImageStatus::Hidden => vec![],
            CursorImageStatus::Named(icon) => {
                let icon = *icon;
                let int_scale = cursor_scale(scale);
                // Match the hotspot to the image's physical output scale,
                // including when the cached image was loaded at a larger scale.
                if let Some(themed) = self.loader.get(icon, int_scale) {
                    let loc = location - themed.hotspot.to_physical(scale).to_i32_round();
                    match MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        loc.to_f64(),
                        &themed.buffer,
                        None,
                        None,
                        None,
                        Kind::Cursor,
                    ) {
                        Ok(elem) => return vec![elem.into()],
                        Err(err) => {
                            tracing::warn!(?err, "failed to render themed cursor");
                        }
                    }
                }
                // No theme (or import failure): built-in fallback bitmap.
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    location.to_f64(),
                    &self.default_buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    Ok(elem) => vec![elem.into()],
                    Err(err) => {
                        tracing::warn!(?err, "failed to render fallback cursor");
                        vec![]
                    }
                }
            }
            CursorImageStatus::Surface(surface) => {
                if !surface.alive() {
                    self.status = CursorImageStatus::default_named();
                    return vec![];
                }
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    surface,
                    location,
                    scale,
                    1.0,
                    Kind::Cursor,
                )
            }
        }
    }

    /// Hotspot offset (in surface-local logical coordinates) to subtract
    /// from the pointer location before rendering, when a client surface
    /// cursor is active.
    pub fn hotspot(&self) -> Point<i32, Logical> {
        if let CursorImageStatus::Surface(surface) = &self.status {
            smithay::wayland::compositor::with_states(surface, |states| {
                states
                    .data_map
                    .get::<std::sync::Mutex<smithay::input::pointer::CursorImageAttributes>>()
                    .map(|attrs| attrs.lock().unwrap().hotspot)
                    .unwrap_or_default()
            })
        } else {
            (0, 0).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(message: &mut MessageBuffer) -> Vec<u8> {
        let mut bytes = Vec::new();
        message
            .buffer
            .render()
            .draw::<_, std::convert::Infallible>(|data| {
                bytes.extend_from_slice(data);
                Ok(Vec::new())
            })
            .unwrap();
        bytes
    }

    #[test]
    fn message_layout_wraps_and_stays_inside_the_output_budget() {
        let message = MessageState::new(&"abcdefghij".repeat(200), 41, (1280, 800));
        assert_eq!(message.generation, 41);
        let message = message.prepare(MessageStyle::default(), Scale::from(1.0), (1280, 800));
        assert!(message.size.0 > 0 && message.size.0 <= 1280);
        assert!(message.size.1 > 0 && message.size.1 <= 800);
    }

    #[test]
    fn empty_message_still_has_a_renderable_box() {
        let message = MessageState::new("", 0, (640, 480)).prepare(
            MessageStyle::default(),
            Scale::from(1.0),
            (640, 480),
        );
        assert!(message.size.0 > 0);
        assert!(message.size.1 > 0);
    }

    #[test]
    fn message_density_changes_buffer_pixels_without_changing_logical_layout() {
        let message = MessageState::new("fractional text", 0, (1280, 800));
        let style = MessageStyle::default();
        let mut normal = message.prepare(style, Scale::from(1.0), (1280, 800));
        let mut fractional = message.prepare(style, Scale::from(1.5), (1280, 800));
        assert_eq!(normal.size, fractional.size);
        assert_eq!(
            pixels(&mut normal).len(),
            (normal.size.0 * normal.size.1 * 4) as usize
        );
        assert_eq!(
            pixels(&mut fractional).len(),
            (normal.size.0 * normal.size.1 * 16) as usize
        );
        let mut again = message.prepare(style, Scale::from(2.0), (1280, 800));
        assert_eq!(pixels(&mut again), pixels(&mut fractional));
        assert_eq!(message.buffers.borrow().len(), 2);
        let bigger = MessageStyle {
            font_size: 24,
            ..style
        };
        let large = message.prepare(bigger, Scale::from(1.5), (1280, 800));
        assert!(large.size.0 > normal.size.0 && large.size.1 > normal.size.1);
        let tiny = message.prepare(bigger, Scale::from(1.5), (8, 4));
        assert!(tiny.size.0 <= 8 && tiny.size.1 <= 4);
        for font_size in 6..30 {
            message.prepare(
                MessageStyle { font_size, ..style },
                Scale::from(1.0),
                (1280, 800),
            );
        }
        assert!(message.buffers.borrow().len() <= 4);
    }

    #[test]
    fn cursor_scale_is_integer_ceiling_at_least_one() {
        assert_eq!(cursor_scale(Scale::from(1.0)), 1);
        assert_eq!(cursor_scale(Scale::from(1.5)), 2);
        assert_eq!(cursor_scale(Scale::from(2.0)), 2);
        assert_eq!(cursor_scale(Scale::from(2.01)), 3);
        // Degenerate/zero scales never size a cursor below 1x.
        assert_eq!(cursor_scale(Scale::from(0.0)), 1);
    }

    #[test]
    fn cursor_icon_names_prefer_canonical_then_legacy_aliases() {
        // cursor-shape maps Shape::Default -> CursorIcon::Default; the
        // themed loader looks up the canonical CSS name first, then the
        // legacy Xcursor aliases so classic themes (left_ptr) still match.
        let icon = CursorIcon::Default;
        assert_eq!(icon.name(), "default");
        assert!(icon.alt_names().contains(&"left_ptr"));

        // A resize shape resolves to its canonical hyphenated name.
        assert_eq!(CursorIcon::EwResize.name(), "ew-resize");
    }

    #[test]
    fn xcursor_loader_size_defaults_and_env_overrides() {
        // Defaults with no env set.
        unsafe {
            std::env::remove_var("XCURSOR_SIZE");
            std::env::remove_var("XCURSOR_THEME");
        }
        assert_eq!(XCursorLoader::from_env().base_size, DEFAULT_CURSOR_SIZE);

        // Valid override is honored; invalid/zero falls back to the default.
        unsafe {
            std::env::set_var("XCURSOR_SIZE", "48");
        }
        assert_eq!(XCursorLoader::from_env().base_size, 48);
        unsafe {
            std::env::set_var("XCURSOR_SIZE", "0");
        }
        assert_eq!(XCursorLoader::from_env().base_size, DEFAULT_CURSOR_SIZE);
        unsafe {
            std::env::set_var("XCURSOR_SIZE", "bogus");
        }
        assert_eq!(XCursorLoader::from_env().base_size, DEFAULT_CURSOR_SIZE);
        unsafe {
            std::env::remove_var("XCURSOR_SIZE");
        }
    }
}
