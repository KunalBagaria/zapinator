//! Choreography.
//!
//! - [`Stage`] owns the panel and every layer.
//! - [`Director`] plays one [`Show`] at a time. While a show runs, a single
//!   60 Hz `NSTimer` drives it (cues, flicker, counter, typewriter); when it
//!   ends the timer is invalidated and the panel ordered out, so idle costs
//!   nothing. Starting a new show cancels the current one first.
//! - The scripts ([`Zap`], [`Defeat`], [`Victory`], [`Calm`]) are plain
//!   timelines: each tick they're told the previous and current time and fire
//!   whatever falls in between.
//!
//! `--slow` sets the root layer's `speed`, which slows every Core Animation
//! animation, and scales the script clock by the same factor.

use std::cell::RefCell;
use std::f64::consts::PI;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::time::Instant;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPanel, NSWorkspace};
use objc2_core_foundation::{CFType, CGPoint, CGRect};
use objc2_foundation::{
    NSActivityOptions, NSArray, NSNumber, NSObjectProtocol, NSProcessInfo, NSRunLoop,
    NSRunLoopCommonModes, NSString, NSTimer,
};
use objc2_quartz_core::{
    CABasicAnimation, CAKeyframeAnimation, CALayer, CAMediaTiming, CAMediaTimingFunction,
    CAShapeLayer, CASpringAnimation, CATransaction,
};
use rand::rngs::ThreadRng;
use rand::seq::IndexedRandom;
use rand::{Rng, RngExt};

use crate::config::{self, Spring};
use crate::content::{Content, Icon, Typed, color, rect};
use crate::geometry::NotchGeometry;
use crate::lightning::{self, Pt};
use crate::shape::{Path, Spec, States};
use crate::sound::{Sfx, Speaker};
use crate::window;

/// What to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// Plugged in.
    Zap { percent: u8 },
    /// Unplugged.
    Defeat { percent: u8 },
    /// Reached 100% while plugged in.
    Victory,
    /// `--debug-shape`.
    DebugShape,
}

// ===========================================================================
// Core Animation helpers
// ===========================================================================

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// CF objects (paths, colours) are Objective-C objects too.
fn obj(cf: &CFType) -> &AnyObject {
    cf.as_ref()
}

fn num(v: f64) -> Retained<NSNumber> {
    NSNumber::new_f64(v)
}

fn curve(c: [f32; 4]) -> Retained<CAMediaTimingFunction> {
    CAMediaTimingFunction::functionWithControlPoints(c[0], c[1], c[2], c[3])
}

const EASE_OUT: [f32; 4] = [0.2, 0.8, 0.3, 1.0];
const EASE_IN: [f32; 4] = [0.45, 0.0, 0.9, 0.6];
const EASE_IN_OUT: [f32; 4] = [0.42, 0.0, 0.58, 1.0];

/// Runs `f` with implicit animations off: our layers are standalone, so any
/// property change would otherwise animate for 0.25s.
fn no_implicit<R>(f: impl FnOnce() -> R) -> R {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    let r = f();
    CATransaction::commit();
    r
}

fn basic(
    key: &str,
    from: &AnyObject,
    to: &AnyObject,
    duration: f64,
    c: [f32; 4],
) -> Retained<CABasicAnimation> {
    let a = CABasicAnimation::animationWithKeyPath(Some(&ns(key)));
    // SAFETY: from/to are NSNumber/CGPath/CGColor values matching `key`.
    unsafe {
        a.setFromValue(Some(from));
        a.setToValue(Some(to));
    }
    a.setDuration(duration);
    a.setTimingFunction(Some(&curve(c)));
    a
}

fn spring(key: &str, from: &AnyObject, to: &AnyObject, s: Spring) -> Retained<CASpringAnimation> {
    let a = CASpringAnimation::animationWithKeyPath(Some(&ns(key)));
    a.setDamping(s.damping);
    a.setStiffness(s.stiffness);
    a.setMass(s.mass);
    // SAFETY: as in `basic`.
    unsafe {
        a.setFromValue(Some(from));
        a.setToValue(Some(to));
    }
    a.setDuration(a.settlingDuration());
    a
}

/// Fades `layer` from wherever it currently appears to `to`.
///
/// "Currently appears" is the presentation layer, i.e. the last committed
/// state: a model change made earlier in the same transaction isn't seen.
fn fade_to(layer: &CALayer, to: f32, duration: f64) {
    // SAFETY: presentationLayer returns a read-only snapshot (or nil if the
    // layer was never committed); we only read its opacity.
    let from = unsafe { layer.presentationLayer() }.map_or(layer.opacity(), |p| p.opacity());
    layer.setOpacity(to);
    let a = basic(
        "opacity",
        &num(f64::from(from)),
        &num(f64::from(to)),
        duration,
        EASE_OUT,
    );
    layer.addAnimation_forKey(&a, Some(&ns("opacity")));
}

/// Endless gentle back-and-forth of a numeric property.
fn pulse(layer: &CALayer, key: &str, from: f64, to: f64, period: f64) {
    let a = basic(key, &num(from), &num(to), period / 2.0, EASE_IN_OUT);
    a.setAutoreverses(true);
    a.setRepeatCount(f32::INFINITY);
    layer.addAnimation_forKey(&a, Some(&ns(key)));
}

enum Motion {
    Spring(Spring),
    Ease(f64, [f32; 4]),
}

// ===========================================================================
// Stage
// ===========================================================================

/// The panel and its layer tree, built for one screen geometry.
///
/// ```text
/// root (hosting layer; `speed` for --slow)
/// └─ shaker (shaken sideways)
///    ├─ body (masked by the notch shape)
///    │  ├─ black, flash
///    │  ├─ icon / battery / line content
///    │  └─ stroke (inner glow line; the mask clips its outer half)
///    ├─ arcs, sparks ×2, bolt glow, bolt core
///    └─ particles (spark bursts)
/// ```
pub struct Stage {
    panel: Retained<NSPanel>,
    frame: CGRect,
    root: Retained<CALayer>,
    shaker: Retained<CALayer>,
    body: Retained<CALayer>,
    mask: Retained<CAShapeLayer>,
    black: Retained<CALayer>,
    flash: Retained<CALayer>,
    stroke: Retained<CAShapeLayer>,
    arcs: Retained<CAShapeLayer>,
    sparks_cyan: Retained<CAShapeLayer>,
    sparks_white: Retained<CAShapeLayer>,
    bolt_glow: Retained<CAShapeLayer>,
    bolt_core: Retained<CAShapeLayer>,
    particles: Retained<CALayer>,
    content: Content,
    states: States,
    scale: f64,
}

