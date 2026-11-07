//! Jagged electricity: midpoint-displacement bolts, tiny sparks and arcs.
//!
//! The generators are pure functions over an `Rng` (so they're testable);
//! `polyline_path` turns their output into a `CGPath` for a shape layer.

use std::f64::consts::PI;

use rand::{Rng, RngExt};

use crate::config;
use crate::shape::{Path, PathBuilder};

pub type Pt = (f64, f64);

/// Recursive midpoint displacement between `a` and `b`, done iteratively.
///
/// Each level splits every segment at its midpoint and pushes that midpoint
/// sideways (perpendicular to the segment) by up to `amplitude`; the
/// amplitude halves each level. Returns `2^levels + 1` points.
pub fn displace(rng: &mut impl Rng, a: Pt, b: Pt, levels: u32, roughness: f64) -> Vec<Pt> {
    let mut pts = vec![a, b];
    let mut amplitude = roughness * dist(a, b);
    for _ in 0..levels {
        let mut next = Vec::with_capacity(pts.len() * 2 - 1);
        for w in pts.windows(2) {
            let (p, q) = (w[0], w[1]);
            let (dx, dy) = (q.0 - p.0, q.1 - p.1);
            let len = (dx * dx + dy * dy).sqrt().max(f64::EPSILON);
            let (nx, ny) = (-dy / len, dx / len);
            let off = if amplitude > 0.0 {
                rng.random_range(-amplitude..=amplitude)
            } else {
                0.0
            };
            next.push(p);
            next.push(((p.0 + q.0) / 2.0 + nx * off, (p.1 + q.1) / 2.0 + ny * off));
        }
        next.push(pts[pts.len() - 1]);
        pts = next;
        amplitude *= 0.5;
    }
    pts
}

/// A bolt: the main channel plus a couple of short forks.
pub struct Bolt {
    pub main: Vec<Pt>,
    pub branches: Vec<Vec<Pt>>,
}

impl Bolt {
    pub fn lines(&self) -> impl Iterator<Item = &[Pt]> {
        std::iter::once(self.main.as_slice()).chain(self.branches.iter().map(Vec::as_slice))
    }
}

/// A bolt shooting straight down from `origin`.
pub fn bolt(rng: &mut impl Rng, origin: Pt, length: f64) -> Bolt {
    let wander = length * config::BOLT_TIP_WANDER;
    let end = (
        origin.0 + rng.random_range(-wander..=wander),
        origin.1 - length,
    );
    let levels = rng.random_range(config::BOLT_LEVELS.0..=config::BOLT_LEVELS.1);
    let main = displace(rng, origin, end, levels, config::BOLT_ROUGHNESS);

    let count = rng.random_range(config::BOLT_BRANCHES.0..=config::BOLT_BRANCHES.1);
    let branches = (0..count)
        .map(|_| {
            // Fork from somewhere in the middle stretch of the main channel.
            let i = rng.random_range(main.len() / 4..=main.len() * 2 / 3);
            let start = main[i];
            let side = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
            // Straight down is -90°; lean 25–55° off it.
            let angle = -PI / 2.0 + side * rng.random_range(25f64..55.0).to_radians();
            let len = length * rng.random_range(0.22..0.42);
            let end = (start.0 + angle.cos() * len, start.1 + angle.sin() * len);
            displace(rng, start, end, 4, 0.28)
        })
        .collect();
    Bolt { main, branches }
}

/// A tiny jagged spark (two or three kinks) starting near `at`.
pub fn spark(rng: &mut impl Rng, at: Pt) -> Vec<Pt> {
    let length = rng.random_range(config::SPARK_LENGTH.0..=config::SPARK_LENGTH.1);
    let kinks = rng.random_range(2..=3);
    let mut angle = rng.random_range(0.0..2.0 * PI);
    let mut p = at;
    let mut pts = vec![p];
    for _ in 0..kinks {
        let step = length / kinks as f64;
        p = (p.0 + angle.cos() * step, p.1 + angle.sin() * step);
        pts.push(p);
        angle += rng.random_range(-1.2..1.2);
    }
    pts
}

