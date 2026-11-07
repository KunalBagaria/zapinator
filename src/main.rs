//! NOTCH-ZAP-INATOR: the MacBook notch announces your technological
//! superiority every time the charger goes in.

mod config;
mod content;
mod geometry;
mod lightning;
mod power;
mod sequences;
mod shape;
mod sound;
mod window;

use std::cell::RefCell;
use std::process::ExitCode;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy,
    NSApplicationDidChangeScreenParametersNotification, NSWorkspace,
    NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceScreensDidWakeNotification, NSWorkspaceWillSleepNotification,
};
use objc2_core_graphics::{CGDisplayIsAsleep, CGMainDisplayID};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSNotificationName, NSObjectProtocol, NSOperationQueue,
};

use crate::power::{Event, PowerState, Tracker};
use crate::sequences::{Director, Show};

const USAGE: &str = "\
notch-zap-inator: a dramatic charging animation for the MacBook notch

USAGE:
    notch-zap-inator [FLAG]

With no flag it runs as a silent background agent.

FLAGS:
    --demo            play ZAP once with the real battery level, then exit
    --demo-low        play ZAP with a fake 9%
    --demo-unplug     play DEFEAT
    --demo-full       play VICTORY
    --slow            play any demo at 0.25x speed (combine with a --demo flag)
    --debug-shape     draw the expanded shape in red for 5s to check alignment
    --print-geometry  print screen and notch geometry, then exit
    -h, --help        show this help

Set RUST_LOG=debug for detailed logging.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Demo {
    Zap,
    ZapLow,
    Unplug,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Agent,
    Demo(Demo),
    DebugShape,
    PrintGeometry,
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    slow: bool,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut mode = None;
        let mut slow = false;
        for arg in args {
            let m = match arg.as_str() {
                "--demo" => Mode::Demo(Demo::Zap),
                "--demo-low" => Mode::Demo(Demo::ZapLow),
                "--demo-unplug" => Mode::Demo(Demo::Unplug),
                "--demo-full" => Mode::Demo(Demo::Full),
                "--debug-shape" => Mode::DebugShape,
                "--print-geometry" => Mode::PrintGeometry,
                "--slow" => {
                    slow = true;
                    continue;
                }
                "-h" | "--help" => return Err(String::new()),
                other => return Err(format!("unknown flag: {other}")),
            };
            if mode.replace(m).is_some() {
                return Err("pick one mode flag at a time".into());
            }
        }
        let mode = mode.unwrap_or(Mode::Agent);
        if slow && !matches!(mode, Mode::Demo(_)) {
            return Err("--slow only applies to a --demo flag".into());
        }
        Ok(Self { mode, slow })
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) if msg.is_empty() => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(msg) => {
            eprintln!("error: {msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    log::debug!("{args:?}");

    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("error: must start on the main thread");
        return ExitCode::FAILURE;
    };

    // Accessory: no Dock icon, no menu bar, never steals focus.
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    match args.mode {
        Mode::PrintGeometry => {
            geometry::print_report(mtm);
            ExitCode::SUCCESS
        }
        Mode::DebugShape => {
            geometry::print_report(mtm);
            play_once(mtm, &app, Show::DebugShape, 1.0)
        }
        Mode::Demo(demo) => {
            let real = || power::read().map_or(100, |s| s.percent);
            let show = match demo {
                Demo::Zap => Show::Zap { percent: real() },
                Demo::ZapLow => Show::Zap {
                    percent: config::DEMO_LOW_PERCENT,
                },
                Demo::Unplug => Show::Defeat { percent: real() },
                Demo::Full => Show::Victory,
            };
            let speed = if args.slow { config::SLOW_SPEED } else { 1.0 };
            play_once(mtm, &app, show, speed)
        }
        Mode::Agent => run_agent(mtm, &app),
    }
}

