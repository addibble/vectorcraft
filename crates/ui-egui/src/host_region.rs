//! A region a host page marks on one of the document's images (`vectorcraft:region`,
//! docs/web-embed.md, host-only): a box in the image's own relative units (0..1 from its top-left
//! corner), and optionally a picture the page lays over the image (its own alpha). Both are drawn
//! through the image's placement, so they follow it moved, scaled or rotated, and the view zoomed,
//! turned or scrolled, as Crop Image's box does. While the image (or the clip group holding it) is
//! selected with the Selection or Direct Selection tool, the box is dragged by its inside and
//! resized by its corners, and the page hears where it was let go (`vectorcraft:regionChanged`).
//! Nothing here is the document's: no undo step, nothing saved, the tool untouched.

use std::sync::Arc;

use egui::{Color32, Pos2, Sense, Stroke, Ui};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Affine, Point};

use crate::VectorcraftApp;
use crate::canvas::Xf;

/// The largest overlay picture the page gives, in bytes and pixels a side.
pub const MAX_OVERLAY_BYTES: usize = 8 << 20;
pub const MAX_OVERLAY_SIDE: u32 = 4096;

/// The smallest the box gets, as a share of the image on each side.
pub const MIN_SHARE: f64 = 0.05;

/// The box's colour: the page's subject green.
const GREEN: Color32 = Color32::from_rgb(0x00, 0xc4, 0x6a);

/// How near a corner the pointer takes it, in screen points.
const GRIP: f32 = 7.0;

/// A box in an image's relative units: left, top, right, bottom.
pub type Rel = [f64; 4];

/// The page's region on one image of one document.
#[derive(Clone)]
pub struct HostRegion {
    /// The document it was marked in ([`vectorcraft_engine::DocState::uid`]); drawn only while active.
    pub doc: u64,
    pub image: NodeId,
    pub rect: Rel,
    pub editable: bool,
    /// The overlay's pixels and its texture once made.
    overlay: Option<(Arc<egui::ColorImage>, Option<egui::TextureHandle>)>,
}

impl std::fmt::Debug for HostRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostRegion")
            .field("doc", &self.doc)
            .field("image", &self.image)
            .field("rect", &self.rect)
            .field("editable", &self.editable)
            .finish()
    }
}

/// The region, and where the page's box was let go since the page last heard.
#[derive(Clone, Debug, Default)]
pub struct HostRegionState {
    pub region: Option<HostRegion>,
    pub changed: Vec<(NodeId, Rel)>,
}

/// What a drag of the box holds: what it started as, and which part of it is held.
#[derive(Clone, Copy, Debug)]
struct Held {
    start: Rel,
    /// The pointer where it was pressed, in the image's relative units.
    from: Point,
    /// A corner (0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left), or the inside.
    corner: Option<usize>,
}

fn held_id() -> egui::Id {
    egui::Id::unique("host-region-held")
}

/// `rect` kept inside the image and at least [`MIN_SHARE`] of it on each side.
pub fn clamped(r: Rel) -> Rel {
    let w = (r[2] - r[0]).clamp(MIN_SHARE, 1.0);
    let h = (r[3] - r[1]).clamp(MIN_SHARE, 1.0);
    let left = r[0].clamp(0.0, 1.0 - w);
    let top = r[1].clamp(0.0, 1.0 - h);
    [left, top, left + w, top + h]
}

/// Mark the page's region on image `image` of the active document, in place of any other; the
/// overlay, a PNG, is decoded now. An id that isn't an image of the active document, or an
/// overlay that isn't a PNG of at most [`MAX_OVERLAY_SIDE`] pixels a side, is refused.
pub fn set_region(app: &mut VectorcraftApp, image: u64, rect: Rel, overlay: Option<&[u8]>, editable: bool) -> Result<(), String> {
    let st = app.session.active().ok_or("vectorcraft:region needs a document open")?;
    let id = NodeId(image);
    if !st.doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Image(_))) {
        return Err(format!("vectorcraft:region: {image} is not an image of the active document"));
    }
    let overlay = match overlay {
        None => None,
        Some(bytes) => {
            let picture = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .map_err(|e| format!("vectorcraft:region: the overlay is not a PNG ({e})"))?;
            if picture.width() > MAX_OVERLAY_SIDE || picture.height() > MAX_OVERLAY_SIDE {
                return Err(format!("vectorcraft:region: the overlay is at most {MAX_OVERLAY_SIDE} pixels a side"));
            }
            let rgba = picture.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            Some((Arc::new(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())), None))
        }
    };
    app.ui.host_region.region = Some(HostRegion { doc: st.uid, image: id, rect: clamped(rect), editable, overlay });
    Ok(())
}

