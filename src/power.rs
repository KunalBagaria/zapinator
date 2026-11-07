//! Power events straight from IOKit's power-source API.
//!
//! `IOPSNotificationCreateRunLoopSource` gives us a run-loop source that fires
//! whenever anything about the power sources changes, which includes every
//! percentage tick. [`Tracker`] keeps the previous state and turns that
//! stream into the few transitions we animate.

use std::ffi::c_void;
use std::fmt;

use core_foundation::ConcreteCFType;
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::runloop::{
    CFRunLoop, CFRunLoopSource, CFRunLoopSourceRef, kCFRunLoopCommonModes,
};
use core_foundation::string::CFString;

type IOPowerSourceCallbackType = extern "C" fn(context: *mut c_void);

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// Create rule: the caller owns the returned source.
    fn IOPSNotificationCreateRunLoopSource(
        callback: IOPowerSourceCallbackType,
        context: *mut c_void,
    ) -> CFRunLoopSourceRef;
    /// Copy rule: the caller owns the returned blob.
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    /// Copy rule: the caller owns the returned array.
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFArrayRef;
    /// Get rule: the dictionary belongs to `blob`; don't release it.
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, ps: CFTypeRef) -> CFDictionaryRef;
}

/// A snapshot of the internal battery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerState {
    pub on_ac: bool,
    pub percent: u8,
    pub charging: bool,
    pub charged: bool,
}

impl fmt::Display for PowerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}% on {}{}{}",
            self.percent,
            if self.on_ac { "AC" } else { "battery" },
            if self.charging { ", charging" } else { "" },
            if self.charged { ", charged" } else { "" }
        )
    }
}

fn get<T: ConcreteCFType>(desc: &CFDictionary<CFString, CFType>, key: &'static str) -> Option<T> {
    desc.find(CFString::from_static_string(key))
        .and_then(|v| v.downcast::<T>())
}

/// Reads the internal battery's state, or `None` on a Mac without one.
pub fn read() -> Option<PowerState> {
    // SAFETY: Copy rule; `wrap_under_create_rule` takes ownership and
    // releases the blob when `blob` drops.
    let blob = unsafe {
        let raw = IOPSCopyPowerSourcesInfo();
        if raw.is_null() {
            return None;
        }
        CFType::wrap_under_create_rule(raw)
    };
    // SAFETY: Copy rule again; the array is released on drop.
    let list: CFArray<CFType> = unsafe {
        let raw = IOPSCopyPowerSourcesList(blob.as_CFTypeRef());
        if raw.is_null() {
            return None;
        }
        CFArray::wrap_under_create_rule(raw)
    };

    let mut fallback = None;
    for source in list.iter() {
        // SAFETY: Get rule; `wrap_under_get_rule` retains it for as long as
        // our wrapper lives (and `blob` outlives the loop anyway).
        let desc: CFDictionary<CFString, CFType> = unsafe {
            let raw = IOPSGetPowerSourceDescription(blob.as_CFTypeRef(), source.as_CFTypeRef());
            if raw.is_null() {
                continue;
            }
            CFDictionary::wrap_under_get_rule(raw)
        };
        let Some(state) = parse(&desc) else { continue };
        let internal = get::<CFString>(&desc, "Type").is_some_and(|t| t == "InternalBattery");
        if internal {
            return Some(state);
        }
        fallback.get_or_insert(state);
    }
    fallback
}

fn parse(desc: &CFDictionary<CFString, CFType>) -> Option<PowerState> {
    let source_state = get::<CFString>(desc, "Power Source State")?.to_string();
    let current = get::<CFNumber>(desc, "Current Capacity")?.to_i64()?;
    let max = get::<CFNumber>(desc, "Max Capacity")?.to_i64()?;
    if max <= 0 {
        return None;
    }
    let flag = |key| get::<CFBoolean>(desc, key).is_some_and(bool::from);
    Some(PowerState {
        on_ac: source_state == "AC Power",
        percent: ((current as f64 / max as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8,
        charging: flag("Is Charging"),
        charged: flag("Is Charged"),
    })
}

/// Keeps the IOKit run-loop source alive. Dropping it stops notifications.
pub struct Monitor {
    source: CFRunLoopSource,
    // Boxed twice so the context pointer is thin and stable.
    _callback: Box<Box<dyn Fn()>>,
}

extern "C" fn trampoline(context: *mut c_void) {
    // SAFETY: `context` is the pointer to the `Box<dyn Fn()>` owned by the
    // `Monitor`, which removes the source before freeing it. IOKit calls us on
    // the main run loop, the only thread that ever touches it.
    let callback = unsafe { &*context.cast::<Box<dyn Fn()>>() };
    callback();
}

/// Calls `on_change` (on the main thread) whenever any power source changes.
pub fn watch(on_change: impl Fn() + 'static) -> Option<Monitor> {
    let callback: Box<Box<dyn Fn()>> = Box::new(Box::new(on_change));
    let context = (&*callback as *const Box<dyn Fn()>)
        .cast_mut()
        .cast::<c_void>();
    // SAFETY: Create rule; the source is released when `Monitor` drops.
    let source = unsafe {
        let raw = IOPSNotificationCreateRunLoopSource(trampoline, context);
        if raw.is_null() {
            return None;
        }
        CFRunLoopSource::wrap_under_create_rule(raw)
    };
    // SAFETY: reading an immutable CoreFoundation constant.
    CFRunLoop::get_main().add_source(&source, unsafe { kCFRunLoopCommonModes });
    Some(Monitor {
        source,
        _callback: callback,
    })
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: as in `watch`.
        CFRunLoop::get_main().remove_source(&self.source, unsafe { kCFRunLoopCommonModes });
    }
}

/// The transitions worth animating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    PluggedIn,
    Unplugged,
    Full,
}