/// Plays one show, then quits.
fn play_once(mtm: MainThreadMarker, app: &NSApplication, show: Show, speed: f32) -> ExitCode {
    let director = Director::new(mtm, speed);
    director.borrow_mut().on_finished(move || {
        NSApplication::sharedApplication(mtm).terminate(None);
    });
    if !director.borrow_mut().play(show) {
        eprintln!("no notched built-in screen found (lid closed or external display only)");
        return ExitCode::FAILURE;
    }
    app.run();
    ExitCode::SUCCESS
}

// ===========================================================================
// Background agent
// ===========================================================================

/// Power bookkeeping plus the "don't fire right now" conditions.
struct Agent {
    tracker: Tracker,
    /// Between NSWorkspaceWillSleep and DidWake.
    asleep: bool,
    /// Just woke from sleep: IOKit may still report sleep-time changes late,
    /// so changes are absorbed silently until then.
    quiet_until: Option<Instant>,
    /// A show that happened while the display was off (typically the plug-in
    /// that is waking it). It plays if the display wakes soon after.
    held: Option<(Show, Instant)>,
}

/// Live check, so a missed notification or a launch while the displays are
/// off can't leave us confused.
fn display_asleep() -> bool {
    CGDisplayIsAsleep(CGMainDisplayID())
}

impl Agent {
    fn absorbing(&self) -> bool {
        self.asleep || self.quiet_until.is_some_and(|t| Instant::now() < t)
    }

    /// Back from system sleep: re-read the state without animating, and stay
    /// quiet for a moment in case IOKit reports the sleep-time change late.
    fn woke(&mut self) {
        self.asleep = false;
        self.held = None;
        self.quiet_until = Some(Instant::now() + Duration::from_secs_f64(config::WAKE_GRACE));
        if let Some(s) = power::read() {
            log::info!("woke; resynced silently: {s}");
            self.tracker.resync(s);
        }
    }

    fn on_power_change(&mut self, state: PowerState) -> Option<Show> {
        if self.absorbing() {
            log::debug!("power change around sleep, absorbed: {state}");
            self.tracker.resync(state);
            return None;
        }
        let event = self.tracker.update(state)?;
        let show = match event {
            Event::PluggedIn => Some(Show::Zap {
                percent: state.percent,
            }),
            Event::Unplugged if config::SHOW_UNPLUG => Some(Show::Defeat {
                percent: state.percent,
            }),
            Event::Full if config::SHOW_FULL => Some(Show::Victory),
            _ => None,
        };
        if display_asleep() {
            log::info!("{event:?}: {state} (display asleep; holding it)");
            self.held = show.map(|s| (s, Instant::now()));
            return None;
        }
        log::info!("{event:?}: {state}");
        show
    }

    /// The display came on. Plays what happened just before (e.g. the
    /// plug-in that woke it); anything older stays silent.
    fn on_display_wake(&mut self) -> Option<Show> {
        if self.asleep {
            // DidWake never arrived; recover here instead of staying deaf.
            self.woke();
            return None;
        }
        let (show, at) = self.held.take()?;
        (at.elapsed().as_secs_f64() <= config::DISPLAY_WAKE_WINDOW).then_some(show)
    }
}

type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// Subscribes `f` to `name` on `center`, delivered on the main queue.
fn observe(
    center: &NSNotificationCenter,
    name: &NSNotificationName,
    f: impl Fn() + 'static,
) -> Observer {
    let block = RcBlock::new(move |_n: NonNull<NSNotification>| f());
    // SAFETY: delivered on the main operation queue, i.e. the main thread,
    // where the non-Send state captured by `f` lives.
    unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        )
    }
}