/// Take the page's region away.
pub fn clear_region(app: &mut VectorcraftApp) {
    app.ui.host_region.region = None;
}

/// The region's image's placement (pixel space into the document) and size, while the region's
/// document is active and its image is still there.
fn placement(app: &VectorcraftApp) -> Option<(&HostRegion, Affine, f64, f64)> {
    let region = app.ui.host_region.region.as_ref()?;
    let st = app.session.active().filter(|st| st.uid == region.doc)?;
    let NodeKind::Image(im) = &st.doc.node(region.image)?.kind else { return None };
    Some((region, im.xf, f64::from(im.width.max(1)), f64::from(im.height.max(1))))
}

/// A point of the image in relative units, on screen.
fn on_screen(xf: &Xf, place: Affine, w: f64, h: f64, x: f64, y: f64) -> Pos2 {
    xf.to_screen(place * Point::new(x * w, y * h))
}

/// The box's corners on screen, clockwise from the top-left (none while the region isn't drawn).
pub fn box_on_screen(app: &VectorcraftApp, xf: &Xf) -> Option<[Pos2; 4]> {
    let (region, place, w, h) = placement(app)?;
    let [l, t, r, b] = region.rect;
    Some([on_screen(xf, place, w, h, l, t), on_screen(xf, place, w, h, r, t), on_screen(xf, place, w, h, r, b), on_screen(xf, place, w, h, l, b)])
}

/// The screen rectangle clipping the region: the clip path of the clip group holding the image,
/// when it is a rectangle square to the screen (a Placement Options frame in an unturned view);
/// none otherwise, and then it is drawn whole.
fn clip_rect(app: &VectorcraftApp, xf: &Xf) -> Option<egui::Rect> {
    let region = app.ui.host_region.region.as_ref()?;
    let st = app.session.active()?;
    let parent = st.doc.node(st.doc.parent_of(region.image)?)?;
    if !matches!(parent.kind, NodeKind::Group { clip: true, .. }) || xf.rot.rem_euclid(std::f64::consts::FRAC_PI_2).abs() > 1e-9 {
        return None;
    }
    let path = parent.children()?.first()?;
    let b = path.geometric_bounds()?;
    let q = xf.quad(b);
    Some(egui::Rect::from_points(&q))
}

/// The region over the art, before the selection's handles: the page's overlay on the image's
/// quad, then the box (with corner handles when it can be edited).
pub(crate) fn draw(app: &mut VectorcraftApp, ui: &Ui, painter: &egui::Painter, xf: &Xf) {
    let Some((region, place, w, h)) = placement(app) else { return };
    let quad = [
        on_screen(xf, place, w, h, 0.0, 0.0),
        on_screen(xf, place, w, h, 1.0, 0.0),
        on_screen(xf, place, w, h, 1.0, 1.0),
        on_screen(xf, place, w, h, 0.0, 1.0),
    ];
    let editable = region.editable;
    let corners = box_on_screen(app, xf);
    let painter = match clip_rect(app, xf) {
        Some(clip) => painter.with_clip_rect(clip.intersect(painter.clip_rect())),
        None => painter.clone(),
    };
    // The overlay's texture, made once from its pixels.
    let texture = app.ui.host_region.region.as_mut().and_then(|r| {
        let (pixels, texture) = r.overlay.as_mut()?;
        Some(texture.get_or_insert_with(|| ui.ctx().load_texture("host-region-overlay", (**pixels).clone(), egui::TextureOptions::LINEAR)).id())
    });
    if let Some(texture) = texture {
        let mut mesh = egui::Mesh::with_texture(texture);
        let uv = [egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)];
        for (p, uv) in quad.iter().zip(uv) {
            mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv, color: Color32::WHITE });
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        painter.add(egui::Shape::mesh(mesh));
    }
    let Some(corners) = corners else { return };
    painter.add(egui::Shape::convex_polygon(corners.to_vec(), GREEN.gamma_multiply(0.12), Stroke::new(2.0, GREEN)));
    if editable {
        for c in corners {
            painter.rect_filled(egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0)), 1.0, GREEN);
        }
    }
}

/// Whether the region's box can be edited now: it is, the image or the clip group holding it is
/// what is selected, and the tool is the Selection or Direct Selection tool.
fn editing(app: &VectorcraftApp) -> bool {
    let Some(region) = app.ui.host_region.region.as_ref().filter(|r| r.editable) else { return false };
    let Some(st) = app.session.active().filter(|st| st.uid == region.doc) else { return false };
    let parent = st.doc.parent_of(region.image);
    let selected = st.selection.objects.len() == 1 && st.selection.objects.first().is_some_and(|id| *id == region.image || Some(*id) == parent);
    selected && matches!(app.session.tool_id(), "selection" | "directSelection")
}

