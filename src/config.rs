//! Every colour, size, timing, line and toggle in one place. Edit freely,
//! then `make install` again.

// ---------------------------------------------------------------------------
// Toggles
// ---------------------------------------------------------------------------

/// Play DEFEAT when the charger is pulled.
pub const SHOW_UNPLUG: bool = true;
/// During DEFEAT, show an unplug line in the ear space, but only if it fits
/// there (the ears are short). Lines that don't fit are dropped.
pub const SHOW_UNPLUG_LINE: bool = true;
/// Play VICTORY when the battery reaches 100% (at most once per plug-in).
pub const SHOW_FULL: bool = true;
/// During VICTORY, drop into the Lair and type a 100% line.
pub const SHOW_FULL_LINE: bool = true;
/// Synthesised sound effects (charge-up, zap, BEHOLD stinger, lab hum,
/// typewriter, power-down, sad slide, victory arpeggio). Needs the `sound`
/// cargo feature, which is on by default.
pub const SOUND_ENABLED: bool = true;
/// Master volume, 0.0 to 1.0.
#[cfg_attr(not(feature = "sound"), allow(dead_code))]
pub const SOUND_VOLUME: f32 = 0.30;
/// A click per typed character.
pub const SOUND_TYPEWRITER: bool = true;
/// Lab hum and crackle under the monologue.
pub const SOUND_HUM: bool = true;

// ---------------------------------------------------------------------------
// Colours (sRGB, 0xRRGGBB)
// ---------------------------------------------------------------------------

pub const ELECTRIC_CYAN: u32 = 0x64D2FF;
pub const TOXIC_GREEN: u32 = 0x30D158;
pub const DEFEAT_GREY: u32 = 0x8E8E93;
pub const WHITE: u32 = 0xFFFFFF;
pub const NOTCH_BLACK: u32 = 0x000000;
/// `--debug-shape` fill.
pub const DEBUG_RED: u32 = 0xFF3B30;
/// `--debug-shape` outline of the real notch.
pub const DEBUG_OUTLINE: u32 = 0xFFD60A;

/// The 1pt inner stroke when the notch is expanded, and how low its pulse dips.
pub const STROKE_OPACITY: f32 = 0.35;
pub const STROKE_PULSE_MIN: f32 = 0.15;
pub const STROKE_PULSE_PERIOD: f64 = 1.8;
/// Battery capsule outline and nub.
pub const CAPSULE_OUTLINE_OPACITY: f64 = 0.45;

// ---------------------------------------------------------------------------
// Shape (points)
// ---------------------------------------------------------------------------

/// Bottom corner radius of the real notch. The Collapsed shape uses it, so it
/// must hide inside the hole: erring large is invisible, erring small shows
/// black slivers at the corners.
pub const NOTCH_CORNER_RADIUS: f64 = 10.0;
/// Fine-tune the notch rect if `--debug-shape` shows a mismatch. Positive
/// values make the shape wider/taller than what macOS reports.
pub const NOTCH_TRIM_WIDTH: f64 = 0.0;
pub const NOTCH_TRIM_HEIGHT: f64 = 0.0;

/// Concave "ears" where the expanded shape meets the top of the screen.
pub const EAR_RADIUS: f64 = 8.0;
/// Bottom corner radius when expanded.
pub const EXPANDED_CORNER_RADIUS: f64 = 14.0;
/// Ears state: how far the shape reaches out on each side of the notch.
pub const EARS_EXTRA_WIDTH: f64 = 110.0;
/// Lair state: the notch drops down into a panel.
pub const LAIR_WIDTH: f64 = 400.0;
pub const LAIR_EXTRA_HEIGHT: f64 = 64.0;
/// DEFEAT: how far the bottom edge sags in the middle.
pub const DROOP_SAG: f64 = 4.0;

/// The panel is fixed-size: room for the biggest state, the bolt and the shake.
pub const WINDOW_PAD_X: f64 = 160.0;
pub const WINDOW_PAD_BOTTOM: f64 = 140.0;

// ---------------------------------------------------------------------------
// Springs
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub damping: f64,
    pub stiffness: f64,
    pub mass: f64,
}

/// Collapsed -> Lair in ZAP. Underdamped, so it overshoots.
pub const OPEN_SPRING: Spring = Spring {
    damping: 11.0,
    stiffness: 190.0,
    mass: 1.0,
};
/// Collapsed -> Ears in VICTORY: a little perkier, a little less bounce.
pub const EARS_SPRING: Spring = Spring {
    damping: 14.0,
    stiffness: 260.0,
    mass: 1.0,
};
/// Back to Collapsed. Stiffer and better damped, so the notch "snaps" home.
pub const CLOSE_SPRING: Spring = Spring {
    damping: 24.0,
    stiffness: 320.0,
    mass: 1.0,
};