fn run_agent(mtm: MainThreadMarker, app: &NSApplication) -> ExitCode {
    let director = Director::new(mtm, 1.0);
    let agent = Rc::new(RefCell::new(Agent {
        tracker: Tracker::default(),
        asleep: false,
        quiet_until: None,
        held: None,
    }));

    // The launch snapshot: recorded, never animated.
    match power::read() {
        Some(s) => {
            log::info!("starting: {s}");
            agent.borrow_mut().tracker.update(s);
        }
        None => log::warn!("no battery found; nothing will ever fire"),
    }

    let monitor = {
        let (agent, director) = (agent.clone(), director.clone());
        power::watch(move || {
            let Some(state) = power::read() else { return };
            log::debug!("IOKit power notification: {state}");
            let show = agent.borrow_mut().on_power_change(state);
            if let Some(show) = show {
                director.borrow_mut().play(show);
            }
        })
    };
    if monitor.is_none() {
        log::error!("couldn't subscribe to IOKit power notifications");
        return ExitCode::FAILURE;
    }

    let ws = NSWorkspace::sharedWorkspace().notificationCenter();
    let default = NSNotificationCenter::defaultCenter();
    // SAFETY: reading immutable AppKit notification-name constants.
    let (will_sleep, did_wake, screens_sleep, screens_wake, screens_changed) = unsafe {
        (
            NSWorkspaceWillSleepNotification,
            NSWorkspaceDidWakeNotification,
            NSWorkspaceScreensDidSleepNotification,
            NSWorkspaceScreensDidWakeNotification,
            NSApplicationDidChangeScreenParametersNotification,
        )
    };
    let _observers: Vec<Observer> = vec![
        observe(&ws, will_sleep, {
            let (agent, director) = (agent.clone(), director.clone());
            move || {
                log::debug!("going to sleep");
                let mut a = agent.borrow_mut();
                a.asleep = true;
                a.held = None;
                director.borrow_mut().stop();
            }
        }),
        observe(&ws, did_wake, {
            let agent = agent.clone();
            move || {
                log::debug!("woke up");
                agent.borrow_mut().woke();
            }
        }),
        observe(&ws, screens_sleep, {
            let director = director.clone();
            move || {
                log::debug!("screens asleep");
                director.borrow_mut().stop();
            }
        }),
        observe(&ws, screens_wake, {
            let (agent, director) = (agent.clone(), director.clone());
            move || {
                log::debug!("screens awake");
                let held = agent.borrow_mut().on_display_wake();
                if let Some(show) = held {
                    log::info!("display woke; playing the held {show:?}");
                    director.borrow_mut().play(show);
                }
            }
        }),
        observe(&default, screens_changed, {
            let director = director.clone();
            move || director.borrow_mut().screens_changed()
        }),
    ];

    log::info!("agent running");
    app.run();
    drop(monitor);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|s| s.to_string()))
    }

    fn agent() -> Agent {
        Agent {
            tracker: Tracker::default(),
            asleep: false,
            quiet_until: None,
            held: None,
        }
    }

    #[test]
    fn held_show_plays_only_if_the_display_wakes_soon() {
        let mut a = agent();
        a.held = Some((Show::Zap { percent: 50 }, Instant::now()));
        assert_eq!(a.on_display_wake(), Some(Show::Zap { percent: 50 }));
        assert_eq!(a.on_display_wake(), None, "a held show plays once");

        let stale = Instant::now() - Duration::from_secs_f64(config::DISPLAY_WAKE_WINDOW + 1.0);
        a.held = Some((Show::Zap { percent: 50 }, stale));
        assert_eq!(a.on_display_wake(), None);
    }

    #[test]
    fn sleep_absorbs_and_display_wake_recovers_a_missed_did_wake() {
        let mut a = agent();
        a.asleep = true;
        assert!(a.absorbing());
        a.held = Some((Show::Victory, Instant::now()));
        // DidWake never came, but the display did: recover, quietly.
        assert_eq!(a.on_display_wake(), None);
        assert!(!a.asleep && a.held.is_none());
        assert!(a.absorbing(), "the post-wake grace window applies");
    }

    #[test]
    fn flags() {
        assert_eq!(parse(&[]).unwrap().mode, Mode::Agent);
        assert_eq!(
            parse(&["--demo-low"]).unwrap().mode,
            Mode::Demo(Demo::ZapLow)
        );
        let a = parse(&["--slow", "--demo-full"]).unwrap();
        assert!(a.slow && a.mode == Mode::Demo(Demo::Full));
        assert!(parse(&["--slow"]).is_err());
        assert!(parse(&["--demo", "--demo-unplug"]).is_err());
        assert!(parse(&["--nope"]).is_err());
    }
}
