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
    /// coordinates, at the given integer output `scale`).
    pub fn elements<R>(
        &mut self,
        geo: Rectangle<i32, Logical>,
        scale: i32,
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
            buf.update(sz, BORDER_COLOR);
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
