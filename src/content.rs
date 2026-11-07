//! What appears inside the expanded notch: the ear icon, the percentage
//! counter, the battery capsule and the typewriter villain line.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSFont, NSFontDescriptorSystemDesignRounded, NSFontWeightBold, NSFontWeightHeavy, NSImage,
    NSImageSymbolConfiguration,
};
use objc2_core_foundation::{CFRetained, CFType, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_foundation::NSString;
use objc2_quartz_core::{
    CALayer, CATextLayer, CATextLayerAlignmentMode, kCAAlignmentLeft, kCAAlignmentRight,
};

use crate::config;
use crate::shape::States;

/// sRGB colour from `0xRRGGBB`.
pub fn color(hex: u32, alpha: f64) -> CFRetained<CGColor> {
    let c = |shift: u32| f64::from((hex >> shift) & 0xFF) / 255.0;
    CGColor::new_srgb(c(16), c(8), c(0), alpha)
}

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

/// `CATextLayer.font` is typed as a bare `CFTypeRef`.
fn font_as_cf(font: &NSFont) -> &CFType {
    // SAFETY: NSFont is toll-free bridged with CTFont, so an NSFont pointer is
    // a valid CFTypeRef; we only lend it for the duration of the call.
    unsafe { &*(font as *const NSFont).cast::<CFType>() }
}

fn set_text(layer: &CATextLayer, text: &str) {
    let s = NSString::from_str(text);
    // SAFETY: CATextLayer.string accepts an NSString (or attributed string).
    unsafe { layer.setString(Some(&s)) };
}

fn alignment(right: bool) -> &'static CATextLayerAlignmentMode {
    // SAFETY: reading immutable framework constants.
    unsafe {
        if right {
            kCAAlignmentRight
        } else {
            kCAAlignmentLeft
        }
    }
}

fn text_layer(font: &NSFont, scale: f64, right: bool) -> Retained<CATextLayer> {
    let t = CATextLayer::new();
    // SAFETY: a valid CTFont-compatible object, see `font_as_cf`.
    unsafe { t.setFont(Some(font_as_cf(font))) };
    t.setFontSize(font.pointSize());
    t.setForegroundColor(Some(&color(config::WHITE, 1.0)));
    t.setAlignmentMode(alignment(right));
    t.setContentsScale(scale);
    t.setWrapped(false);
    t
}

/// SF Pro Rounded Heavy, falling back to plain SF Pro Heavy.
fn rounded_heavy(size: f64) -> Retained<NSFont> {
    // SAFETY: reading immutable AppKit constants.
    let (weight, design) = unsafe { (NSFontWeightHeavy, NSFontDescriptorSystemDesignRounded) };
    let base = NSFont::systemFontOfSize_weight(size, weight);
    base.fontDescriptor()
        .fontDescriptorWithDesign(design)
        .and_then(|d| NSFont::fontWithDescriptor_size(&d, size))
        .unwrap_or(base)
}

fn mono_bold(size: f64) -> Retained<NSFont> {
    // SAFETY: reading an immutable AppKit constant.
    NSFont::monospacedSystemFontOfSize_weight(size, unsafe { NSFontWeightBold })
}

/// A rasterised SF Symbol, ready to be a layer's contents.
struct Symbol {
    contents: Retained<AnyObject>,
    size: CGSize,
}