fn shape_layer(bounds: CGRect, scale: f64) -> Retained<CAShapeLayer> {
    let l = CAShapeLayer::new();
    l.setFrame(bounds);
    l.setContentsScale(scale);
    l.setFillColor(None);
    l
}

fn glow(layer: &CALayer, hex: u32, radius: f64, opacity: f32) {
    layer.setShadowColor(Some(&color(hex, 1.0)));
    layer.setShadowRadius(radius);
    layer.setShadowOpacity(opacity);
    layer.setShadowOffset(objc2_core_foundation::CGSize::new(0.0, 0.0));
}

impl Stage {
    fn build(mtm: MainThreadMarker, g: &NotchGeometry) -> Self {
        let frame = window::panel_frame(g);
        let (panel, root) = window::create_panel(mtm, frame);
        let (w, h) = (frame.size.width, frame.size.height);
        let bounds = rect(0.0, 0.0, w, h);
        let scale = g.backing_scale;

        let notch_w = g.notch.size.width + config::NOTCH_TRIM_WIDTH;
        let notch_h = g.notch.size.height + config::NOTCH_TRIM_HEIGHT;
        let states = States::new(w / 2.0, h, notch_w, notch_h);

        let shaker = CALayer::new();
        shaker.setFrame(bounds);
        root.addSublayer(&shaker);

        // Body: everything that lives inside the notch shape.
        let body = CALayer::new();
        body.setFrame(bounds);
        let mask = shape_layer(bounds, scale);
        mask.setFillColor(Some(&color(config::NOTCH_BLACK, 1.0)));
        // SAFETY: masking a layer we own with a fresh layer.
        unsafe { body.setMask(Some(&mask)) };
        shaker.addSublayer(&body);

        let black = CALayer::new();
        black.setFrame(bounds);
        black.setBackgroundColor(Some(&color(config::NOTCH_BLACK, 1.0)));
        body.addSublayer(&black);

        let flash = CALayer::new();
        flash.setFrame(bounds);
        flash.setBackgroundColor(Some(&color(config::WHITE, 1.0)));
        flash.setOpacity(0.0);
        body.addSublayer(&flash);

        let content = Content::new(&states, notch_w, scale);
        content.attach(&body);

        // Centred on the outline and 2pt wide; the mask hides the outer half,
        // leaving a 1pt inner stroke.
        let stroke = shape_layer(bounds, scale);
        stroke.setLineWidth(2.0);
        body.addSublayer(&stroke);

        // Effects that may spill outside the shape.
        let arcs = shape_layer(bounds, scale);
        arcs.setStrokeColor(Some(&color(config::ELECTRIC_CYAN, 1.0)));
        arcs.setLineWidth(1.0);
        glow(&arcs, config::ELECTRIC_CYAN, 3.0, 1.0);

        let sparks_cyan = shape_layer(bounds, scale);
        sparks_cyan.setStrokeColor(Some(&color(config::ELECTRIC_CYAN, 1.0)));
        sparks_cyan.setLineWidth(1.0);
        glow(&sparks_cyan, config::ELECTRIC_CYAN, 2.5, 1.0);
        let sparks_white = shape_layer(bounds, scale);
        sparks_white.setStrokeColor(Some(&color(config::WHITE, 1.0)));
        sparks_white.setLineWidth(1.0);
        glow(&sparks_white, config::ELECTRIC_CYAN, 2.5, 1.0);

        let bolt_glow = shape_layer(bounds, scale);
        bolt_glow.setStrokeColor(Some(&color(config::ELECTRIC_CYAN, 1.0)));
        bolt_glow.setLineWidth(config::BOLT_GLOW_WIDTH);
        glow(
            &bolt_glow,
            config::ELECTRIC_CYAN,
            config::BOLT_GLOW_RADIUS,
            1.0,
        );
        let bolt_core = shape_layer(bounds, scale);
        bolt_core.setStrokeColor(Some(&color(config::WHITE, 1.0)));
        bolt_core.setLineWidth(config::BOLT_CORE_WIDTH);
        for l in [&bolt_glow, &bolt_core] {
            // SAFETY: reading immutable framework constants.
            unsafe {
                l.setLineJoin(objc2_quartz_core::kCALineJoinRound);
                l.setLineCap(objc2_quartz_core::kCALineCapRound);
            }
        }

        let particles = CALayer::new();
        particles.setFrame(bounds);

        let effects: [&CALayer; 6] = [
            &arcs,
            &sparks_cyan,
            &sparks_white,
            &bolt_glow,
            &bolt_core,
            &particles,
        ];
        for l in effects {
            shaker.addSublayer(l);
        }

        log::debug!(
            "stage built: panel {:.1}x{:.1} at ({:.1}, {:.1}), notch {notch_w}x{notch_h}",
            w,
            h,
            frame.origin.x,
            frame.origin.y
        );

        Self {
            panel,
            frame: CGRect::new(frame.origin, frame.size),
            root,
            shaker,
            body,
            mask,
            black,
            flash,
            stroke,
            arcs,
            sparks_cyan,
            sparks_white,
            bolt_glow,
            bolt_core,
            particles,
            content,
            states,
            scale,
        }
    }

    // ----- Lifecycle --------------------------------------------------------