/// A short jagged trail for a flying spark: starts at the origin and points
/// back against its direction of travel (`angle`, radians).
pub fn streak(rng: &mut impl Rng, angle: f64) -> Vec<Pt> {
    let length = rng.random_range(3.0..6.5);
    let mut p = (0.0, 0.0);
    let mut pts = vec![p];
    for _ in 0..3 {
        let a = angle + PI + rng.random_range(-0.8..0.8);
        p = (p.0 + a.cos() * length / 3.0, p.1 + a.sin() * length / 3.0);
        pts.push(p);
    }
    pts
}

/// A low, crackly arc hugging a horizontal edge at height `y`.
pub fn edge_arc(rng: &mut impl Rng, x_range: (f64, f64), y: f64) -> Vec<Pt> {
    let len = rng.random_range(config::ARC_LENGTH.0..=config::ARC_LENGTH.1);
    let span = (x_range.1 - x_range.0 - len).max(0.0);
    let x0 = x_range.0 + rng.random_range(0.0..=span);
    // Start and end just outside the edge so the arc visibly jumps along it.
    let a = (x0, y - rng.random_range(0.0..1.5));
    let b = (x0 + len, y - rng.random_range(0.0..1.5));
    displace(rng, a, b, 4, 0.16)
}

/// One `CGPath` containing every polyline as its own open subpath.
pub fn polyline_path<'a>(lines: impl IntoIterator<Item = &'a [Pt]>) -> Path {
    let p = PathBuilder::new();
    for line in lines {
        let mut it = line.iter().copied();
        if let Some(first) = it.next() {
            p.move_to(first);
            it.for_each(|pt| p.line_to(pt));
        }
    }
    p.finish()
}

fn dist(a: Pt, b: Pt) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn displacement_keeps_endpoints_and_doubles_points() {
        let mut rng = StdRng::seed_from_u64(7);
        for levels in 0..=6 {
            let pts = displace(&mut rng, (0.0, 0.0), (0.0, -100.0), levels, 0.25);
            assert_eq!(pts.len(), (1 << levels) + 1);
            assert_eq!(pts[0], (0.0, 0.0));
            assert_eq!(*pts.last().unwrap(), (0.0, -100.0));
        }
    }

    #[test]
    fn displacement_stays_within_the_amplitude_budget() {
        // Total sideways drift is bounded by the geometric series 2·a.
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..200 {
            let pts = displace(&mut rng, (0.0, 0.0), (0.0, -100.0), 6, 0.2);
            assert!(pts.iter().all(|p| p.0.abs() <= 40.0 + 1e-9));
        }
    }

    #[test]
    fn bolts_go_down_and_fork() {
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..200 {
            let b = bolt(&mut rng, (100.0, 50.0), 110.0);
            assert_eq!(b.main[0], (100.0, 50.0));
            assert!((b.main.last().unwrap().1 - (50.0 - 110.0)).abs() < 1e-9);
            assert!((1..=2).contains(&b.branches.len()));
            for br in &b.branches {
                assert!(br.last().unwrap().1 < br[0].1, "branches point downwards");
            }
            assert_eq!(b.lines().count(), 1 + b.branches.len());
        }
    }

    #[test]
    fn sparks_are_tiny() {
        let mut rng = StdRng::seed_from_u64(3);
        for _ in 0..200 {
            let s = spark(&mut rng, (10.0, 10.0));
            let len: f64 = s.windows(2).map(|w| dist(w[0], w[1])).sum();
            assert!((config::SPARK_LENGTH.0 - 1e-9..=config::SPARK_LENGTH.1 + 1e-9).contains(&len));
        }
    }

    #[test]
    fn arcs_stay_in_range() {
        let mut rng = StdRng::seed_from_u64(9);
        for _ in 0..200 {
            let a = edge_arc(&mut rng, (0.0, 300.0), 20.0);
            assert!(a[0].0 >= 0.0 && a.last().unwrap().0 <= 300.0);
        }
    }
}