/// Turns raw power snapshots into transitions. The first snapshot (launch)
/// and any [`Tracker::resync`] (wake) never produce an event.
#[derive(Debug, Default)]
pub struct Tracker {
    prev: Option<PowerState>,
    /// VICTORY already shown (or not deserved) during this plug-in.
    full_done: bool,
}

impl Tracker {
    pub fn update(&mut self, new: PowerState) -> Option<Event> {
        let Some(prev) = self.prev.replace(new) else {
            self.full_done = is_full(new);
            return None;
        };
        match (prev.on_ac, new.on_ac) {
            (false, true) => {
                // Plugging in at 100% is a ZAP, not also a VICTORY later.
                self.full_done = is_full(new);
                Some(Event::PluggedIn)
            }
            (true, false) => {
                self.full_done = false;
                Some(Event::Unplugged)
            }
            (true, true) if !self.full_done && is_full(new) && !is_full(prev) => {
                self.full_done = true;
                Some(Event::Full)
            }
            _ => None,
        }
    }

    /// Adopts `new` silently (after wake, or while the screen is asleep).
    pub fn resync(&mut self, new: PowerState) {
        let same_session = self.prev.is_some_and(|p| p.on_ac == new.on_ac && new.on_ac);
        self.full_done = if same_session {
            self.full_done || is_full(new)
        } else {
            is_full(new)
        };
        self.prev = Some(new);
    }
}

fn is_full(s: PowerState) -> bool {
    s.on_ac && s.percent >= 100
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(on_ac: bool, percent: u8) -> PowerState {
        PowerState {
            on_ac,
            percent,
            charging: on_ac && percent < 100,
            charged: percent == 100,
        }
    }

    #[test]
    fn launch_is_silent() {
        let mut t = Tracker::default();
        assert_eq!(t.update(st(true, 50)), None);
    }

    #[test]
    fn plug_and_unplug() {
        let mut t = Tracker::default();
        t.update(st(false, 50));
        assert_eq!(t.update(st(true, 50)), Some(Event::PluggedIn));
        assert_eq!(t.update(st(false, 50)), Some(Event::Unplugged));
    }

    #[test]
    fn percentage_ticks_are_ignored() {
        let mut t = Tracker::default();
        t.update(st(false, 50));
        for p in (40..50).rev() {
            assert_eq!(t.update(st(false, p)), None);
        }
        t.update(st(true, 40));
        for p in 41..99 {
            assert_eq!(t.update(st(true, p)), None);
        }
    }

    #[test]
    fn full_fires_once_per_plug_in() {
        let mut t = Tracker::default();
        t.update(st(false, 97));
        t.update(st(true, 97));
        assert_eq!(t.update(st(true, 99)), None);
        assert_eq!(t.update(st(true, 100)), Some(Event::Full));
        // Wobbling around 100 doesn't repeat it.
        assert_eq!(t.update(st(true, 99)), None);
        assert_eq!(t.update(st(true, 100)), None);
        // A new plug-in earns a new one.
        t.update(st(false, 99));
        t.update(st(true, 99));
        assert_eq!(t.update(st(true, 100)), Some(Event::Full));
    }

    #[test]
    fn plugging_in_full_is_only_a_zap() {
        let mut t = Tracker::default();
        t.update(st(false, 100));
        assert_eq!(t.update(st(true, 100)), Some(Event::PluggedIn));
        assert_eq!(t.update(st(true, 100)), None);
    }

    #[test]
    fn launching_full_on_ac_never_fires() {
        let mut t = Tracker::default();
        t.update(st(true, 100));
        assert_eq!(t.update(st(true, 100)), None);
    }

    #[test]
    fn wake_with_charger_in_is_silent() {
        let mut t = Tracker::default();
        t.update(st(false, 60));
        // Charger connected during sleep.
        t.resync(st(true, 62));
        assert_eq!(t.update(st(true, 63)), None);
        // But a later unplug still counts.
        assert_eq!(t.update(st(false, 63)), Some(Event::Unplugged));
    }

    #[test]
    fn reaching_full_while_asleep_is_silent() {
        let mut t = Tracker::default();
        t.update(st(false, 90));
        t.update(st(true, 90));
        t.resync(st(true, 100));
        assert_eq!(t.update(st(true, 100)), None);
    }
}