    /// Everything back to Collapsed and invisible. Call inside `no_implicit`.
    fn reset(&self, speed: f32) {
        let collapsed = self.states.collapsed;
        for l in [
            &*self.root,
            &self.shaker,
            &self.body,
            &self.mask,
            &self.flash,
            &self.stroke,
            &self.arcs,
            &self.sparks_cyan,
            &self.sparks_white,
            &self.bolt_glow,
            &self.bolt_core,
        ] {
            l.removeAllAnimations();
        }
        self.root.setSpeed(speed);
        self.mask.setPath(Some(&self.states.fill(collapsed)));
        self.body.setOpacity(1.0);
        self.black
            .setBackgroundColor(Some(&color(config::NOTCH_BLACK, 1.0)));
        self.flash.setOpacity(0.0);
        self.stroke.setPath(Some(&self.states.stroke(collapsed)));
        self.stroke.setOpacity(0.0);
        self.stroke.setLineWidth(2.0);
        self.stroke
            .setStrokeColor(Some(&color(config::ELECTRIC_CYAN, 1.0)));
        for l in [
            &self.arcs,
            &self.sparks_cyan,
            &self.sparks_white,
            &self.bolt_glow,
            &self.bolt_core,
        ] {
            l.setPath(None);
            l.setOpacity(1.0);
        }
        self.arcs.setOpacity(config::ARC_OPACITY);
        // SAFETY: removes our own transient particle layers.
        unsafe { self.particles.setSublayers(None) };
        self.content.reset();
    }

    fn show(&self) {
        window::place(&self.panel, self.frame);
        self.panel.orderFrontRegardless();
    }

    fn hide(&self) {
        self.panel.orderOut(None);
    }

    // ----- Shape --------------------------------------------------------------

    /// Morphs the fill (mask) and the stroke together.
    fn morph(&self, from: Spec, to: Spec, motion: &Motion) {
        let pairs: [(&CAShapeLayer, Path, Path); 2] = [
            (&self.mask, self.states.fill(from), self.states.fill(to)),
            (
                &self.stroke,
                self.states.stroke(from),
                self.states.stroke(to),
            ),
        ];
        for (layer, a, b) in pairs {
            let anim: Retained<CABasicAnimation> = match *motion {
                Motion::Spring(s) => Retained::into_super(spring("path", obj(&a), obj(&b), s)),
                Motion::Ease(d, c) => basic("path", obj(&a), obj(&b), d, c),
            };
            layer.setPath(Some(&b));
            layer.addAnimation_forKey(&anim, Some(&ns("path")));
        }
    }

    /// Keyframed morph through several shapes (DEFEAT's droop).
    fn morph_keyframes(&self, specs: &[Spec], times: &[f64], duration: f64) {
        let fills: Vec<Path> = specs.iter().map(|s| self.states.fill(*s)).collect();
        let strokes: Vec<Path> = specs.iter().map(|s| self.states.stroke(*s)).collect();
        let key_times: Vec<Retained<NSNumber>> = times.iter().map(|t| num(*t)).collect();
        for (layer, paths) in [(&self.mask, &fills), (&self.stroke, &strokes)] {
            let values: Vec<&AnyObject> = paths.iter().map(|p| obj(p)).collect();
            let a = CAKeyframeAnimation::animationWithKeyPath(Some(&ns("path")));
            // SAFETY: an array of CGPaths for the "path" key path.
            unsafe { a.setValues(Some(&NSArray::from_slice(&values))) };
            a.setKeyTimes(Some(&NSArray::from_retained_slice(&key_times)));
            a.setDuration(duration);
            a.setTimingFunction(Some(&curve(EASE_IN_OUT)));
            if let Some(last) = paths.last() {
                layer.setPath(Some(last));
            }
            layer.addAnimation_forKey(&a, Some(&ns("path")));
        }
    }

    fn shake(&self) {
        let values: Vec<Retained<NSNumber>> =
            config::zap::SHAKE_OFFSETS.iter().map(|v| num(*v)).collect();
        let values: Vec<&AnyObject> = values.iter().map(|n| -> &AnyObject { n }).collect();
        let a = CAKeyframeAnimation::animationWithKeyPath(Some(&ns("transform.translation.x")));
        // SAFETY: NSNumbers for a scalar key path.
        unsafe { a.setValues(Some(&NSArray::from_slice(&values))) };
        a.setDuration(config::zap::SHAKE_DURATION);
        self.shaker.addAnimation_forKey(&a, Some(&ns("shake")));
    }

    fn stroke_in(&self, hex: u32, duration: f64) {
        self.stroke.setStrokeColor(Some(&color(hex, 1.0)));
        fade_to(&self.stroke, config::STROKE_OPACITY, duration);
    }

    fn stroke_pulse(&self) {
        pulse(
            &self.stroke,
            "opacity",
            f64::from(config::STROKE_OPACITY),
            f64::from(config::STROKE_PULSE_MIN),
            config::STROKE_PULSE_PERIOD,
        );
    }

    fn flash(&self) {
        let a = basic(
            "opacity",
            &num(f64::from(config::zap::FLASH_OPACITY)),
            &num(0.0),
            config::zap::FLASH_DURATION,
            EASE_IN,
        );
        self.flash.addAnimation_forKey(&a, Some(&ns("flash")));
    }

    // ----- Electricity --------------------------------------------------------

    fn notch_bottom(&self) -> f64 {
        self.states.top - self.states.collapsed.height
    }

    fn charge_sparks(&self, rng: &mut impl Rng) {
        let c = self.states.collapsed;
        let half = c.width / 2.0;
        let corner_in = c.corner_radius * 0.3;
        let corners = [
            (self.states.center_x - half + corner_in, self.notch_bottom()),
            (self.states.center_x + half - corner_in, self.notch_bottom()),
        ];
        let count = rng.random_range(config::SPARK_COUNT.0..=config::SPARK_COUNT.1);
        let (mut cyan, mut white) = (Vec::new(), Vec::new());
        for _ in 0..count {
            let base = *corners.choose(rng).expect("two corners");
            let at = (
                base.0 + rng.random_range(-3.0..3.0),
                base.1 - rng.random_range(0.5..3.5),
            );
            let s = lightning::spark(rng, at);
            if rng.random_bool(0.5) {
                cyan.push(s)
            } else {
                white.push(s)
            }
        }
        self.sparks_cyan.setPath(Some(&lightning::polyline_path(
            cyan.iter().map(Vec::as_slice),
        )));
        self.sparks_white.setPath(Some(&lightning::polyline_path(
            white.iter().map(Vec::as_slice),
        )));
    }

