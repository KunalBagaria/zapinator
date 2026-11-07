//! The one window: a transparent, click-through panel hugging the top of the
//! built-in screen, centred on the notch. All motion happens in its layers.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSView, NSWindowAnimationBehavior,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_graphics::CGShieldingWindowLevel;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::CALayer;

use crate::config;
use crate::geometry::NotchGeometry;

/// The panel's frame: anchored to the top edge, centred on the notch, big
/// enough for the Lair plus the bolt and the shake.
pub fn panel_frame(g: &NotchGeometry) -> NSRect {
    let width = g.notch.size.width + 2.0 * config::WINDOW_PAD_X;
    let height = g.notch.size.height + config::WINDOW_PAD_BOTTOM;
    NSRect::new(
        NSPoint::new(g.notch_center_x() - width / 2.0, g.screen_top() - height),
        NSSize::new(width, height),
    )
}

/// Builds the (hidden) panel with a layer-hosting content view. Returns the
/// panel and its root layer.
pub fn create_panel(
    mtm: MainThreadMarker,
    frame: NSRect,
) -> (Retained<NSPanel>, Retained<CALayer>) {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        style,
        NSBackingStoreType::Buffered,
        false,
    );

    // SAFETY: turning release-on-close off is what makes the later `close()`
    // (when the screens change) safe while we still hold a `Retained`: AppKit
    // won't release the panel behind our back.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setLevel(CGShieldingWindowLevel() as isize);
    panel.setIgnoresMouseEvents(true);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setHidesOnDeactivate(false);
    panel.setAnimationBehavior(NSWindowAnimationBehavior::None);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );

    // Layer-hosting view: set the layer first, then ask for layers, so AppKit
    // leaves the tree entirely to us.
    let root = CALayer::new();
    let view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), frame.size),
    );
    view.setLayer(Some(&root));
    view.setWantsLayer(true);
    panel.setContentView(Some(&view));
    place(&panel, frame);

    (panel, root)
}

/// Moves the panel, then checks AppKit didn't nudge it below the menu bar.
pub fn place(panel: &NSPanel, frame: NSRect) {
    panel.setFrame_display(frame, false);
    let actual = panel.frame();
    if (actual.origin.y - frame.origin.y).abs() > 0.5
        || (actual.origin.x - frame.origin.x).abs() > 0.5
    {
        log::warn!(
            "panel frame was constrained: wanted ({:.1}, {:.1}), got ({:.1}, {:.1})",
            frame.origin.x,
            frame.origin.y,
            actual.origin.x,
            actual.origin.y
        );
    }
}
