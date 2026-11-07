//! Finding the notch at runtime.
//!
//! Everything here is in AppKit global screen coordinates: points, origin at
//! the bottom-left of the primary display, y growing upwards.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// Everything we know about the built-in screen and the hole in it.
#[derive(Clone, Copy, Debug)]
pub struct NotchGeometry {
    /// `screen.frame()` of the built-in display (not `visibleFrame`).
    pub screen_frame: NSRect,
    pub backing_scale: f64,
    /// The notch itself. Its top edge is flush with the top of the screen.
    pub notch: NSRect,
}

impl NotchGeometry {
    /// Geometry of the built-in (notched) screen, or `None` in clamshell mode,
    /// on an external-only setup, or on a Mac without a notch.
    pub fn current(mtm: MainThreadMarker) -> Option<Self> {
        let screen = builtin_screen(mtm)?;
        let geometry = Self::from_screen(&screen);
        if geometry.is_none() {
            log::warn!("built-in screen reports a safe-area inset but no usable notch gap");
        }
        geometry
    }

    pub fn from_screen(screen: &NSScreen) -> Option<Self> {
        let screen_frame = screen.frame();
        let aux_left = screen.auxiliaryTopLeftArea();
        let aux_right = screen.auxiliaryTopRightArea();

        let height = screen.safeAreaInsets().top;
        let width = screen_frame.size.width - aux_left.size.width - aux_right.size.width;
        if height <= 0.0 || width <= 0.0 || aux_left.size.width <= 0.0 {
            return None;
        }

        // "Notch x = right edge of auxiliaryTopLeftArea". The top-left area
        // always starts at the screen's left edge, so its right edge is
        // `frame.x + width`. Writing it that way stays correct whether AppKit
        // hands back the aux rect in global or screen-local coordinates (the
        // two differ when the built-in panel is not the primary display).
        let x = screen_frame.origin.x + aux_left.size.width;
        let y = screen_frame.origin.y + screen_frame.size.height - height;

        Some(Self {
            screen_frame,
            backing_scale: screen.backingScaleFactor(),
            notch: NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)),
        })
    }

    pub fn notch_center_x(&self) -> f64 {
        self.notch.origin.x + self.notch.size.width / 2.0
    }

    /// The top edge of the screen (and of the notch).
    pub fn screen_top(&self) -> f64 {
        self.screen_frame.origin.y + self.screen_frame.size.height
    }
}

/// The built-in display is the one with a non-zero top safe-area inset.
pub fn builtin_screen(mtm: MainThreadMarker) -> Option<Retained<NSScreen>> {
    NSScreen::screens(mtm)
        .to_vec()
        .into_iter()
        .find(|s| s.safeAreaInsets().top > 0.0)
}

/// `--print-geometry`: dump every screen and the computed notch rect.
pub fn print_report(mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm).to_vec();
    println!("NOTCH-ZAP-INATOR geometry report");
    println!("screens: {}", screens.len());

    for (i, screen) in screens.iter().enumerate() {
        let inset = screen.safeAreaInsets();
        let tag = if inset.top > 0.0 {
            "  <- built-in (notched)"
        } else {
            ""
        };
        println!();
        println!("[{i}] {}{tag}", screen.localizedName());
        println!("    frame            {}", fmt_rect(screen.frame()));
        println!("    visibleFrame     {}", fmt_rect(screen.visibleFrame()));
        println!(
            "    safeAreaInsets   top={} left={} bottom={} right={}",
            fmt_num(inset.top),
            fmt_num(inset.left),
            fmt_num(inset.bottom),
            fmt_num(inset.right)
        );
        println!(
            "    auxTopLeftArea   {}",
            fmt_rect(screen.auxiliaryTopLeftArea())
        );
        println!(
            "    auxTopRightArea  {}",
            fmt_rect(screen.auxiliaryTopRightArea())
        );
        println!(
            "    backingScale     {}",
            fmt_num(screen.backingScaleFactor())
        );
    }

    println!();
    match NotchGeometry::current(mtm) {
        Some(g) => {
            let n = g.notch;
            let from_left = n.origin.x - g.screen_frame.origin.x;
            println!("notch (global AppKit coords, origin bottom-left):");
            println!("    {}", fmt_rect(n));
            println!(
                "    centre x={}  top edge y={}  bottom edge y={}",
                fmt_num(g.notch_center_x()),
                fmt_num(g.screen_top()),
                fmt_num(n.origin.y)
            );
            println!(
                "    = {}pt to {}pt from the screen's left edge, {}pt tall ({}x{} px)",
                fmt_num(from_left),
                fmt_num(from_left + n.size.width),
                fmt_num(n.size.height),
                fmt_num(n.size.width * g.backing_scale),
                fmt_num(n.size.height * g.backing_scale)
            );
        }
        None => {
            println!("notch: none found (clamshell mode, external display only, or no notch)");
        }
    }
}

fn fmt_rect(r: NSRect) -> String {
    format!(
        "x={} y={} w={} h={}",
        fmt_num(r.origin.x),
        fmt_num(r.origin.y),
        fmt_num(r.size.width),
        fmt_num(r.size.height)
    )
}

/// Whole numbers print without a trailing `.0`; fractions keep two places.
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}