    fn clear_sparks(&self) {
        self.sparks_cyan.setPath(None);
        self.sparks_white.setPath(None);
    }

    fn set_bolt(&self, rng: &mut impl Rng) {
        // Start a touch inside the notch so the bolt grows out of it.
        let origin = (self.states.center_x, self.notch_bottom() + 2.0);
        let bolt = lightning::bolt(rng, origin, config::BOLT_LENGTH + 2.0);
        let path = lightning::polyline_path(bolt.lines());
        self.bolt_glow.setPath(Some(&path));
        self.bolt_core.setPath(Some(&path));
    }

    fn fade_bolt(&self, duration: f64) {
        fade_to(&self.bolt_glow, 0.0, duration);
        fade_to(&self.bolt_core, 0.0, duration);
    }

    fn set_arcs(&self, rng: &mut impl Rng) {
        let lair = self.states.lair;
        let half = lair.width / 2.0 - lair.corner_radius;
        let y = self.states.top - lair.height;
        let range = (self.states.center_x - half, self.states.center_x + half);
        let count = rng.random_range(config::ARC_COUNT.0..=config::ARC_COUNT.1);
        let arcs: Vec<Vec<Pt>> = (0..count)
            .map(|_| lightning::edge_arc(rng, range, y))
            .collect();
        self.arcs.setPath(Some(&lightning::polyline_path(
            arcs.iter().map(Vec::as_slice),
        )));
    }

    fn clear_arcs(&self) {
        fade_to(&self.arcs, 0.0, 0.12);
    }

    /// Little sparks flying out of `origin` in directions within `angles`
    /// (radians), each fading as it travels.
    fn burst(
        &self,
        rng: &mut impl Rng,
        origin: Pt,
        count: u32,
        angles: (f64, f64),
        reach: (f64, f64),
        duration: f64,
    ) {
        for _ in 0..count {
            let angle = rng.random_range(angles.0..angles.1);
            let dist = rng.random_range(reach.0..reach.1);
            let streak = lightning::streak(rng, angle);
            let l = CAShapeLayer::new();
            l.setContentsScale(self.scale);
            l.setFillColor(None);
            l.setStrokeColor(Some(&color(
                if rng.random_bool(0.7) {
                    config::ELECTRIC_CYAN
                } else {
                    config::WHITE
                },
                1.0,
            )));
            l.setLineWidth(1.2);
            glow(&l, config::ELECTRIC_CYAN, 3.0, 1.0);
            l.setPath(Some(&lightning::polyline_path([streak.as_slice()])));
            l.setBounds(rect(0.0, 0.0, 0.0, 0.0));
            let end = (origin.0 + angle.cos() * dist, origin.1 + angle.sin() * dist);
            l.setPosition(CGPoint::new(end.0, end.1));
            l.setOpacity(0.0);
            for (key, from, to) in [
                ("position.x", origin.0, end.0),
                ("position.y", origin.1, end.1),
            ] {
                l.addAnimation_forKey(
                    &basic(key, &num(from), &num(to), duration, EASE_OUT),
                    Some(&ns(key)),
                );
            }
            l.addAnimation_forKey(
                &basic("opacity", &num(1.0), &num(0.0), duration, EASE_IN),
                Some(&ns("opacity")),
            );
            self.particles.addSublayer(&l);
        }
    }

    // ----- Content --------------------------------------------------------------

    fn content_in(&self, duration: f64, with_line: bool) {
        fade_to(&self.content.icon_group, 1.0, duration);
        fade_to(&self.content.battery_group, 1.0, duration);
        if with_line {
            fade_to(&self.content.line_group, 1.0, duration);
        }
    }

    fn content_out(&self, duration: f64) {
        for l in [
            &*self.content.icon_group,
            &self.content.battery_group,
            &self.content.line_group,
        ] {
            fade_to(l, 0.0, duration);
        }
        self.content
            .icon_group
            .removeAnimationForKey(&ns("shadowOpacity"));
    }

    fn icon_glow_pulse(&self) {
        pulse(
            &self.content.icon_group,
            "shadowOpacity",
            0.25,
            1.0,
            config::ICON_GLOW_PERIOD,
        );
    }

    fn shimmer(&self, duration: f64) {
        let (from, to) = self.content.shimmer_span();
        let s = &self.content.shimmer;
        s.addAnimation_forKey(
            &basic("position.x", &num(from), &num(to), duration, EASE_IN_OUT),
            Some(&ns("x")),
        );
        let fade = CAKeyframeAnimation::animationWithKeyPath(Some(&ns("opacity")));
        let vals = [num(0.0), num(1.0), num(1.0), num(0.0)];
        let vals: Vec<&AnyObject> = vals.iter().map(|n| -> &AnyObject { n }).collect();
        // SAFETY: NSNumbers for a scalar key path.
        unsafe { fade.setValues(Some(&NSArray::from_slice(&vals))) };
        fade.setDuration(duration);
        s.addAnimation_forKey(&fade, Some(&ns("opacity")));
    }
}

// ===========================================================================
// Scripts
// ===========================================================================

/// The previous and current script time. A cue at `t` fires on the one tick
/// whose `(prev, now]` interval contains it, so nothing is skipped or doubled
/// even if a tick runs late.
#[derive(Clone, Copy, Debug)]
struct Clock {
    prev: f64,
    now: f64,
}

impl Clock {
    fn at(self, t: f64) -> bool {
        self.prev < t && t <= self.now
    }

    fn within(self, start: f64, end: f64) -> bool {
        start <= self.now && self.now < end
    }
}

/// A periodic regenerator ("every 45ms").
#[derive(Default)]
struct Every {
    next: f64,
}

impl Every {
    fn due(&mut self, now: f64, period: f64) -> bool {
        if now >= self.next {
            self.next = now + period;
            true
        } else {
            false
        }
    }
}

/// Typewriter + blinking cursor shared by the scripts.
struct Typewriter {
    typed: Typed,
    start: f64,
}