fn symbol(name: &str, scale: f64) -> Option<Symbol> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        None,
    )?;
    // SAFETY: reading an immutable AppKit constant.
    let weight = unsafe { NSFontWeightBold };
    let cfg = NSImageSymbolConfiguration::configurationWithPointSize_weight(
        config::ICON_POINT_SIZE,
        weight,
    );
    let image = image.imageWithSymbolConfiguration(&cfg)?;
    let size = image.size();
    Some(Symbol {
        contents: image.layerContentsForContentsScale(scale),
        size,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Bolt,
    BoltSlash,
}

/// A villain line laid out for the typewriter.
pub struct Typed {
    chars: Vec<char>,
    shown: usize,
    left: f64,
    advance: f64,
}

impl Typed {
    pub fn char_count(&self) -> usize {
        self.chars.len()
    }
}

/// All content layers, positioned for one notch geometry.
pub struct Content {
    // Left ear.
    pub icon_group: Retained<CALayer>,
    icon: Retained<CALayer>,
    icon_mask: Retained<CALayer>,
    bolt: Option<Symbol>,
    bolt_slash: Option<Symbol>,
    icon_center: CGPoint,
    // Right ear.
    pub battery_group: Retained<CALayer>,
    percent: Retained<CATextLayer>,
    capsule: Retained<CALayer>,
    capsule_fill: Retained<CALayer>,
    nub: Retained<CALayer>,
    pub shimmer: Retained<CALayer>,
    // Lair panel (or ear space).
    pub line_group: Retained<CALayer>,
    line: Retained<CATextLayer>,
    cursor: Retained<CALayer>,
    line_center: CGPoint,
    line_max_width: f64,
    ear_line_left: f64,
    ear_line_width: f64,
    strip_center_y: f64,
}

impl Content {
    pub fn new(states: &States, notch_width: f64, scale: f64) -> Self {
        let cx = states.center_x;
        let top = states.top;
        let strip_center_y = top - states.collapsed.height / 2.0;
        let ears_half = states.ears.width / 2.0;

        // ----- Left ear: icon -------------------------------------------------
        let icon_center = CGPoint::new(cx - ears_half + config::ICON_INSET, strip_center_y);
        let icon_group = CALayer::new();
        icon_group.setShadowColor(Some(&color(config::ELECTRIC_CYAN, 1.0)));
        icon_group.setShadowRadius(config::ICON_GLOW_RADIUS);
        icon_group.setShadowOffset(CGSize::new(0.0, 0.0));
        icon_group.setShadowOpacity(0.0);
        let icon = CALayer::new();
        let icon_mask = CALayer::new();
        icon_mask.setContentsScale(scale);
        // SAFETY: setting the mask of a layer we own to a fresh layer.
        unsafe { icon.setMask(Some(&icon_mask)) };
        icon_group.addSublayer(&icon);

        // ----- Right ear: percentage + capsule ---------------------------------
        let battery_group = CALayer::new();
        let outer_right = cx + ears_half - config::CAPSULE_INSET;
        let nub_w = 1.5;
        let cap_x = outer_right - nub_w - config::CAPSULE_WIDTH;
        let cap_y = strip_center_y - config::CAPSULE_HEIGHT / 2.0;

        let outline = color(config::WHITE, config::CAPSULE_OUTLINE_OPACITY);
        let capsule = CALayer::new();
        capsule.setFrame(rect(
            cap_x,
            cap_y,
            config::CAPSULE_WIDTH,
            config::CAPSULE_HEIGHT,
        ));
        capsule.setCornerRadius(3.5);
        capsule.setBorderWidth(1.0);
        capsule.setBorderColor(Some(&outline));
        capsule.setMasksToBounds(true);
        capsule.setContentsScale(scale);

        let capsule_fill = CALayer::new();
        capsule_fill.setAnchorPoint(CGPoint::new(0.0, 0.5));
        capsule_fill.setCornerRadius(1.5);
        capsule.addSublayer(&capsule_fill);

        let shimmer = CALayer::new();
        shimmer.setBackgroundColor(Some(&color(config::WHITE, 0.85)));
        shimmer.setFrame(rect(-8.0, 0.0, 5.0, config::CAPSULE_HEIGHT));
        shimmer.setOpacity(0.0);
        capsule.addSublayer(&shimmer);

        let nub = CALayer::new();
        nub.setFrame(rect(
            cap_x + config::CAPSULE_WIDTH + 0.5,
            strip_center_y - 2.0,
            nub_w,
            4.0,
        ));
        nub.setCornerRadius(0.75);
        nub.setBackgroundColor(Some(&outline));

        let pct_font = rounded_heavy(config::PERCENT_FONT_SIZE);
        let percent = text_layer(&pct_font, scale, true);
        let pct_w = 52.0;
        let pct_right = cap_x - config::PERCENT_GAP;
        percent.setFrame(baseline_frame(
            &pct_font,
            pct_right - pct_w,
            strip_center_y,
            pct_w,
        ));

        battery_group.addSublayer(&percent);
        battery_group.addSublayer(&capsule);
        battery_group.addSublayer(&nub);

        // ----- Lair panel: typewriter line -----------------------------------
        let line_group = CALayer::new();
        let line = text_layer(&mono_bold(config::LINE_FONT_SIZE), scale, false);
        let cursor = CALayer::new();
        cursor.setBackgroundColor(Some(&color(config::ELECTRIC_CYAN, 1.0)));
        line_group.addSublayer(&line);
        line_group.addSublayer(&cursor);
        let line_center = CGPoint::new(
            cx,
            top - states.lair.height + config::LAIR_EXTRA_HEIGHT / 2.0,
        );

        // Ear space for DEFEAT's optional line: between the icon and the notch.
        let ear_line_left = icon_center.x + config::ICON_POINT_SIZE / 2.0 + 8.0;
        let ear_line_width = (cx - notch_width / 2.0 - 6.0) - ear_line_left;

        for l in [&*icon_group, &battery_group, &line_group] {
            l.setOpacity(0.0);
        }

        Self {
            icon_group,
            icon,
            icon_mask,
            bolt: symbol("bolt.fill", scale),
            bolt_slash: symbol("bolt.slash.fill", scale),
            icon_center,
            battery_group,
            percent,
            capsule,
            capsule_fill,
            nub,
            shimmer,
            line_group,
            line,
            cursor,
            line_center,
            line_max_width: states.lair.width - 2.0 * config::LINE_SIDE_MARGIN,
            ear_line_left,
            ear_line_width,
            strip_center_y,
        }
    }

    /// Adds every content layer to `parent`.
    pub fn attach(&self, parent: &CALayer) {
        parent.addSublayer(&self.icon_group);
        parent.addSublayer(&self.battery_group);
        parent.addSublayer(&self.line_group);
    }

    /// Back to invisible and empty (call with implicit actions disabled).
    pub fn reset(&self) {
        for l in [&*self.icon_group, &self.battery_group, &self.line_group] {
            l.removeAllAnimations();
            l.setOpacity(0.0);
        }
        self.icon_group.setShadowOpacity(0.0);
        self.shimmer.removeAllAnimations();
        self.shimmer.setOpacity(0.0);
        set_text(&self.line, "");
        self.cursor.setHidden(true);
    }

    pub fn set_icon(&self, icon: Icon, hex: u32) {
        let sym = match icon {
            Icon::Bolt => &self.bolt,
            Icon::BoltSlash => &self.bolt_slash,
        };
        let Some(sym) = sym else {
            log::warn!("SF Symbol for {icon:?} unavailable");
            return;
        };
        let (w, h) = (sym.size.width, sym.size.height);
        let frame = rect(
            self.icon_center.x - w / 2.0,
            self.icon_center.y - h / 2.0,
            w,
            h,
        );
        self.icon_group.setFrame(frame);
        self.icon.setFrame(rect(0.0, 0.0, w, h));
        self.icon_mask.setFrame(rect(0.0, 0.0, w, h));
        // SAFETY: an NSImage-derived contents object, which CALayer accepts.
        unsafe { self.icon_mask.setContents(Some(&sym.contents)) };
        self.icon.setBackgroundColor(Some(&color(hex, 1.0)));
        self.icon_group.setShadowColor(Some(&color(hex, 1.0)));
    }

    pub fn set_percent(&self, value: u8, hex: u32) {
        set_text(&self.percent, &format!("{value}%"));
        self.percent.setForegroundColor(Some(&color(hex, 1.0)));
    }

    /// Fill the capsule to `level` (0..=1).
    pub fn set_capsule(&self, level: f64, hex: u32) {
        let inner_w = config::CAPSULE_WIDTH - 4.0;
        let inner_h = config::CAPSULE_HEIGHT - 4.0;
        let w = (inner_w * level.clamp(0.0, 1.0)).max(if level > 0.0 { 1.5 } else { 0.0 });
        self.capsule_fill.setBounds(rect(0.0, 0.0, w, inner_h));
        self.capsule_fill
            .setPosition(CGPoint::new(2.0, config::CAPSULE_HEIGHT / 2.0));
        self.capsule_fill.setBackgroundColor(Some(&color(hex, 1.0)));
    }

    /// Recolour the capsule outline and nub (grey for DEFEAT).
    pub fn set_capsule_outline(&self, hex: u32) {
        let c = color(hex, config::CAPSULE_OUTLINE_OPACITY);
        self.capsule.setBorderColor(Some(&c));
        self.nub.setBackgroundColor(Some(&c));
    }

    /// Lays a line out centred in the Lair panel, ready to type. Picks 13pt,
    /// or 11pt if it is too long, and truncates as a last resort (never wraps).
    pub fn prepare_line(&self, text: &str) -> Typed {
        let chars: Vec<char> = text.chars().collect();
        let big = mono_bold(config::LINE_FONT_SIZE);
        let fits_big = chars.len() <= config::MAX_LINE_CHARS
            && advance(&big) * chars.len() as f64 <= self.line_max_width;
        let font = if fits_big {
            big
        } else {
            mono_bold(config::LINE_FONT_SIZE_SMALL)
        };
        let adv = advance(&font);
        let max_chars = (self.line_max_width / adv).floor() as usize;
        let chars = truncate(chars, max_chars);
        if !fits_big {
            log::debug!("line {text:?} is long; using {}pt", font.pointSize());
        }
        let width = adv * chars.len() as f64;
        let left = self.line_center.x - width / 2.0;
        self.layout_line(&font, left, self.line_center.y, width, adv);
        Typed {
            chars,
            shown: usize::MAX,
            left,
            advance: adv,
        }
    }

    /// DEFEAT's line goes in the left ear, if it fits there at 11pt at all.
    pub fn prepare_ear_line(&self, text: &str) -> Option<Typed> {
        let font = mono_bold(config::LINE_FONT_SIZE_SMALL);
        let adv = advance(&font);
        let chars: Vec<char> = text.chars().collect();
        let width = adv * chars.len() as f64;
        if width > self.ear_line_width {
            log::debug!(
                "unplug line {text:?} needs {width:.0}pt but the ear has {:.0}pt; dropping it",
                self.ear_line_width
            );
            return None;
        }
        self.layout_line(&font, self.ear_line_left, self.strip_center_y, width, adv);
        Some(Typed {
            chars,
            shown: usize::MAX,
            left: self.ear_line_left,
            advance: adv,
        })
    }

    fn layout_line(&self, font: &NSFont, left: f64, center_y: f64, width: f64, adv: f64) {
        // SAFETY: valid CTFont-compatible object, see `font_as_cf`.
        unsafe { self.line.setFont(Some(font_as_cf(font))) };
        self.line.setFontSize(font.pointSize());
        // A few points of slack so CATextLayer never decides to truncate.
        self.line
            .setFrame(baseline_frame(font, left, center_y, width + 8.0));
        let cap = font.capHeight();
        let baseline = center_y - cap / 2.0;
        self.cursor
            .setBounds(rect(0.0, 0.0, (adv * 0.85).round(), (cap + 4.0).round()));
        self.cursor.setAnchorPoint(CGPoint::new(0.0, 0.0));
        self.cursor.setPosition(CGPoint::new(left, baseline - 2.0));
        set_text(&self.line, "");
    }

    /// Shows the first `n` characters and parks the cursor after them.
    /// Returns `true` if a new character appeared.
    pub fn type_to(&self, typed: &mut Typed, n: usize) -> bool {
        let n = n.min(typed.char_count());
        if n == typed.shown {
            return false;
        }
        let advanced = typed.shown != usize::MAX && n > typed.shown;
        typed.shown = n;
        let s: String = typed.chars[..n].iter().collect();
        set_text(&self.line, &s);
        let x = typed.left + typed.advance * n as f64 + 1.0;
        self.cursor
            .setPosition(CGPoint::new(x, self.cursor.position().y));
        advanced
    }

    pub fn show_cursor(&self, visible: bool) {
        self.cursor.setHidden(!visible);
    }

    /// Where the right edge of the right-ear capsule's travel is, for the shimmer.
    pub fn shimmer_span(&self) -> (f64, f64) {
        (-6.0, config::CAPSULE_WIDTH + 6.0)
    }
}

/// Monospaced advance width.
fn advance(font: &NSFont) -> f64 {
    font.maximumAdvancement().width
}

fn truncate(mut chars: Vec<char>, max: usize) -> Vec<char> {
    if chars.len() > max && max > 0 {
        log::warn!("line too long even at small size; truncating");
        chars.truncate(max - 1);
        chars.push('…');
    }
    chars
}

/// A text layer frame whose single line has its cap height centred on
/// `center_y`. CATextLayer draws from the top of its bounds, ascender first.
fn baseline_frame(font: &NSFont, x: f64, center_y: f64, width: f64) -> CGRect {
    let baseline = center_y - font.capHeight() / 2.0;
    let height = (font.ascender() - font.descender()).ceil();
    rect(x, baseline + font.ascender() - height, width, height)
}
