// SPDX-License-Identifier: MIT

//! Shared render-element plumbing used by both the winit (nested) and udev
//! (DRM/TTY) backends: the `MindeRenderElements` enum, the
//! focused-window border builder, and a small cursor element helper
//! (adapted from Smithay's `anvil` example; see README for the exact
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

/// Focused-window border: gruvbox yellow, matching the user's StumpWM theme.
pub const BORDER_COLOR: [f32; 4] = [0.84, 0.60, 0.13, 1.0]; // #d79921
pub const BORDER_WIDTH: i32 = 3;

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
    ) -> Vec<MindeRenderElements<R>>
    where
        R: Renderer + ImportAll + ImportMem,
    {
        let t = BORDER_WIDTH;
        let (x, y, w, h) = (geo.loc.x, geo.loc.y, geo.size.w, geo.size.h);
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

const MESSAGE_FONT_PX: f32 = 17.0;
const MESSAGE_PAD: i32 = 12;
const MESSAGE_BORDER: i32 = 2;
// Gruvbox, matching the user's StumpWM theme.
const MESSAGE_BG: [u8; 4] = [0x28, 0x28, 0x28, 0xff]; // #282828
const MESSAGE_FG: [u8; 4] = [0xeb, 0xdb, 0xb2, 0xff]; // #ebdbb2
const MESSAGE_BORDER_COLOR: [u8; 4] = [0xd7, 0x99, 0x21, 0xff]; // #d79921

/// A rasterized message ready to render, plus the generation counter used
/// so a stale hide-timer can't clear a newer message.
pub struct MessageState {
    pub buffer: MemoryRenderBuffer,
    pub size: (i32, i32),
    pub generation: u64,
}

fn put_px(data: &mut [u8], stride: usize, x: i32, y: i32, rgba: [u8; 4]) {
    let off = y as usize * stride + x as usize * 4;
    // DRM Argb8888 is packed little-endian: B G R A in memory.
    data[off] = rgba[2];
    data[off + 1] = rgba[1];
    data[off + 2] = rgba[0];
    data[off + 3] = rgba[3];
}

/// Rasterizes `text` (multi-line) into an ARGB `MemoryRenderBuffer` styled
/// like StumpWM's message window. Lines are hard-wrapped/clamped to fit
/// within `max_w` x `max_h` (pass the output size).
pub fn render_message(text: &str, generation: u64, max_w: i32, max_h: i32) -> MessageState {
    let font = fontdue::Font::from_bytes(MESSAGE_FONT_BYTES, fontdue::FontSettings::default())
        .expect("embedded font must parse");

    let advance = font.metrics('M', MESSAGE_FONT_PX).advance_width.ceil() as i32;
    let line_h = (MESSAGE_FONT_PX * 1.35).ceil() as i32;
    let baseline = (MESSAGE_FONT_PX * 1.05).ceil() as i32;

    // Hard-wrap to the width budget, clamp to the height budget.
    let inset = MESSAGE_PAD + MESSAGE_BORDER;
    let max_cols = (((max_w * 8 / 10) - 2 * inset) / advance.max(1)).max(8) as usize;
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
    let w = cols * advance + 2 * inset;
    let h = lines.len() as i32 * line_h + 2 * inset;

    let stride = w as usize * 4;
    let mut data = vec![0u8; stride * h as usize];

    for y in 0..h {
        for x in 0..w {
            let border = x < MESSAGE_BORDER
                || y < MESSAGE_BORDER
                || x >= w - MESSAGE_BORDER
                || y >= h - MESSAGE_BORDER;
            put_px(
                &mut data,
                stride,
                x,
                y,
                if border {
                    MESSAGE_BORDER_COLOR
                } else {
                    MESSAGE_BG
                },
            );
        }
    }

    for (row, line) in lines.iter().enumerate() {
        let y0 = inset + row as i32 * line_h;
        for (col, ch) in line.chars().enumerate() {
            let (metrics, bitmap) = font.rasterize(ch, MESSAGE_FONT_PX);
            let gx = inset + col as i32 * advance + metrics.xmin;
            let gy = y0 + baseline - metrics.height as i32 - metrics.ymin;
            for by in 0..metrics.height as i32 {
                for bx in 0..metrics.width as i32 {
                    let cov = bitmap[(by * metrics.width as i32 + bx) as usize] as u32;
                    if cov == 0 {
                        continue;
                    }
                    let (px, py) = (gx + bx, gy + by);
                    if px < MESSAGE_BORDER
                        || py < MESSAGE_BORDER
                        || px >= w - MESSAGE_BORDER
                        || py >= h - MESSAGE_BORDER
                    {
                        continue;
                    }
                    let blend =
                        |f: u8, b: u8| ((f as u32 * cov + b as u32 * (255 - cov)) / 255) as u8;
                    let rgba = [
                        blend(MESSAGE_FG[0], MESSAGE_BG[0]),
                        blend(MESSAGE_FG[1], MESSAGE_BG[1]),
                        blend(MESSAGE_FG[2], MESSAGE_BG[2]),
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
        1,
        smithay::utils::Transform::Normal,
        None,
    );

    MessageState {
        buffer,
        size: (w, h),
        generation,
    }
}

/// Builds the render element showing `msg` centered on an output of
/// logical size `output_size`, or `None` if import fails.
pub fn message_element<R>(
    renderer: &mut R,
    msg: &MessageState,
    output_size: (i32, i32),
    scale: Scale<f64>,
) -> Option<MindeRenderElements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
{
    let loc = Point::<i32, Logical>::from((
        (output_size.0 - msg.size.0) / 2,
        (output_size.1 - msg.size.1) / 2,
    ));
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
            tracing::warn!(?err, "failed to render message element");
            None
        }
    }
}

/// Builds the render element showing `msg` at an explicit output-local
/// logical position (fselect/expose frame-number overlays), or `None`
/// if import fails.
pub fn overlay_element<R>(
    renderer: &mut R,
    msg: &MessageState,
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
    /// Hotspot in cursor-image pixels (i.e. physical px at load scale).
    hotspot: Point<i32, Physical>,
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
        let buffer = MemoryRenderBuffer::from_slice(
            &image.pixels_rgba,
            smithay::backend::allocator::Fourcc::Argb8888,
            (image.width as i32, image.height as i32),
            int_scale.max(1),
            smithay::utils::Transform::Normal,
            None,
        );
        Some(ThemedCursor {
            buffer,
            hotspot: Point::from((image.xhot as i32, image.yhot as i32)),
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
                // Themed cursor: offset the physical location by the image
                // hotspot (in physical px) so the tip lands on the pointer.
                if let Some(themed) = self.loader.get(icon, int_scale) {
                    let loc = location - themed.hotspot;
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

    #[test]
    fn message_layout_wraps_and_stays_inside_the_output_budget() {
        let message = render_message(&"abcdefghij".repeat(200), 41, 1280, 800);
        assert_eq!(message.generation, 41);
        assert!(message.size.0 > 0 && message.size.0 <= 1280);
        assert!(message.size.1 > 0 && message.size.1 <= 800);
    }

    #[test]
    fn empty_message_still_has_a_renderable_box() {
        let message = render_message("", 0, 640, 480);
        assert!(message.size.0 > 0);
        assert!(message.size.1 > 0);
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