// ---------------------------------------------------------------------------
// Lightning
// ---------------------------------------------------------------------------

pub const BOLT_LENGTH: f64 = 110.0;
/// Midpoint-displacement levels (picked at random per frame, inclusive).
pub const BOLT_LEVELS: (u32, u32) = (5, 6);
/// First-level sideways displacement as a fraction of the bolt length.
pub const BOLT_ROUGHNESS: f64 = 0.22;
/// How far the tip may wander sideways, as a fraction of the length.
pub const BOLT_TIP_WANDER: f64 = 0.18;
/// Side branches per bolt (inclusive).
pub const BOLT_BRANCHES: (u32, u32) = (1, 2);
pub const BOLT_CORE_WIDTH: f64 = 1.5;
pub const BOLT_GLOW_WIDTH: f64 = 5.0;
pub const BOLT_GLOW_RADIUS: f64 = 8.0;

/// Charge-up sparks: how many, and how long each jagged line is.
pub const SPARK_COUNT: (u32, u32) = (3, 5);
pub const SPARK_LENGTH: (f64, f64) = (2.0, 4.0);
/// Electric arcs along the panel's bottom edge during the monologue.
pub const ARC_COUNT: (u32, u32) = (2, 3);
pub const ARC_LENGTH: (f64, f64) = (22.0, 60.0);
pub const ARC_OPACITY: f32 = 0.45;
/// VICTORY sparks shooting out of the ears.
pub const VICTORY_SPARKS: (u32, u32) = (6, 8);
/// Sparks in the puff when ZAP's notch closes.
pub const PUFF_SPARKS: u32 = 6;

/// Regeneration periods (seconds) for the crackling effects.
pub const SPARK_FLICKER: f64 = 0.050;
pub const BOLT_FLICKER: f64 = 0.045;
pub const ARC_FLICKER: f64 = 0.080;

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

/// SF Symbol point size for the ear icon.
pub const ICON_POINT_SIZE: f64 = 15.0;
/// Icon centre, measured in from the outer edge of the left ear.
pub const ICON_INSET: f64 = 30.0;
pub const ICON_GLOW_RADIUS: f64 = 6.0;
pub const ICON_GLOW_PERIOD: f64 = 1.4;

/// Percentage: SF Pro Rounded Heavy.
pub const PERCENT_FONT_SIZE: f64 = 14.0;
/// Battery capsule (outline size, excluding the nub).
pub const CAPSULE_WIDTH: f64 = 24.0;
pub const CAPSULE_HEIGHT: f64 = 11.0;
/// Capsule's right edge, measured in from the outer edge of the right ear.
pub const CAPSULE_INSET: f64 = 18.0;
/// Gap between the percentage and the capsule.
pub const PERCENT_GAP: f64 = 5.0;
/// 0 -> level count-up time.
pub const COUNTER_DURATION: f64 = 0.6;

/// Villain line: SF Mono Bold, all caps, never wrapped.
pub const LINE_FONT_SIZE: f64 = 13.0;
/// Fallback size when a line is too long for the Lair at 13pt.
pub const LINE_FONT_SIZE_SMALL: f64 = 11.0;
/// Longest line that may use the full 13pt size.
pub const MAX_LINE_CHARS: usize = 32;
/// Horizontal breathing room inside the Lair panel, per side.
pub const LINE_SIDE_MARGIN: f64 = 20.0;
pub const TYPE_CHARS_PER_SEC: f64 = 35.0;
pub const CURSOR_BLINK: f64 = 0.45;

// ---------------------------------------------------------------------------
// Choreography (seconds from the start of each sequence)
// ---------------------------------------------------------------------------

pub mod zap {
    /// Charge-up sparks flicker at the notch's bottom corners until the strike.
    pub const STRIKE: f64 = 0.35;
    /// The bolt crackles until here, then fades out by `STRIKE_END`.
    pub const BOLT_FADE: f64 = 0.60;
    pub const STRIKE_END: f64 = 0.70;
    pub const FLASH_DURATION: f64 = 0.06;
    pub const FLASH_OPACITY: f32 = 0.25;
    /// A second flash this long after BEHOLD, once the shape is visible.
    pub const FLASH_ECHO: f64 = 0.07;
    /// Spring Collapsed -> Lair and shake.
    pub const BEHOLD: f64 = 0.45;
    pub const SHAKE_DURATION: f64 = 0.30;
    pub const SHAKE_OFFSETS: [f64; 8] = [0.0, 6.0, -6.0, 4.0, -4.0, 2.0, -2.0, 0.0];
    /// Icon, counter, capsule and typewriter line.
    pub const MONOLOGUE: f64 = 0.95;
    /// Arcs start once the spring has mostly settled.
    pub const ARCS_START: f64 = 1.15;
    pub const RETREAT: f64 = 3.90;
    pub const CONTENT_FADE: f64 = 0.15;
    pub const PUFF: f64 = 4.30;
    pub const PUFF_DURATION: f64 = 0.30;
    pub const END: f64 = 4.65;
}