impl Typewriter {
    fn tick(&mut self, content: &Content, now: f64, sp: &Speaker) {
        let elapsed = (now - self.start).max(0.0);
        let n = (elapsed * config::TYPE_CHARS_PER_SEC).floor() as usize;
        if content.type_to(&mut self.typed, n) {
            sp.play(Sfx::Type);
        }
        let done_at = self.typed.char_count() as f64 / config::TYPE_CHARS_PER_SEC;
        // Solid while typing, blinking once finished.
        let visible =
            elapsed < done_at || ((elapsed - done_at) / config::CURSOR_BLINK) as u64 % 2 == 1;
        content.show_cursor(visible);
    }
}

fn ease_out_cubic(p: f64) -> f64 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// ZAP: charge-up, strike, BEHOLD, monologue, retreat.
struct Zap {
    percent: u8,
    line: String,
    typewriter: Option<Typewriter>,
    sparks: Every,
    bolt: Every,
    arcs: Every,
}

impl Zap {
    fn tick(&mut self, s: &Stage, c: Clock, rng: &mut ThreadRng, sp: &Speaker) -> bool {
        use config::zap::*;
        let st = s.states;

        if c.at(0.0) {
            sp.play(Sfx::ChargeUp);
        }
        if c.within(0.0, STRIKE) && self.sparks.due(c.now, config::SPARK_FLICKER) {
            s.charge_sparks(rng);
        }
        if c.at(STRIKE) {
            s.clear_sparks();
            s.flash();
            sp.play(Sfx::Strike);
        }
        if c.within(STRIKE, BOLT_FADE) && self.bolt.due(c.now, config::BOLT_FLICKER) {
            s.set_bolt(rng);
        }
        if c.at(BOLT_FADE) {
            s.fade_bolt(STRIKE_END - BOLT_FADE);
        }
        if c.at(BEHOLD) {
            s.morph(st.collapsed, st.lair, &Motion::Spring(config::OPEN_SPRING));
            s.shake();
            s.stroke_in(config::ELECTRIC_CYAN, MONOLOGUE - BEHOLD);
            sp.play(Sfx::Behold);
        }
        // The strike flash happens inside the still-collapsed notch, i.e. in
        // the hole, so echo it once the ears have burst out where it shows.
        if c.at(BEHOLD + FLASH_ECHO) {
            s.flash();
        }
        if c.at(MONOLOGUE) {
            let k = &s.content;
            k.set_icon(Icon::Bolt, config::ELECTRIC_CYAN);
            k.set_percent(0, config::WHITE);
            k.set_capsule(0.0, config::TOXIC_GREEN);
            k.set_capsule_outline(config::WHITE);
            let typed = k.prepare_line(&self.line);
            self.typewriter = Some(Typewriter {
                typed,
                start: MONOLOGUE + 0.05,
            });
            s.content_in(0.15, true);
            s.icon_glow_pulse();
            s.stroke_pulse();
        }
        if c.within(MONOLOGUE, RETREAT) {
            let p = ease_out_cubic((c.now - MONOLOGUE) / config::COUNTER_DURATION);
            let shown = (p * f64::from(self.percent)).round();
            s.content.set_percent(shown as u8, config::WHITE);
            s.content.set_capsule(shown / 100.0, config::TOXIC_GREEN);
            if let Some(tw) = &mut self.typewriter {
                tw.tick(&s.content, c.now, sp);
            }
            if c.at(ARCS_START) {
                sp.hum(RETREAT + CONTENT_FADE - ARCS_START);
            }
            if c.now >= ARCS_START && self.arcs.due(c.now, config::ARC_FLICKER) {
                s.set_arcs(rng);
            }
        }
        if c.at(RETREAT) {
            s.content_out(CONTENT_FADE);
            s.clear_arcs();
            s.stroke.removeAllAnimations();
            fade_to(&s.stroke, 0.0, CONTENT_FADE + 0.1);
        }
        if c.at(RETREAT + CONTENT_FADE) {
            s.morph(st.lair, st.collapsed, &Motion::Spring(config::CLOSE_SPRING));
            sp.play(Sfx::PowerDown);
        }
        if c.at(PUFF) {
            sp.play(Sfx::Puff);
            let origin = (st.center_x, s.notch_bottom() - 1.0);
            s.burst(
                rng,
                origin,
                config::PUFF_SPARKS,
                (-PI + 0.25, -0.25),
                (6.0, 14.0),
                PUFF_DURATION,
            );
        }
        c.now >= END
    }
}

/// DEFEAT: a sad little Ears with a droop.
struct Defeat {
    percent: u8,
    line: Option<String>,
}

impl Defeat {
    fn tick(&mut self, s: &Stage, c: Clock, sp: &Speaker) -> bool {
        use config::defeat::*;
        let st = s.states;
        if c.at(0.0) {
            sp.play(Sfx::Sad);
            s.morph(
                st.collapsed,
                st.ears,
                &Motion::Ease(EXPAND_DURATION, EASE_OUT),
            );
            s.stroke_in(config::DEFEAT_GREY, EXPAND_DURATION);
        }
        if c.at(CONTENT_IN) {
            let k = &s.content;
            k.set_icon(Icon::BoltSlash, config::DEFEAT_GREY);
            k.set_percent(self.percent, config::DEFEAT_GREY);
            k.set_capsule(f64::from(self.percent) / 100.0, config::DEFEAT_GREY);
            k.set_capsule_outline(config::DEFEAT_GREY);
            let mut with_line = false;
            if let Some(line) = &self.line
                && let Some(mut typed) = k.prepare_ear_line(line)
            {
                k.type_to(&mut typed, usize::MAX);
                with_line = true;
            }
            s.content_in(0.18, with_line);
        }
        if c.at(DROOP) {
            let sad = st.ears.with_sag(config::DROOP_SAG);
            s.morph_keyframes(
                &[st.ears, sad, sad, st.ears],
                &[0.0, 0.3, 0.6, 1.0],
                DROOP_DURATION,
            );
        }
        if c.at(RETREAT) {
            s.content_out(CONTENT_FADE);
        }
        if c.at(COLLAPSE) {
            s.morph(
                st.ears,
                st.collapsed,
                &Motion::Ease(COLLAPSE_DURATION, EASE_IN),
            );
            sp.play(Sfx::Close);
            fade_to(&s.stroke, 0.0, COLLAPSE_DURATION);
        }
        c.now >= END
    }
}

