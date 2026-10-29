//! The base material of a 3D object: how rough and how metallic its surface is.
//!
//! The reference app's Materials tab gives its Default material two Base Properties, Roughness and
//! Metallic, each 0 to 1. Its renderer is not documented, so the shading here is our own, a vector
//! approximation of a metal–roughness surface:
//!
//! - A **non-metal** keeps the diffuse lighting of the other 3D settings (the paint lit by the
//!   lights and Ambient Light) and adds a white sheen: the lights' highlights and the reflection of
//!   a studio surround, stronger at grazing angles (Fresnel) and spread out as Roughness rises.
//! - A **metal** has no diffuse color of its own: it reflects the surround and the lights in its
//!   paint's color, so a smooth metal shows the surround's bands (a bright sky, a dark horizon, a
//!   grey ground) across its faces.
//!
//! At Roughness 1 and Metallic 0 (the defaults) both terms vanish and the surface shades exactly as
//! before materials, so documents keep their appearance.

use crate::{Revolve, V3, finite, unclipped_illumination};

/// Roughness and Metallic, each 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub roughness: f64,
    pub metallic: f64,
}

impl Default for Material {
    fn default() -> Self {
        Self { roughness: 1.0, metallic: 0.0 }
    }
}

/// A material's shading at one point: the paint is multiplied by `diffuse` (per RGB channel) and
/// `sheen` is added to the result, both before clipping to 0–1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shade {
    pub diffuse: [f64; 3],
    pub sheen: [f64; 3],
}

impl Shade {
    /// The color of `rgb` (0–1 each) shaded.
    pub fn apply(self, rgb: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| (rgb[i] * self.diffuse[i] + self.sheen[i]).clamp(0.0, 1.0))
    }
}

impl Material {
    pub(crate) fn sanitized(self) -> Self {
        Self { roughness: finite(self.roughness, 1.0, 0.0, 1.0), metallic: finite(self.metallic, 0.0, 0.0, 1.0) }
    }

    /// Does the material shade as the plain diffuse lighting (no sheen, no metal)?
    pub fn is_plain(self) -> bool {
        let m = self.sanitized();
        m.metallic <= 0.0 && m.roughness >= 1.0
    }
}

/// The studio surround's brightness in direction `r` (view space, y down), our own: a sky
/// brightening upward, a dark band at a horizon a little below the eye (so a face turned toward
/// the viewer reflects the lower sky) and a mid-grey ground.
fn surround(r: V3) -> f64 {
    const HORIZON: f64 = -0.12;
    const BAND: f64 = 0.08;
    let up = -r.1;
    let smooth = |e0: f64, e1: f64, x: f64| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mix = |a: f64, b: f64, t: f64| a + (b - a) * t;
    if up >= HORIZON {
        mix(BAND, 0.55 + 0.6 * smooth(HORIZON, 0.8, up), smooth(HORIZON, HORIZON + 0.08, up))
    } else {
        mix(BAND, 0.3 + 0.15 * smooth(HORIZON - 0.2, -1.0, up), smooth(HORIZON, HORIZON - 0.18, up))
    }
}

/// About the surround's average, which a fully rough surface reflects.
const SURROUND_MEAN: f64 = 0.55;

/// The material of `o` at a point with unit normal `n` facing `to_eye` (both view space, unit).
/// `legacy_light` is the single light of documents from before light lists.
pub(crate) fn shade(o: &Revolve, n: V3, to_eye: V3, legacy_light: V3) -> Shade {
    let diffuse = unclipped_illumination(o, n, legacy_light);
    let m = o.material.sanitized();
    if !o.shade || m.is_plain() {
        return Shade { diffuse, sheen: [0.0; 3] };
    }
    let gloss = 1.0 - m.roughness;
    let ndv = n.dot(to_eye).max(0.0);
    // The view reflected about the normal, and the surround it sees, blurred by roughness.
    let r = V3(2.0 * ndv * n.0 - to_eye.0, 2.0 * ndv * n.1 - to_eye.1, 2.0 * ndv * n.2 - to_eye.2);
    let env = gloss * surround(r) + m.roughness * SURROUND_MEAN;
    // Each light's highlight: a lobe about the half vector, tighter as roughness falls.
    let a = (m.roughness * m.roughness).max(0.004);
    let exponent = (2.0 / (a * a) - 2.0).clamp(1.0, 20_000.0);
    let mut highlight = [0.0; 3];
    let mut add = |dir: [f64; 3], color: [f64; 3], intensity: f64| {
        let h = V3(dir[0] + to_eye.0, dir[1] + to_eye.1, dir[2] + to_eye.2).unit();
        let lobe = intensity * n.dot(h).max(0.0).powf(exponent) * gloss;
        for (out, c) in highlight.iter_mut().zip(color) {
            *out += lobe * c;
        }
    };
    match o.lights {
        Some(lights) => lights.iter().for_each(|l| add(l.direction(), l.color, l.intensity)),
        None => add([legacy_light.0, legacy_light.1, legacy_light.2], o.light_color, o.light_intensity),
    }
    // Schlick's Fresnel for a dielectric (4% head on).
    let fresnel = 0.04 + 0.96 * (1.0 - ndv).powi(5);
    let dielectric = 1.0 - m.metallic;
    // A metal reflects in its own color; its brightest highlights also pale toward the light's.
    Shade {
        diffuse: std::array::from_fn(|i| dielectric * diffuse[i] + m.metallic * (env + highlight[i] + 0.15 * diffuse[i])),
        sheen: std::array::from_fn(|i| dielectric * (fresnel * gloss * env + highlight[i]) + m.metallic * 0.35 * highlight[i]),
    }
}