pub mod defeat {
    pub const EXPAND_DURATION: f64 = 0.30;
    pub const CONTENT_IN: f64 = 0.22;
    pub const DROOP: f64 = 0.45;
    /// The sag lasts this long (down, hold, recover).
    pub const DROOP_DURATION: f64 = 0.40;
    pub const RETREAT: f64 = 1.40;
    pub const CONTENT_FADE: f64 = 0.20;
    pub const COLLAPSE: f64 = 1.50;
    pub const COLLAPSE_DURATION: f64 = 0.50;
    pub const END: f64 = 2.05;
}

pub mod victory {
    pub const CONTENT_IN: f64 = 0.25;
    pub const SPARKS: f64 = 0.30;
    pub const SPARKS_DURATION: f64 = 0.45;
    pub const SHIMMER: f64 = 0.40;
    pub const SHIMMER_DURATION: f64 = 0.45;
    /// With SHOW_FULL_LINE: drop into the Lair and type.
    pub const LAIR: f64 = 0.55;
    pub const TYPE_START: f64 = 0.85;
    /// Without the line the ears retreat earlier.
    pub const RETREAT_SHORT: f64 = 1.75;
    pub const RETREAT_LONG: f64 = 2.05;
    pub const CONTENT_FADE: f64 = 0.15;
    pub const END_AFTER_RETREAT: f64 = 0.55;
}

/// Reduce Motion: cross-fades instead of springs, nothing flickers.
pub mod calm {
    pub const FADE_IN: f64 = 0.20;
    pub const BOLT_HOLD: f64 = 0.35;
    pub const HOLD: f64 = 2.60;
    pub const FADE_OUT: f64 = 0.30;
}

// ---------------------------------------------------------------------------
// Villain lines. `{pct}` becomes the battery level. Keep them <= 32 chars.
// ---------------------------------------------------------------------------

/// Below this percentage a plug-in uses the "low" lines.
pub const LOW_BATTERY: u8 = 20;

pub const LINES_PLUG: &[&str] = &[
    "BEHOLD! {pct}% AND RISING!",
    "THE WALL SOCKET KNEELS BEFORE ME",
    "ELECTRONS: ACQUIRED. MWAHAHA!",
    "MORE VOLTS! MORE! MOOORE!",
    "PHASE ONE OF MY PLAN: CHARGING",
    "THE GRID IS MINE. ALL OF IT.",
    "YES... YES! FEED THE FOREHEAD!",
];

pub const LINES_PLUG_LOW: &[&str] = &[
    "SAVED BY THE GRID... THIS TIME",
    "{pct}%?! MINIONS, THE CABLE! NOW!",
    "ONE MORE MINUTE AND ALL WAS LOST",
];

pub const LINES_UNPLUG: &[&str] = &[
    "NOOO! MY PRECIOUS ELECTRONS!",
    "YOU HAVEN'T SEEN THE LAST OF ME",
    "RETREAT! RETREAT TO BATTERY!",
];

pub const LINES_FULL: &[&str] = &["MAXIMUM POWER ACHIEVED", "FULLY CHARGED. FULLY EVIL."];

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/// Animation ticker rate while a sequence runs (no ticker at all when idle).
pub const TICK_HZ: f64 = 60.0;
/// `--slow` playback speed.
pub const SLOW_SPEED: f32 = 0.25;
/// `--demo-low` fake battery level.
pub const DEMO_LOW_PERCENT: u8 = 9;
/// `--debug-shape` hold time.
pub const DEBUG_SHAPE_HOLD: f64 = 5.0;
/// After wake, power changes are absorbed silently for this long, so a
/// charger connected during sleep never fires.
pub const WAKE_GRACE: f64 = 2.5;
/// A plug-in (or unplug) while the display is off is held this long; if the
/// display comes on in time (often the plug-in itself wakes it), it plays.
pub const DISPLAY_WAKE_WINDOW: f64 = 3.0;
