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
    input::pointer::CursorImageStatus,
    utils::{IsAlive, Logical, Physical, Point, Rectangle, Scale},
};

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
    /// coordinates, at the given integer output `scale`), in `color`
    /// (changes with prefix-key state, StumpWM style).
    pub fn elements<R>(
        &mut self,
        geo: Rectangle<i32, Logical>,
        scale: i32,
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
            ((x, y), (w, t)),             // top
            ((x, y + h - t), (w, t)),     // bottom
            ((x, y + t), (t, h - 2 * t)), // left
            ((x + w - t, y + t), (t, h - 2 * t)), // right
        ];
        let mut out = Vec::with_capacity(4);
        for (buf, (loc, sz)) in self.buffers.iter_mut().zip(rects) {
            buf.update(sz, color);
            out.push(
                SolidColorRenderElement::from_buffer(
                    buf,
                    Point::<i32, Logical>::from(loc).to_physical(scale),
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

    let cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as i32;
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
            put_px(&mut data, stride, x, y, if border { MESSAGE_BORDER_COLOR } else { MESSAGE_BG });
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
                    if px < MESSAGE_BORDER || py < MESSAGE_BORDER || px >= w - MESSAGE_BORDER || py >= h - MESSAGE_BORDER {
                        continue;
                    }
                    let blend = |f: u8, b: u8| ((f as u32 * cov + b as u32 * (255 - cov)) / 255) as u8;
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

    MessageState { buffer, size: (w, h), generation }
}

/// Builds the render element showing `msg` centered on an output of
/// logical size `output_size`, or `None` if import fails.
pub fn message_element<R>(
    renderer: &mut R,
    msg: &MessageState,
    output_size: (i32, i32),
    scale: i32,
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
        loc.to_physical(scale).to_f64(),
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
    scale: i32,
) -> Option<MindeRenderElements<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
{
    match MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        loc.to_physical(scale).to_f64(),
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
/// client has set a surface-based cursor and no xcursor theme lookup is
/// performed (this compositor always uses the built-in fallback).
pub static FALLBACK_CURSOR_RGBA: &[u8] = include_bytes!("../resources/cursor.rgba");
const FALLBACK_CURSOR_SIZE: (i32, i32) = (64, 64);

/// Tracks the current pointer cursor: either our default fallback image or
/// a client-provided surface (set via `SeatHandler::cursor_image`).
pub struct CursorState {
    status: CursorImageStatus,
    default_buffer: MemoryRenderBuffer,
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
    /// the client's own surface while `Surface(..)`, our fallback image
    /// while `Named(..)`.
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
            CursorImageStatus::Named(_) => {
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
