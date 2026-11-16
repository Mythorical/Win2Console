//! Startup animation, in the spirit of the Xbox Series X|S boot sequence (recreated procedurally; no
//! Microsoft assets): a thin line morphs from a four-point star into a circle, blooms into a spinning
//! sphere of folded petals, shrinks into an emblem, and the wordmark fades in. About 2.5 seconds.
//!
//! Shapes are SVG path strings on a 1920×1080 viewbox, recomputed every frame.

use std::f32::consts::TAU;

/// Total length, including the final fade-out.
pub const DURATION: f32 = 2.6;

const CX: f32 = 960.0;
const CY: f32 = 470.0;

/// One filled petal of the sphere.
#[derive(Debug, Clone, PartialEq)]
pub struct Petal {
    pub path: String,
    /// 0 = dark green, 1 = light green, 2 = white highlight.
    pub shade: i32,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub outline: String,
    pub outline_opacity: f32,
    pub petals: Vec<Petal>,
    /// Shaded ball behind the petals.
    pub sphere_radius: f32,
    pub sphere_opacity: f32,
    pub emblem_radius: f32,
    pub emblem_opacity: f32,
    pub wordmark_opacity: f32,
    /// Whole-screen fade at the end.
    pub opacity: f32,
    /// Shadow under the shape (radius, opacity).
    pub shadow: (f32, f32),
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// 0→1 progress of `t` within [a, b].
fn phase(t: f32, a: f32, b: f32) -> f32 {
    smooth((t - a) / (b - a))
}

fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x
}

/// Closed outline through `n` points of a radius function.
fn radial_path(n: usize, rotation: f32, r: impl Fn(f32) -> f32) -> String {
    let mut s = String::with_capacity(n * 24);
    for i in 0..=n {
        let a = i as f32 / n as f32 * TAU;
        let rr = r(a);
        let (x, y) = (CX + rr * (a + rotation).cos(), CY + rr * (a + rotation).sin());
        s.push_str(&format!("{} {x:.1} {y:.1} ", if i == 0 { "M" } else { "L" }));
    }
    s.push('Z');
    s
}

pub fn frame(t: f32) -> Frame {
    // 1) Line morph: four-point star -> wobbling blob -> circle, growing and turning.
    let grow = phase(t, 0.0, 0.9);
    let radius = lerp(60.0, 230.0, grow);
    let to_blob = phase(t, 0.15, 0.55);
    let to_circle = phase(t, 0.5, 0.95);
    let wobble = 0.5 * t;
    let outline = radial_path(120, t * 2.2, |a| {
        let star = 0.3 + 0.7 * (2.0 * a).cos().abs().powf(3.0);
        let blob = 1.0 + 0.16 * (3.0 * a + wobble * TAU).sin() + 0.08 * (5.0 * a - wobble * 9.0).cos();
        let shape = lerp(lerp(star, blob, to_blob), 1.0, to_circle);
        radius * shape
    });
    let outline_opacity = phase(t, 0.0, 0.12) * (1.0 - phase(t, 0.85, 1.15));

    // 2) Petal sphere: lens-shaped petals around the centre, spinning, blooming then shrinking.
    let bloom = phase(t, 0.75, 1.15);
    let shrink = phase(t, 1.55, 2.0);
    let sphere_r = lerp(235.0, 48.0, shrink) * lerp(0.6, 1.0, bloom);
    let spin = t * 2.4;
    let petal_count = 14;
    let mut petals: Vec<(f32, Petal)> = (0..petal_count)
        .map(|i| {
            let base = i as f32 / petal_count as f32 * TAU + spin;
            // Fake depth: petals facing us are wider and drawn on top.
            let depth = (base + spin * 0.6).sin();
            // Folds reach the rim of the ball; ones facing us are wider.
            let length = sphere_r * lerp(0.92, 1.0, (depth + 1.0) / 2.0);
            let width = sphere_r * lerp(0.18, 0.5, (depth + 1.0) / 2.0);
            let (dx, dy) = (base.cos(), base.sin());
            let (nx, ny) = (-dy, dx);
            let tip = (CX + dx * length, CY + dy * length);
            let mid = (CX + dx * length * 0.5, CY + dy * length * 0.5);
            let c1 = (mid.0 + nx * width, mid.1 + ny * width);
            let c2 = (mid.0 - nx * width, mid.1 - ny * width);
            let path = format!(
                "M {CX:.1} {CY:.1} Q {:.1} {:.1} {:.1} {:.1} Q {:.1} {:.1} {CX:.1} {CY:.1} Z",
                c1.0, c1.1, tip.0, tip.1, c2.0, c2.1
            );
            let shade = if depth > 0.55 {
                2
            } else if depth > -0.2 {
                1
            } else {
                0
            };
            (depth, Petal { path, shade, opacity: bloom * (1.0 - phase(t, 1.95, 2.15)) })
        })
        .collect();
    petals.sort_by(|a, b| a.0.total_cmp(&b.0));

    // 3) Emblem (white disc) and wordmark.
    let emblem = phase(t, 1.85, 2.15);
    let wordmark_opacity = phase(t, 1.95, 2.3);
    let opacity = 1.0 - phase(t, 2.35, DURATION);
    let shadow_r = lerp(radius, sphere_r, phase(t, 0.75, 1.15)) * 1.1;

    Frame {
        outline,
        outline_opacity,
        petals: petals.into_iter().map(|(_, p)| p).collect(),
        sphere_radius: sphere_r,
        sphere_opacity: bloom * (1.0 - phase(t, 1.95, 2.15)),
        emblem_radius: lerp(sphere_r, 46.0, emblem),
        emblem_opacity: emblem,
        wordmark_opacity,
        opacity,
        shadow: (shadow_r, 0.35 * (1.0 - phase(t, 2.0, 2.3))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_progress_and_end_faded() {
        let start = frame(0.0);
        assert_eq!(start.outline_opacity, 0.0);
        assert!(start.petals.iter().all(|p| p.opacity == 0.0));
        let mid = frame(1.3);
        assert!(mid.petals.iter().any(|p| p.opacity > 0.9));
        let end = frame(DURATION);
        assert_eq!(end.opacity, 0.0);
        assert!(end.wordmark_opacity > 0.99);
        assert!(frame(0.5).outline.starts_with("M ") && frame(0.5).outline.ends_with('Z'));
    }
}