/// VICTORY: green Ears, sparks out of the ears, a shimmer, maybe a line.
struct Victory {
    line: Option<String>,
    typewriter: Option<Typewriter>,
}

impl Victory {
    fn retreat_at(&self) -> f64 {
        use config::victory::*;
        if self.line.is_some() {
            RETREAT_LONG
        } else {
            RETREAT_SHORT
        }
    }

    fn tick(&mut self, s: &Stage, c: Clock, rng: &mut ThreadRng, sp: &Speaker) -> bool {
        use config::victory::*;
        let st = s.states;
        let retreat = self.retreat_at();
        let open = if self.line.is_some() {
            st.lair
        } else {
            st.ears
        };

        if c.at(0.0) {
            s.morph(st.collapsed, st.ears, &Motion::Spring(config::EARS_SPRING));
            s.stroke_in(config::ELECTRIC_CYAN, 0.25);
            sp.play(Sfx::Open);
        }
        if c.at(CONTENT_IN) {
            let k = &s.content;
            k.set_icon(Icon::Bolt, config::TOXIC_GREEN);
            k.set_percent(100, config::TOXIC_GREEN);
            k.set_capsule(1.0, config::TOXIC_GREEN);
            k.set_capsule_outline(config::WHITE);
            s.content_in(0.15, false);
            sp.play(Sfx::Fanfare);
            s.icon_glow_pulse();
            s.stroke_pulse();
        }
        if c.at(SPARKS) {
            let count = rng.random_range(config::VICTORY_SPARKS.0..=config::VICTORY_SPARKS.1);
            let half = st.ears.width / 2.0 - 4.0;
            let y = st.top - st.ears.height / 2.0;
            let left = count / 2;
            s.burst(
                rng,
                (st.center_x - half, y),
                left,
                (PI - 0.7, PI + 0.7),
                (22.0, 42.0),
                SPARKS_DURATION,
            );
            s.burst(
                rng,
                (st.center_x + half, y),
                count - left,
                (-0.7, 0.7),
                (22.0, 42.0),
                SPARKS_DURATION,
            );
        }
        if c.at(SHIMMER) {
            s.shimmer(SHIMMER_DURATION);
        }
        if let Some(line) = &self.line {
            if c.at(LAIR) {
                s.morph(st.ears, st.lair, &Motion::Spring(config::OPEN_SPRING));
                sp.play(Sfx::Open);
            }
            if c.at(TYPE_START) {
                let typed = s.content.prepare_line(line);
                self.typewriter = Some(Typewriter {
                    typed,
                    start: TYPE_START,
                });
                fade_to(&s.content.line_group, 1.0, 0.1);
            }
        }
        if c.within(TYPE_START, retreat)
            && let Some(tw) = &mut self.typewriter
        {
            tw.tick(&s.content, c.now, sp);
        }
        if c.at(retreat) {
            s.content_out(CONTENT_FADE);
            s.stroke.removeAllAnimations();
            fade_to(&s.stroke, 0.0, CONTENT_FADE + 0.1);
        }
        if c.at(retreat + CONTENT_FADE) {
            s.morph(open, st.collapsed, &Motion::Spring(config::CLOSE_SPRING));
            sp.play(Sfx::Close);
        }
        c.now >= retreat + END_AFTER_RETREAT
    }
}

/// Reduce Motion: a still bolt, 0.2s cross-fade into the shape, everything
/// shown at once, hold, fade. No shake, overshoot, flicker or typing.
struct Calm {
    show: Show,
    line: Option<String>,
    bolt_drawn: bool,
}

impl Calm {
    fn tick(&mut self, s: &Stage, c: Clock, rng: &mut ThreadRng, sp: &Speaker) -> bool {
        use config::calm::*;
        let st = s.states;
        let (spec, icon, tint, percent, bolt) = match self.show {
            Show::Zap { percent } => (st.lair, Icon::Bolt, config::ELECTRIC_CYAN, percent, true),
            Show::Defeat { percent } => (
                st.ears,
                Icon::BoltSlash,
                config::DEFEAT_GREY,
                percent,
                false,
            ),
            Show::Victory if self.line.is_some() => {
                (st.lair, Icon::Bolt, config::TOXIC_GREEN, 100, false)
            }
            _ => (st.ears, Icon::Bolt, config::TOXIC_GREEN, 100, false),
        };
        let text_color = match self.show {
            Show::Zap { .. } => config::WHITE,
            _ => tint,
        };

        if c.at(0.0) {
            sp.play(match self.show {
                Show::Zap { .. } => Sfx::Strike,
                Show::Defeat { .. } => Sfx::Sad,
                _ => Sfx::Fanfare,
            });
            s.mask.setPath(Some(&s.states.fill(spec)));
            s.stroke.setPath(Some(&s.states.stroke(spec)));
            s.stroke.setStrokeColor(Some(&color(tint, 1.0)));
            s.stroke.setOpacity(config::STROKE_OPACITY);
            // Explicit 0 -> 1: `fade_to` reads the last *committed* opacity,
            // which `reset` just left at 1, so it would fade 1 -> 1.
            s.body.setOpacity(1.0);
            s.body.addAnimation_forKey(
                &basic("opacity", &num(0.0), &num(1.0), FADE_IN, EASE_OUT),
                Some(&ns("opacity")),
            );
            if bolt && !self.bolt_drawn {
                self.bolt_drawn = true;
                s.set_bolt(rng);
            }
            let k = &s.content;
            k.set_icon(icon, tint);
            k.set_percent(percent, text_color);
            let fill = if matches!(self.show, Show::Defeat { .. }) {
                config::DEFEAT_GREY
            } else {
                config::TOXIC_GREEN
            };
            k.set_capsule(f64::from(percent) / 100.0, fill);
            k.set_capsule_outline(if matches!(self.show, Show::Defeat { .. }) {
                config::DEFEAT_GREY
            } else {
                config::WHITE
            });
            k.icon_group.setShadowOpacity(if bolt { 0.7 } else { 0.0 });
            let mut with_line = false;
            if let Some(line) = &self.line {
                let typed = if spec == st.lair {
                    Some(k.prepare_line(line))
                } else {
                    k.prepare_ear_line(line)
                };
                if let Some(mut typed) = typed {
                    k.type_to(&mut typed, usize::MAX);
                    with_line = true;
                }
            }
            k.show_cursor(false);
            s.content_in(FADE_IN, with_line);
        }
        if bolt && c.at(BOLT_HOLD) {
            s.fade_bolt(FADE_IN);
        }
        if c.at(HOLD) {
            fade_to(&s.body, 0.0, FADE_OUT);
        }
        c.now >= HOLD + FADE_OUT + 0.05
    }
}