/// Whether `p` is inside the convex quad `q`.
fn inside(q: &[Pos2; 4], p: Pos2) -> bool {
    let side = |a: Pos2, b: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let s: Vec<f32> = (0..4).map(|i| side(q[i], q[(i + 1) % 4])).collect();
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

/// A screen point in the image's relative units.
fn relative(app: &VectorcraftApp, xf: &Xf, p: Pos2) -> Option<Point> {
    let (_, place, w, h) = placement(app)?;
    let px = place.inverse() * xf.to_doc(p);
    Some(Point::new(px.x / w, px.y / h))
}

/// The box moved by its inside, or a corner of it moved, to `to` (relative units).
pub fn dragged(held_start: Rel, from: Point, to: Point, corner: Option<usize>) -> Rel {
    let [l, t, r, b] = held_start;
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    match corner {
        None => {
            let (w, h) = (r - l, b - t);
            let left = (l + dx).clamp(0.0, 1.0 - w);
            let top = (t + dy).clamp(0.0, 1.0 - h);
            [left, top, left + w, top + h]
        }
        Some(c) => {
            let (mut l, mut t, mut r, mut b) = (l, t, r, b);
            match c {
                0 => (l, t) = ((l + dx).clamp(0.0, r - MIN_SHARE), (t + dy).clamp(0.0, b - MIN_SHARE)),
                1 => (r, t) = ((r + dx).clamp(l + MIN_SHARE, 1.0), (t + dy).clamp(0.0, b - MIN_SHARE)),
                2 => (r, b) = ((r + dx).clamp(l + MIN_SHARE, 1.0), (b + dy).clamp(t + MIN_SHARE, 1.0)),
                _ => (l, b) = ((l + dx).clamp(0.0, r - MIN_SHARE), (b + dy).clamp(t + MIN_SHARE, 1.0)),
            }
            [l, t, r, b]
        }
    }
}

/// The box's handles and inside, taken before the tool while it can be edited ([`editing`]), on a
/// layer of its own over the canvas as the Contextual Task Bar takes its clicks: a drag moves or
/// resizes it, live; letting go tells the page; Escape during a drag puts it back and tells nothing.
pub(crate) fn interact(app: &mut VectorcraftApp, ui: &Ui, xf: &Xf) {
    let held: Option<Held> = ui.data(|d| d.get_temp(held_id()));
    if !editing(app) {
        if held.is_some() {
            ui.data_mut(|d| d.remove::<Held>(held_id()));
        }
        return;
    }
    let Some(corners) = box_on_screen(app, xf) else { return };
    let area = egui::Rect::from_points(&corners).expand(GRIP).intersect(xf.rect);
    if !area.is_positive() {
        return;
    }
    let mut update: Option<Rel> = None;
    let mut release: Option<Rel> = None;
    let mut cancel = false;
    egui::Area::new(egui::Id::unique("host-region")).order(egui::Order::Middle).fixed_pos(area.min).show(ui.ctx(), |ui| {
        let (rect, resp) = ui.allocate_exact_size(area.size(), Sense::drag());
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let corner_at = |p: Pos2| corners.iter().position(|c| c.distance(p) <= GRIP);
        if let Some(p) = resp.hover_pos() {
            let icon = match corner_at(p) {
                Some(0 | 2) => egui::CursorIcon::ResizeNwSe,
                Some(_) => egui::CursorIcon::ResizeNeSw,
                None if inside(&corners, p) => egui::CursorIcon::Move,
                None => egui::CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(icon);
        }
        let _ = rect;
        if resp.drag_started()
            && let Some(p) = pointer
            && (corner_at(p).is_some() || inside(&corners, p))
            && let Some(from) = relative(app, xf, p)
            && let Some(region) = &app.ui.host_region.region
        {
            ui.data_mut(|d| d.insert_temp(held_id(), Held { start: region.rect, from, corner: corner_at(p) }));
        }
        let held: Option<Held> = ui.data(|d| d.get_temp(held_id()));
        if let Some(held) = held {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
                update = Some(held.start);
            } else if let Some(p) = pointer
                && let Some(to) = relative(app, xf, p)
            {
                let r = dragged(held.start, held.from, to, held.corner);
                if resp.drag_stopped() {
                    release = Some(r);
                } else {
                    update = Some(r);
                }
            }
        }
    });
    if let Some(r) = update.or(release)
        && let Some(region) = app.ui.host_region.region.as_mut()
    {
        region.rect = r;
    }
    if cancel || release.is_some() {
        ui.data_mut(|d| d.remove::<Held>(held_id()));
    }
    if let (Some(r), Some(region)) = (release, app.ui.host_region.region.as_ref()) {
        app.ui.host_region.changed.push((region.image, r));
    }
}
