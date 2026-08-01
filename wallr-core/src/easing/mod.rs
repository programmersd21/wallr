//! Easing curves used by animation packages and validation tooling.

use crate::animation::Easing;

/// Evaluate an easing curve at `t` in the inclusive range `0..=1`.
pub fn sample(curve: &Easing, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        Easing::Linear => t,
        Easing::EaseIn => t * t * t,
        Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
        Easing::EaseInOut => cubic_bezier(t, 0.4, 0.0, 0.2, 1.0),
        Easing::Emphatic => cubic_bezier(t, 0.18, 0.89, 0.32, 1.28),
        Easing::Spring => spring(t, 1.0, 170.0, 26.0),
    }
}

/// Evaluate a cubic Bézier timing curve using Newton iteration and bisection.
pub fn cubic_bezier(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let mut u = t;
    for _ in 0..8 {
        let x = bezier(u, x1, x2) - t;
        let dx = 3.0 * (1.0 - u).powi(2) * x1
            + 6.0 * (1.0 - u) * u * (x2 - x1)
            + 3.0 * u.powi(2) * (1.0 - x2);
        if dx.abs() < 1e-5 {
            break;
        }
        u = (u - x / dx).clamp(0.0, 1.0);
    }
    bezier(u, y1, y2)
}

fn bezier(t: f32, p1: f32, p2: f32) -> f32 {
    3.0 * (1.0 - t).powi(2) * t * p1 + 3.0 * (1.0 - t) * t.powi(2) * p2 + t.powi(3)
}

/// Evaluate a damped spring. Parameters are mass, stiffness, and damping.
pub fn spring(t: f32, mass: f32, stiffness: f32, damping: f32) -> f32 {
    let omega = (stiffness / mass.max(0.001)).sqrt();
    let zeta = damping / (2.0 * (stiffness * mass).sqrt().max(0.001));
    let e = (-zeta * omega * t * 6.0).exp();
    if zeta < 1.0 {
        let wd = omega * (1.0 - zeta * zeta).sqrt();
        1.0 - e * ((wd * t * 6.0).cos() + zeta * omega / wd * (wd * t * 6.0).sin())
    } else {
        1.0 - e * (1.0 + omega * t * 6.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curves_start_and_end_at_expected_points() {
        for curve in [Easing::Linear, Easing::EaseInOut, Easing::Emphatic] {
            assert!((sample(&curve, 0.0)).abs() < 0.01);
            assert!((sample(&curve, 1.0) - 1.0).abs() < 0.01);
        }
    }
}