/// `--debug-shape`: the Lair in translucent red, with the reported notch
/// rect outlined, held still.
struct DebugShape;

impl DebugShape {
    fn tick(&mut self, s: &Stage, c: Clock) -> bool {
        if c.at(0.0) {
            let st = s.states;
            s.mask.setPath(Some(&st.fill(st.lair)));
            s.black
                .setBackgroundColor(Some(&color(config::DEBUG_RED, 0.6)));
            s.stroke.setPath(Some(&st.fill(st.collapsed)));
            s.stroke
                .setStrokeColor(Some(&color(config::DEBUG_OUTLINE, 1.0)));
            s.stroke.setLineWidth(1.0);
            s.stroke.setOpacity(1.0);
        }
        c.now >= config::DEBUG_SHAPE_HOLD
    }
}

enum Script {
    Zap(Zap),
    Defeat(Defeat),
    Victory(Victory),
    Calm(Calm),
    DebugShape(DebugShape),
}

impl Script {
    fn tick(&mut self, s: &Stage, c: Clock, rng: &mut ThreadRng, sp: &Speaker) -> bool {
        match self {
            Script::Zap(z) => z.tick(s, c, rng, sp),
            Script::Defeat(d) => d.tick(s, c, sp),
            Script::Victory(v) => v.tick(s, c, rng, sp),
            Script::Calm(m) => m.tick(s, c, rng, sp),
            Script::DebugShape(d) => d.tick(s, c),
        }
    }
}

// ===========================================================================
// Line picking
// ===========================================================================

/// Picks villain lines at random, never the same one twice in a row.
#[derive(Default)]
pub struct LinePicker {
    last: Option<&'static str>,
}

impl LinePicker {
    pub fn pick(
        &mut self,
        rng: &mut impl Rng,
        pool: &'static [&'static str],
        percent: u8,
    ) -> Option<String> {
        let candidates: Vec<&'static str> = pool
            .iter()
            .copied()
            .filter(|l| Some(*l) != self.last)
            .collect();
        let line = *candidates.choose(rng).or_else(|| pool.first())?;
        self.last = Some(line);
        Some(line.replace("{pct}", &percent.to_string()).to_uppercase())
    }
}

// ===========================================================================
// Director
// ===========================================================================

struct Run {
    timer: Retained<NSTimer>,
    started: Instant,
    prev: f64,
    script: Script,
    speaker: Speaker,
    /// Keeps App Nap from throttling the ticker while the show runs.
    activity: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

/// Plays one show at a time. Lives in an `Rc<RefCell<_>>` on the main thread.
pub struct Director {
    mtm: MainThreadMarker,
    stage: Option<Stage>,
    run: Option<Run>,
    lines: LinePicker,
    rng: ThreadRng,
    speed: f32,
    on_finished: Option<Rc<dyn Fn()>>,
    this: Weak<RefCell<Director>>,
}

impl Director {
    pub fn new(mtm: MainThreadMarker, speed: f32) -> Rc<RefCell<Self>> {
        Rc::new_cyclic(|this| {
            RefCell::new(Self {
                mtm,
                stage: None,
                run: None,
                lines: LinePicker::default(),
                rng: rand::rng(),
                speed,
                on_finished: None,
                this: this.clone(),
            })
        })
    }

    /// Called from the ticker, outside any borrow of the director, whenever a
    /// show ends on its own (not when it's cancelled).
    pub fn on_finished(&mut self, f: impl Fn() + 'static) {
        self.on_finished = Some(Rc::new(f));
    }

    /// Cancels whatever is playing and starts `show`. Returns `false` if there
    /// is no notched built-in screen to play on.
    pub fn play(&mut self, show: Show) -> bool {
        self.stop();
        if self.stage.is_none() {
            let Some(g) = NotchGeometry::current(self.mtm) else {
                log::info!(
                    "no notched built-in screen (clamshell or external only); skipping {show:?}"
                );
                return false;
            };
            self.stage = Some(Stage::build(self.mtm, &g));
        }
        let reduce_motion = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
            || std::env::var_os("NOTCH_ZAP_REDUCE_MOTION").is_some_and(|v| v == "1");
        let script = self.script_for(show, reduce_motion);
        log::info!(
            "playing {show:?}{}",
            if reduce_motion {
                " (reduce motion)"
            } else {
                ""
            }
        );

        let Some(stage) = &self.stage else {
            return false;
        };
        no_implicit(|| stage.reset(self.speed));
        stage.show();
        let speaker = if show == Show::DebugShape {
            Speaker::silent()
        } else {
            Speaker::open(self.speed)
        };
        let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep
                | NSActivityOptions::LatencyCritical,
            &ns("NOTCH-ZAP-INATOR animation"),
        );
        let timer = self.start_ticker();
        self.run = Some(Run {
            timer,
            started: Instant::now(),
            prev: -1.0,
            script,
            speaker,
            activity,
        });
        // Draw t = 0 right away rather than one tick late. Even if a script
        // were already done here, ending it is left to the ticker, so the
        // finished callback never runs inside the caller's borrow.
        self.advance();
        true
    }

