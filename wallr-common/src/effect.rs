use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Fade(FadeParams),
    Wipe(WipeParams),
    Slide(SlideParams),
    Wave(WaveParams),
    Grow(GrowParams),
    Outer(OuterParams),
}

impl<'de> Deserialize<'de> for Effect {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EffectVisitor;

        impl<'de> serde::de::Visitor<'de> for EffectVisitor {
            type Value = Effect;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or a map representing an effect")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "fade" => Ok(Effect::Fade(FadeParams::default())),
                    "wipe" => Ok(Effect::Wipe(WipeParams::default())),
                    "slide" => Ok(Effect::Slide(SlideParams::default())),
                    "wave" => Ok(Effect::Wave(WaveParams::default())),
                    "grow" => Ok(Effect::Grow(GrowParams::default())),
                    "outer" => Ok(Effect::Outer(OuterParams::default())),
                    _ => Err(E::custom(format!("unknown effect type: {}", value))),
                }
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| serde::de::Error::custom("expected a key in Effect map"))?;

                match key.as_str() {
                    "fade" => Ok(Effect::Fade(map.next_value()?)),
                    "wipe" => Ok(Effect::Wipe(map.next_value()?)),
                    "slide" => Ok(Effect::Slide(map.next_value()?)),
                    "wave" => Ok(Effect::Wave(map.next_value()?)),
                    "grow" => Ok(Effect::Grow(map.next_value()?)),
                    "outer" => Ok(Effect::Outer(map.next_value()?)),
                    _ => Err(serde::de::Error::custom(format!(
                        "unknown effect type: {}",
                        key
                    ))),
                }
            }
        }

        deserializer.deserialize_any(EffectVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FadeParams {
    #[serde(default)]
    pub from: f32,
    #[serde(default = "default_one")]
    pub to: f32,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for FadeParams {
    fn default() -> Self {
        Self {
            from: 0.0,
            to: 1.0,
            easing: Easing::Bezier,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WipeParams {
    #[serde(default)]
    pub direction: WipeDirection,
    #[serde(default = "default_wipe_softness")]
    pub softness: f32,
    #[serde(default)]
    pub angle: Option<f32>,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for WipeParams {
    fn default() -> Self {
        Self {
            direction: WipeDirection::Left,
            softness: 0.006,
            angle: None,
            easing: Easing::Bezier,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WipeDirection {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlideParams {
    #[serde(default)]
    pub direction: SlideDirection,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for SlideParams {
    fn default() -> Self {
        Self {
            direction: SlideDirection::Left,
            easing: Easing::Bezier,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SlideDirection {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    Center,
    Cursor,
    Custom(f32, f32),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WaveParams {
    #[serde(default = "default_wave_frequency")]
    pub frequency: f32,
    #[serde(default = "default_wave_amplitude")]
    pub amplitude: f32,
    #[serde(default)]
    pub angle: Option<f32>,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for WaveParams {
    fn default() -> Self {
        Self {
            frequency: 3.0,
            amplitude: 0.05,
            angle: None,
            easing: Easing::Bezier,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GrowParams {
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for GrowParams {
    fn default() -> Self {
        Self {
            origin: Origin::Center,
            easing: Easing::Bezier,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OuterParams {
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for OuterParams {
    fn default() -> Self {
        Self {
            origin: Origin::Center,
            easing: Easing::Bezier,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    #[serde(alias = "ease-in")]
    EaseIn,
    #[serde(alias = "ease-out")]
    EaseOut,
    #[serde(alias = "ease-in-out")]
    #[default]
    EaseInOut,
    Emphatic,
    Spring,
    /// Cubic-bezier(.54, 0, .34, .99): awww's default transition curve.
    /// Evaluated on the CPU once per frame and sent with linear passthrough,
    /// since the shader only implements fixed curves.
    Bezier,
}

fn default_one() -> f32 {
    1.0
}

fn default_wipe_softness() -> f32 {
    0.006
}
fn default_wave_frequency() -> f32 {
    3.0
}
fn default_wave_amplitude() -> f32 {
    0.05
}

/// Cubic-bezier y(x) with awww's default control points, in the style of
/// gre/bezier-easing (the same family awww ports): Newton-Raphson with a
/// bisection fallback.
fn cubic_bezier_y(x: f32) -> f32 {
    const X1: f32 = 0.54;
    const Y1: f32 = 0.0;
    const X2: f32 = 0.34;
    const Y2: f32 = 0.99;
    fn sample_curve_x(t: f32) -> f32 {
        ((1.0 - 3.0 * X2 + 3.0 * X1) * t + (3.0 * X2 - 6.0 * X1)) * t * t + 3.0 * X1 * t
    }
    fn sample_curve_y(t: f32) -> f32 {
        ((1.0 - 3.0 * Y2 + 3.0 * Y1) * t + (3.0 * Y2 - 6.0 * Y1)) * t * t + 3.0 * Y1 * t
    }
    fn sample_slope_x(t: f32) -> f32 {
        3.0 * (1.0 - 3.0 * X2 + 3.0 * X1) * t * t + 2.0 * (3.0 * X2 - 6.0 * X1) * t + 3.0 * X1
    }
    let mut t = x.clamp(0.0, 1.0);
    for _ in 0..5 {
        let slope = sample_slope_x(t);
        if slope.abs() < 1e-4 {
            break;
        }
        t -= (sample_curve_x(t) - x) / slope;
    }
    if !(0.0..=1.0).contains(&t) {
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        t = x;
        while hi - lo > 1e-4 {
            if sample_curve_x(t) < x {
                lo = t;
            } else {
                hi = t;
            }
            t = (lo + hi) / 2.0;
        }
    }
    sample_curve_y(t.clamp(0.0, 1.0))
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EffectUniforms {
    pub effect_type: u32,
    pub progress: f32,
    pub param_a: f32,
    pub param_b: f32,
    pub param_c: f32,
    pub param_d: f32,
    pub origin: [f32; 2],
    pub direction: [f32; 2],
    pub easing: u32,
}

pub fn compute_effect_uniforms(effect: &Effect, progress: f32) -> EffectUniforms {
    let progress = progress.clamp(0.0, 1.0);
    let easing_index = |e: &Easing| match e {
        Easing::Linear | Easing::Bezier => 0,
        Easing::EaseIn => 1,
        Easing::EaseOut => 2,
        Easing::EaseInOut => 3,
        Easing::Emphatic => 4,
        Easing::Spring => 5,
    };
    // Bezier is pre-evaluated here (once per frame) and sent with linear
    // passthrough; the shader only implements fixed curves.
    let eased = |easing: &Easing| match easing {
        Easing::Bezier => cubic_bezier_y(progress),
        _ => progress,
    };
    match effect {
        Effect::Fade(params) => EffectUniforms {
            effect_type: 0,
            progress: eased(&params.easing),
            param_a: params.from,
            param_b: params.to,
            param_c: 0.0,
            param_d: 0.0,
            origin: [0.5, 0.5],
            direction: [0.0, 0.0],
            easing: easing_index(&params.easing),
        },
        Effect::Wipe(params) => {
            let (dir_vec, origin) = if let Some(angle_deg) = params.angle {
                let rad = angle_deg.to_radians();
                (
                    [rad.cos(), rad.sin()],
                    [0.5 + 0.5 * rad.cos(), 0.5 - 0.5 * rad.sin()],
                )
            } else {
                match params.direction {
                    WipeDirection::Left => ([-1.0, 0.0], [0.0, 0.5]),
                    WipeDirection::Right => ([1.0, 0.0], [1.0, 0.5]),
                    WipeDirection::Up => ([0.0, 1.0], [0.5, 0.0]),
                    WipeDirection::Down => ([0.0, -1.0], [0.5, 1.0]),
                }
            };
            EffectUniforms {
                effect_type: 1,
                progress: eased(&params.easing),
                param_a: params.softness,
                param_b: 0.0,
                param_c: 0.0,
                param_d: 0.0,
                origin,
                direction: dir_vec,
                easing: easing_index(&params.easing),
            }
        }
        Effect::Slide(params) => {
            let (dir_vec, origin) = match params.direction {
                SlideDirection::Left => ([-1.0, 0.0], [0.0, 0.5]),
                SlideDirection::Right => ([1.0, 0.0], [1.0, 0.5]),
                SlideDirection::Up => ([0.0, 1.0], [0.5, 0.0]),
                SlideDirection::Down => ([0.0, -1.0], [0.5, 1.0]),
            };
            EffectUniforms {
                effect_type: 2,
                progress: eased(&params.easing),
                param_a: 0.0,
                param_b: 0.0,
                param_c: 0.0,
                param_d: 0.0,
                origin,
                direction: dir_vec,
                easing: easing_index(&params.easing),
            }
        }
        Effect::Wave(params) => {
            let (dir_vec, origin) = if let Some(angle_deg) = params.angle {
                let rad = angle_deg.to_radians();
                (
                    [rad.cos(), rad.sin()],
                    [0.5 + 0.5 * rad.cos(), 0.5 - 0.5 * rad.sin()],
                )
            } else {
                ([0.0, 0.0], [0.5, 0.5])
            };
            EffectUniforms {
                effect_type: 3,
                progress: eased(&params.easing),
                param_a: params.frequency,
                param_b: params.amplitude,
                param_c: 0.0,
                param_d: 0.0,
                origin,
                direction: dir_vec,
                easing: easing_index(&params.easing),
            }
        }
        Effect::Grow(params) => {
            let orig = match params.origin {
                Origin::Center | Origin::Cursor => [0.5, 0.5],
                Origin::Custom(x, y) => [x, y],
            };
            EffectUniforms {
                effect_type: 4,
                progress: eased(&params.easing),
                param_a: 0.0,
                param_b: 0.0,
                param_c: 0.0,
                param_d: 0.0,
                origin: orig,
                direction: [0.0, 0.0],
                easing: easing_index(&params.easing),
            }
        }
        Effect::Outer(params) => {
            let orig = match params.origin {
                Origin::Center | Origin::Cursor => [0.5, 0.5],
                Origin::Custom(x, y) => [x, y],
            };
            EffectUniforms {
                effect_type: 5,
                progress: eased(&params.easing),
                param_a: 0.0,
                param_b: 0.0,
                param_c: 0.0,
                param_d: 0.0,
                origin: orig,
                direction: [0.0, 0.0],
                easing: easing_index(&params.easing),
            }
        }
    }
}

/// Parse an effect name into its default `Effect`.
/// Accepts Wallr names plus the directional aliases familiar from awww:
/// `simple`, `left`, `right`, `top`, `bottom`, `center`, `any`, and `random`.
pub fn effect_from_name(name: &str) -> Option<Effect> {
    let seed = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    };
    Some(match name {
        // `simple` is a plain linear crossfade, matching awww's constant
        // byte-step feel; `fade` keeps the eased (bezier-like) polish.
        "simple" => Effect::Fade(FadeParams {
            easing: Easing::Linear,
            ..FadeParams::default()
        }),
        "fade" => Effect::Fade(FadeParams::default()),
        "wipe" => Effect::Wipe(WipeParams::default()),
        "slide" => Effect::Slide(SlideParams::default()),
        "left" => Effect::Slide(SlideParams {
            direction: SlideDirection::Left,
            ..SlideParams::default()
        }),
        "right" => Effect::Slide(SlideParams {
            direction: SlideDirection::Right,
            ..SlideParams::default()
        }),
        "top" => Effect::Slide(SlideParams {
            direction: SlideDirection::Up,
            ..SlideParams::default()
        }),
        "bottom" => Effect::Slide(SlideParams {
            direction: SlideDirection::Down,
            ..SlideParams::default()
        }),
        "wave" => Effect::Wave(WaveParams::default()),
        "grow" => Effect::Grow(GrowParams::default()),
        "center" => Effect::Grow(GrowParams::default()),
        "outer" => Effect::Outer(OuterParams::default()),
        "any" => {
            let value = seed();
            let origin = Origin::Custom(
                (value % 1000) as f32 / 1000.0,
                ((value / 1000) % 1000) as f32 / 1000.0,
            );
            if value % 2 == 0 {
                Effect::Grow(GrowParams {
                    origin,
                    ..GrowParams::default()
                })
            } else {
                Effect::Outer(OuterParams {
                    origin,
                    ..OuterParams::default()
                })
            }
        }
        "random" => match seed() % 5 {
            0 => Effect::Fade(FadeParams::default()),
            1 => Effect::Slide(SlideParams::default()),
            2 => Effect::Wave(WaveParams::default()),
            3 => Effect::Grow(GrowParams::default()),
            _ => Effect::Outer(OuterParams::default()),
        },
        _ => return None,
    })
}

pub fn effect_names() -> &'static [&'static str] {
    &[
        "simple", "fade", "wipe", "slide", "left", "right", "top", "bottom", "wave", "grow",
        "center", "outer", "any", "random",
    ]
}

#[derive(Debug, Clone, Default)]
pub struct EffectOverrides {
    pub origin: Option<(f32, f32)>,
    pub origin_preset: Option<String>,
    pub direction: Option<[f32; 2]>,
    pub angle: Option<f32>,
    pub easing: Option<Easing>,
    pub from: Option<f32>,
    pub to: Option<f32>,
    pub frequency: Option<f32>,
    pub amplitude: Option<f32>,
    pub softness: Option<f32>,
}

fn origin_from_preset(preset: &str) -> (f32, f32) {
    match preset {
        "top_left" => (0.0, 0.0),
        "top" => (0.5, 0.0),
        "top_right" => (1.0, 0.0),
        "left" => (0.0, 0.5),
        "center" => (0.5, 0.5),
        "right" => (1.0, 0.5),
        "bottom_left" => (0.0, 1.0),
        "bottom" => (0.5, 1.0),
        "bottom_right" => (1.0, 1.0),
        _ => (0.5, 0.5),
    }
}

/// Apply CLI-style overrides to an effect, mutating it in place.
pub fn apply_effect_overrides(effect: &mut Effect, o: &EffectOverrides) {
    let origin = o
        .origin
        .or_else(|| o.origin_preset.as_ref().map(|p| origin_from_preset(p)));

    match effect {
        Effect::Fade(p) => {
            if let Some(v) = o.from {
                p.from = v;
            }
            if let Some(v) = o.to {
                p.to = v;
            }
            if let Some(e) = o.easing {
                p.easing = e;
            }
        }
        Effect::Wipe(p) => {
            if let Some(s) = o.softness {
                p.softness = s;
            }
            if let Some(a) = o.angle {
                p.angle = Some(a);
            }
            if let Some(e) = o.easing {
                p.easing = e;
            }
            if let Some(d) = o.direction {
                p.direction = match d {
                    [-1.0, 0.0] => WipeDirection::Left,
                    [1.0, 0.0] => WipeDirection::Right,
                    [0.0, 1.0] => WipeDirection::Up,
                    [0.0, -1.0] => WipeDirection::Down,
                    _ => p.direction,
                };
            }
        }
        Effect::Slide(p) => {
            if let Some(e) = o.easing {
                p.easing = e;
            }
            if let Some(d) = o.direction {
                p.direction = match d {
                    [-1.0, 0.0] => SlideDirection::Left,
                    [1.0, 0.0] => SlideDirection::Right,
                    [0.0, 1.0] => SlideDirection::Up,
                    [0.0, -1.0] => SlideDirection::Down,
                    _ => p.direction,
                };
            }
        }
        Effect::Wave(p) => {
            if let Some(v) = o.frequency {
                p.frequency = v;
            }
            if let Some(v) = o.amplitude {
                p.amplitude = v;
            }
            if let Some(a) = o.angle {
                p.angle = Some(a);
            }
            if let Some(e) = o.easing {
                p.easing = e;
            }
        }
        Effect::Grow(p) => {
            if let Some((x, y)) = origin {
                p.origin = Origin::Custom(x, y);
            }
            if let Some(e) = o.easing {
                p.easing = e;
            }
        }
        Effect::Outer(p) => {
            if let Some((x, y)) = origin {
                p.origin = Origin::Custom(x, y);
            }
            if let Some(e) = o.easing {
                p.easing = e;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranged_effects_keep_both_endpoints_for_the_shader() {
        let fade = Effect::Fade(FadeParams {
            from: 0.2,
            to: 0.9,
            easing: Easing::EaseOut,
        });
        let start = compute_effect_uniforms(&fade, 0.0);
        let end = compute_effect_uniforms(&fade, 1.0);
        assert_eq!((start.param_a, start.param_b), (0.2, 0.9));
        assert_eq!((end.param_a, end.param_b), (0.2, 0.9));
    }

    #[test]
    fn awww_direction_aliases_resolve_to_typed_effects() {
        assert!(matches!(effect_from_name("simple"), Some(Effect::Fade(_))));
        assert!(matches!(effect_from_name("left"), Some(Effect::Slide(_))));
        assert!(matches!(effect_from_name("right"), Some(Effect::Slide(_))));
        assert!(matches!(effect_from_name("center"), Some(Effect::Grow(_))));
        assert!(matches!(
            effect_from_name("any"),
            Some(Effect::Grow(_)) | Some(Effect::Outer(_))
        ));
        assert!(effect_from_name("random").is_some());
    }

    #[test]
    fn effect_types_match_shader_arms() {
        // Must track effects.wgsl: 0 fade, 1 wipe, 2 slide, 3 wave, 4 grow, 5 outer.
        let cases = [
            (Effect::Fade(FadeParams::default()), 0),
            (Effect::Wipe(WipeParams::default()), 1),
            (Effect::Slide(SlideParams::default()), 2),
            (Effect::Wave(WaveParams::default()), 3),
            (Effect::Grow(GrowParams::default()), 4),
            (Effect::Outer(OuterParams::default()), 5),
        ];
        for (effect, expected) in cases {
            assert_eq!(compute_effect_uniforms(&effect, 0.5).effect_type, expected);
        }
    }

    #[test]
    fn removed_effects_no_longer_resolve() {
        for name in ["blur", "zoom", "pixelate", "ripple", "dissolve", "shader"] {
            assert_eq!(effect_from_name(name), None);
        }
    }

    #[test]
    fn bezier_easing_matches_awww_curve() {
        assert_eq!(cubic_bezier_y(0.0), 0.0);
        assert_eq!(cubic_bezier_y(1.0), 1.0);
        let mut prev = 0.0;
        let mut i = 1;
        while i <= 20 {
            let y = cubic_bezier_y(i as f32 / 20.0);
            assert!(y >= prev, "bezier must not decrease");
            prev = y;
            i += 1;
        }
        // awww's (.54, 0, .34, .99): slow start, fast middle, soft landing.
        let mid = cubic_bezier_y(0.5);
        assert!((0.4..0.7).contains(&mid), "mid: {mid}");
        assert!(cubic_bezier_y(0.25) < 0.25);
    }
}
