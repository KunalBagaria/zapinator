//! The notch outline as `CGPath`s.
//!
//! Every path has exactly the same element sequence (move, line, 4 corner
//! curves, a curved bottom edge, ...), so Core Animation can morph between
//! any two of them point by point, overshoot included.
//!
//! Coordinates are layer coordinates: origin bottom-left, y up, `top` is the
//! top edge of the screen.

use std::ptr;

use objc2_core_foundation::{CFRetained, CGFloat};
use objc2_core_graphics::CGMutablePath;

use crate::config;

/// Cubic Bézier handle length for a quarter circle.
const KAPPA: CGFloat = 0.552_284_75;

pub type Path = CFRetained<CGMutablePath>;

/// The dimensions of one notch state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub width: f64,
    pub height: f64,
    /// Concave flare where the shape meets the screen edge. 0 = square.
    pub ear_radius: f64,
    pub corner_radius: f64,
    /// How far the middle of the bottom edge droops.
    pub sag: f64,
}

impl Spec {
    pub fn with_sag(self, sag: f64) -> Self {
        Self { sag, ..self }
    }
}

/// The three named states, all centred on the notch.
#[derive(Clone, Copy, Debug)]
pub struct States {
    pub center_x: f64,
    pub top: f64,
    pub collapsed: Spec,
    pub ears: Spec,
    pub lair: Spec,
}

impl States {
    pub fn new(center_x: f64, top: f64, notch_width: f64, notch_height: f64) -> Self {
        let collapsed = Spec {
            width: notch_width,
            height: notch_height,
            ear_radius: 0.0,
            corner_radius: config::NOTCH_CORNER_RADIUS,
            sag: 0.0,
        };
        let ears = Spec {
            width: notch_width + 2.0 * config::EARS_EXTRA_WIDTH,
            height: notch_height + config::EARS_EXTRA_HEIGHT,
            ear_radius: config::EAR_RADIUS,
            corner_radius: config::EXPANDED_CORNER_RADIUS,
            sag: 0.0,
        };
        let lair = Spec {
            // Never narrower than the ears, so the ear content always fits.
            width: config::LAIR_WIDTH.max(ears.width),
            height: notch_height + config::LAIR_EXTRA_HEIGHT,
            ..ears
        };
        Self {
            center_x,
            top,
            collapsed,
            ears,
            lair,
        }
    }

    /// Closed outline, for filling (or masking).
    pub fn fill(&self, spec: Spec) -> Path {
        outline(self.center_x, self.top, spec, true)
    }

    /// The same outline minus the top edge, for the inner stroke: a line
    /// along the very top of the screen would look like a glitch.
    pub fn stroke(&self, spec: Spec) -> Path {
        outline(self.center_x, self.top, spec, false)
    }
}

/// Small safe wrapper over `CGMutablePath`'s construction functions.
pub struct PathBuilder(Path);

impl PathBuilder {
    pub fn new() -> Self {
        Self(CGMutablePath::new())
    }

    pub fn move_to(&self, (x, y): (f64, f64)) {
        // SAFETY: a null transform pointer means "identity", which CG allows.
        unsafe { CGMutablePath::move_to_point(Some(&self.0), ptr::null(), x, y) }
    }

    pub fn line_to(&self, (x, y): (f64, f64)) {
        // SAFETY: as above, null transform.
        unsafe { CGMutablePath::add_line_to_point(Some(&self.0), ptr::null(), x, y) }
    }

    pub fn curve_to(&self, c1: (f64, f64), c2: (f64, f64), to: (f64, f64)) {
        // SAFETY: as above, null transform.
        unsafe {
            CGMutablePath::add_curve_to_point(
                Some(&self.0),
                ptr::null(),
                c1.0,
                c1.1,
                c2.0,
                c2.1,
                to.0,
                to.1,
            )
        }
    }

    pub fn close(&self) {
        CGMutablePath::close_subpath(Some(&self.0));
    }

    pub fn finish(self) -> Path {
        self.0
    }
}

/// The notch outline. Walks clockwise from the left ear tip when `closed`,
/// or from the right ear tip (skipping the top edge) when not.
fn outline(cx: f64, top: f64, s: Spec, closed: bool) -> Path {
    let hw = s.width / 2.0;
    let (l, r) = (cx - hw, cx + hw);
    let bottom = top - s.height;
    let e = s.ear_radius.max(0.0);
    // Corners can never be bigger than the space available for them.
    let c = s.corner_radius.clamp(0.0, hw.min(s.height - e).max(0.0));
    let ke = KAPPA * e;
    let kc = KAPPA * c;
    // A cubic whose two handles sit at depth d dips 3/4·d at its midpoint.
    let sag_handle = bottom - s.sag * 4.0 / 3.0;
    let third = (s.width - 2.0 * c) / 3.0;

    let p = PathBuilder::new();
    if closed {
        p.move_to((l - e, top));
        p.line_to((r + e, top));
    } else {
        p.move_to((r + e, top));
    }
    // Right ear: concave quarter circle centred at (r + e, top - e).
    p.curve_to((r + e - ke, top), (r, top - e + ke), (r, top - e));
    p.line_to((r, bottom + c));
    // Bottom-right corner: convex quarter circle centred at (r - c, bottom + c).
    p.curve_to((r, bottom + c - kc), (r - c + kc, bottom), (r - c, bottom));
    // Bottom edge, sagging in the middle when asked to.
    p.curve_to(
        (r - c - third, sag_handle),
        (l + c + third, sag_handle),
        (l + c, bottom),
    );
    // Bottom-left corner.
    p.curve_to((l + c - kc, bottom), (l, bottom + c - kc), (l, bottom + c));
    p.line_to((l, top - e));
    // Left ear.
    p.curve_to((l, top - e + ke), (l - e + ke, top), (l - e, top));
    if closed {
        p.close();
    }
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_graphics::CGPath;

    fn states() -> States {
        States::new(250.0, 172.0, 179.0, 32.0)
    }

    #[test]
    fn collapsed_is_exactly_the_notch() {
        let st = states();
        let b = CGPath::bounding_box(Some(&st.fill(st.collapsed)));
        assert!((b.size.width - 179.0).abs() < 1e-9);
        assert!((b.size.height - 32.0).abs() < 1e-9);
        assert!((b.origin.x - (250.0 - 89.5)).abs() < 1e-9);
        assert!((b.origin.y - 140.0).abs() < 1e-9);
    }

    #[test]
    fn lair_and_ears_are_flush_with_the_top() {
        let st = states();
        for spec in [st.ears, st.lair] {
            let b = CGPath::bounding_box(Some(&st.fill(spec)));
            assert!((b.origin.y + b.size.height - 172.0).abs() < 1e-9);
            assert!((b.size.width - (spec.width + 2.0 * spec.ear_radius)).abs() < 1e-9);
        }
        assert!(st.lair.width >= st.ears.width);
        assert_eq!(st.lair.height, 32.0 + config::LAIR_EXTRA_HEIGHT);
    }

    #[test]
    fn sag_droops_the_bottom() {
        let st = states();
        let flat = CGPath::bounding_box(Some(&st.fill(st.ears)));
        let sagged = CGPath::bounding_box(Some(&st.fill(st.ears.with_sag(4.0))));
        // Bounding boxes include control points, so check it went down.
        assert!(sagged.origin.y < flat.origin.y - 4.0);
    }
}