    fn script_for(&mut self, show: Show, reduce_motion: bool) -> Script {
        let rng = &mut self.rng;
        let line = match show {
            Show::Zap { percent } => {
                let pool = if percent < config::LOW_BATTERY {
                    config::LINES_PLUG_LOW
                } else {
                    config::LINES_PLUG
                };
                self.lines.pick(rng, pool, percent)
            }
            Show::Defeat { percent } if config::SHOW_UNPLUG_LINE => {
                self.lines.pick(rng, config::LINES_UNPLUG, percent)
            }
            Show::Victory if config::SHOW_FULL_LINE => {
                self.lines.pick(rng, config::LINES_FULL, 100)
            }
            _ => None,
        };
        if let Some(l) = &line {
            log::debug!("line: {l}");
        }
        if reduce_motion && show != Show::DebugShape {
            return Script::Calm(Calm {
                show,
                line,
                bolt_drawn: false,
            });
        }
        match show {
            Show::Zap { percent } => Script::Zap(Zap {
                percent,
                line: line.unwrap_or_default(),
                typewriter: None,
                sparks: Every::default(),
                bolt: Every::default(),
                arcs: Every::default(),
            }),
            Show::Defeat { percent } => Script::Defeat(Defeat { percent, line }),
            Show::Victory => Script::Victory(Victory {
                line,
                typewriter: None,
            }),
            Show::DebugShape => Script::DebugShape(DebugShape),
        }
    }

    fn start_ticker(&self) -> Retained<NSTimer> {
        let this = self.this.clone();
        let block = RcBlock::new(move |timer: NonNull<NSTimer>| {
            let Some(director) = this.upgrade() else {
                // SAFETY: NSTimer passes itself, valid for the duration of
                // the callback.
                unsafe { timer.as_ref() }.invalidate();
                return;
            };
            let done = match director.try_borrow_mut() {
                Ok(mut d) => d.tick(),
                Err(_) => false,
            };
            if done {
                let callback = director.borrow().finished_callback();
                if let Some(f) = callback {
                    f();
                }
            }
        });
        // SAFETY: the timer is only ever scheduled on the main run loop (just
        // below), so the block (and the Rc/Weak it holds) stays on the main
        // thread even though the API asks for a "sendable" block.
        let timer = unsafe {
            NSTimer::timerWithTimeInterval_repeats_block(1.0 / config::TICK_HZ, true, &block)
        };
        // SAFETY: NSRunLoopCommonModes is an immutable framework constant, and
        // we're on the main thread adding to the main run loop.
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
        timer
    }

    /// Advances the running script and ends the show when it's done.
    /// Returns `true` when it just finished.
    fn tick(&mut self) -> bool {
        let done = self.advance();
        if done {
            self.end(true);
        }
        done
    }

    /// Draws the script at the current time. Returns `true` once it's done.
    fn advance(&mut self) -> bool {
        let (Some(run), Some(stage)) = (self.run.as_mut(), self.stage.as_ref()) else {
            return false;
        };
        let now = run.started.elapsed().as_secs_f64() * f64::from(self.speed);
        let clock = Clock {
            prev: run.prev,
            now,
        };
        run.prev = now;
        let rng = &mut self.rng;
        let (script, speaker) = (&mut run.script, &run.speaker);
        no_implicit(|| script.tick(stage, clock, rng, speaker))
    }

    fn finished_callback(&self) -> Option<Rc<dyn Fn()>> {
        self.on_finished.clone()
    }

    /// Cancels the current show (if any): timer invalidated, sound cut,
    /// layers reset, panel ordered out.
    pub fn stop(&mut self) {
        self.end(false);
    }

    /// Ends the current show. `graceful` lets its last sounds ring out.
    fn end(&mut self, graceful: bool) {
        if let Some(run) = self.run.take() {
            run.timer.invalidate();
            // SAFETY: the token came from `beginActivityWithOptions_reason`
            // and is ended exactly once (the run is gone after this).
            unsafe { NSProcessInfo::processInfo().endActivity(&run.activity) };
            if graceful {
                run.speaker.finish();
            }
            if let Some(stage) = &self.stage {
                no_implicit(|| stage.reset(self.speed));
                stage.hide();
            }
        }
    }

    /// The screen layout changed: drop the stage so the next show rebuilds it
    /// from fresh geometry.
    pub fn screens_changed(&mut self) {
        self.stop();
        if let Some(stage) = self.stage.take() {
            stage.panel.close();
        }
        log::debug!("screen parameters changed; geometry will be recomputed");
    }
}

impl Drop for Director {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn clock_fires_each_cue_once() {
        let ticks = [-1.0, 0.0, 0.016, 0.2, 0.35, 0.36, 1.0];
        let mut fired = 0;
        for w in ticks.windows(2) {
            let c = Clock {
                prev: w[0],
                now: w[1],
            };
            if c.at(0.35) {
                fired += 1;
            }
        }
        assert_eq!(fired, 1);
        assert!(
            Clock {
                prev: -1.0,
                now: 0.0
            }
            .at(0.0)
        );
    }

    #[test]
    fn every_regenerates_on_period() {
        let mut e = Every::default();
        let hits = (0..60).filter(|i| e.due(*i as f64 / 60.0, 0.045)).count();
        // 1s at 60 Hz with a 45ms period: one hit every third frame.
        assert_eq!(hits, 20);
    }

    #[test]
    fn never_the_same_line_twice() {
        let mut rng = StdRng::seed_from_u64(5);
        let mut p = LinePicker::default();
        let mut last = String::new();
        for _ in 0..500 {
            let l = p.pick(&mut rng, config::LINES_PLUG, 50).unwrap();
            assert_ne!(l, last);
            last = l;
        }
    }

    #[test]
    fn pct_is_substituted() {
        let mut rng = StdRng::seed_from_u64(5);
        let mut p = LinePicker::default();
        let pool: &'static [&'static str] = &["{pct}% HELLO"];
        assert_eq!(p.pick(&mut rng, pool, 9).unwrap(), "9% HELLO");
        // A one-line pool still works on the second pick.
        assert_eq!(p.pick(&mut rng, pool, 9).unwrap(), "9% HELLO");
    }

    #[test]
    fn every_stock_line_fits_at_13pt() {
        let all = [
            config::LINES_PLUG,
            config::LINES_PLUG_LOW,
            config::LINES_UNPLUG,
            config::LINES_FULL,
        ];
        for line in all.iter().flat_map(|p| p.iter()) {
            for pct in [1, 19, 99, 100] {
                let l = line.replace("{pct}", &pct.to_string());
                assert!(
                    l.chars().count() <= config::MAX_LINE_CHARS,
                    "{l:?} is too long"
                );
            }
        }
    }
}
