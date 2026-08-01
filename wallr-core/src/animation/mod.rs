use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnimationSpec {
    pub name: String,
    /// Total transition duration. Every publishable package must set this.
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub timeline: Option<Vec<TimelineEntry>>,
    #[serde(default)]
    pub variables: HashMap<String, f64>,
    /// Parent package references, merged from base to child.
    #[serde(default)]
    pub extends: Vec<String>,
    /// Declarative custom per-pixel effects.
    #[serde(default)]
    pub custom_effects: HashMap<String, crate::custom_effects::CustomEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub at: String,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(flatten)]
    pub effect: Effect,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Fade(FadeParams),
    Blur(BlurParams),
    Wipe(WipeParams),
    Slide(SlideParams),
    Zoom(ZoomParams),
    Pixelate(PixelateParams),
    Ripple(RippleParams),
    Dissolve(DissolveParams),
    Wave(WaveParams),
    Grow(GrowParams),
    Outer(OuterParams),
    Shader(ShaderParams),
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
                    "blur" => Ok(Effect::Blur(BlurParams::default())),
                    "wipe" => Ok(Effect::Wipe(WipeParams::default())),
                    "slide" => Ok(Effect::Slide(SlideParams::default())),
                    "zoom" => Ok(Effect::Zoom(ZoomParams::default())),
                    "pixelate" => Ok(Effect::Pixelate(PixelateParams::default())),
                    "ripple" => Ok(Effect::Ripple(RippleParams::default())),
                    "dissolve" => Ok(Effect::Dissolve(DissolveParams::default())),
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
                    "blur" => Ok(Effect::Blur(map.next_value()?)),
                    "wipe" => Ok(Effect::Wipe(map.next_value()?)),
                    "slide" => Ok(Effect::Slide(map.next_value()?)),
                    "zoom" => Ok(Effect::Zoom(map.next_value()?)),
                    "pixelate" => Ok(Effect::Pixelate(map.next_value()?)),
                    "ripple" => Ok(Effect::Ripple(map.next_value()?)),
                    "dissolve" => Ok(Effect::Dissolve(map.next_value()?)),
                    "wave" => Ok(Effect::Wave(map.next_value()?)),
                    "grow" => Ok(Effect::Grow(map.next_value()?)),
                    "outer" => Ok(Effect::Outer(map.next_value()?)),
                    "shader" => Ok(Effect::Shader(map.next_value()?)),
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
            easing: Easing::EaseInOut,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlurParams {
    #[serde(default = "default_blur_from")]
    pub from: f32,
    #[serde(default)]
    pub to: f32,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for BlurParams {
    fn default() -> Self {
        Self {
            from: 20.0,
            to: 0.0,
            easing: Easing::EaseInOut,
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
            softness: 0.12,
            angle: None,
            easing: Easing::EaseInOut,
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
            easing: Easing::EaseInOut,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ZoomParams {
    #[serde(default = "default_zoom_from")]
    pub from: f32,
    #[serde(default = "default_one")]
    pub to: f32,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for ZoomParams {
    fn default() -> Self {
        Self {
            from: 1.08,
            to: 1.0,
            origin: Origin::Center,
            easing: Easing::EaseInOut,
        }
    }
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
pub struct PixelateParams {
    #[serde(default = "default_pixelate_from")]
    pub from: f32,
    #[serde(default = "default_one")]
    pub to: f32,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for PixelateParams {
    fn default() -> Self {
        Self {
            from: 64.0,
            to: 1.0,
            easing: Easing::EaseInOut,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RippleParams {
    #[serde(default)]
    pub origin: Origin,
    #[serde(default = "default_frequency")]
    pub frequency: f32,
    #[serde(default = "default_amplitude")]
    pub amplitude: f32,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for RippleParams {
    fn default() -> Self {
        Self {
            origin: Origin::Center,
            frequency: 12.0,
            amplitude: 0.03,
            speed: 5.0,
            easing: Easing::EaseInOut,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DissolveParams {
    #[serde(default = "default_dissolve_scale")]
    pub scale: f32,
    #[serde(default = "default_softness")]
    pub softness: f32,
    #[serde(default)]
    pub easing: Easing,
}

impl Default for DissolveParams {
    fn default() -> Self {
        Self {
            scale: 4.0,
            softness: 0.05,
            easing: Easing::EaseInOut,
        }
    }
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
            easing: Easing::EaseInOut,
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
            easing: Easing::EaseInOut,
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
            easing: Easing::EaseInOut,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ShaderParams {
    pub file: String,
    #[serde(default)]
    pub uniforms: HashMap<String, f64>,
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
}

fn default_one() -> f32 {
    1.0
}
fn default_zoom_from() -> f32 {
    1.08
}
fn default_blur_from() -> f32 {
    20.0
}
fn default_softness() -> f32 {
    0.05
}

fn default_wipe_softness() -> f32 {
    0.12
}
fn default_pixelate_from() -> f32 {
    64.0
}
fn default_frequency() -> f32 {
    12.0
}
fn default_amplitude() -> f32 {
    0.03
}
fn default_speed() -> f32 {
    5.0
}
fn default_dissolve_scale() -> f32 {
    4.0
}
fn default_wave_frequency() -> f32 {
    3.0
}
fn default_wave_amplitude() -> f32 {
    0.05
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

#[derive(Debug, thiserror::Error)]
pub enum AnimationError {
    #[error("failed to read animation file: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("failed to parse animation YAML: {0}")]
    ParseError(#[from] serde_yaml::Error),
    #[error("invalid effect: {0}")]
    InvalidEffect(String),
    #[error("invalid timeline: {0}")]
    InvalidTimeline(String),
    #[error("unresolved variable: {0}")]
    UnresolvedVariable(String),
    #[error("invalid duration: {0}")]
    InvalidDuration(String),
    #[error("shader error: {0}")]
    ShaderError(String),
}

pub fn load_animation(path: &Path) -> Result<AnimationSpec, AnimationError> {
    let content = std::fs::read_to_string(path)?;
    parse_animation_yaml(&content)
}

/// Parse an animation YAML document and expand `${name}` numeric variables.
pub fn parse_animation_yaml(content: &str) -> Result<AnimationSpec, AnimationError> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(content)?;
    let variables = value
        .get("variables")
        .and_then(serde_yaml::Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    fn expand(value: &mut serde_yaml::Value, variables: &serde_yaml::Mapping) {
        match value {
            serde_yaml::Value::Mapping(map) => {
                for child in map.values_mut() {
                    expand(child, variables);
                }
            }
            serde_yaml::Value::Sequence(items) => {
                for child in items {
                    expand(child, variables);
                }
            }
            serde_yaml::Value::String(text) if text.starts_with("${") && text.ends_with('}') => {
                let key = &text[2..text.len() - 1];
                if let Some(replacement) = variables.get(serde_yaml::Value::String(key.to_string()))
                {
                    *value = replacement.clone();
                }
            }
            _ => {}
        }
    }
    expand(&mut value, &variables);
    serde_yaml::from_value(value).map_err(AnimationError::from)
}

pub fn validate_animation(spec: &AnimationSpec) -> Result<(), Vec<AnimationError>> {
    let mut errors = Vec::new();
    if spec.name.is_empty() {
        errors.push(AnimationError::InvalidEffect(
            "Animation name cannot be empty".to_string(),
        ));
    }
    if spec.duration.is_none() {
        errors.push(AnimationError::InvalidDuration(
            "duration is required; add e.g. duration: 800ms".to_string(),
        ));
    } else if let Some(duration) = &spec.duration
        && crate::config::parse_duration(duration).is_err()
    {
        errors.push(AnimationError::InvalidDuration(duration.clone()));
    }
    if let Some(timeline) = &spec.timeline {
        for entry in timeline {
            if crate::config::parse_duration(&entry.at).is_err() {
                errors.push(AnimationError::InvalidTimeline(format!(
                    "invalid at: {}",
                    entry.at
                )));
            }
            if let Some(duration) = &entry.duration
                && crate::config::parse_duration(duration).is_err()
            {
                errors.push(AnimationError::InvalidTimeline(format!(
                    "invalid duration: {duration}"
                )));
            }
        }
    }
    for (name, custom) in &spec.custom_effects {
        if let Err(error) = crate::custom_effects::transpile(name, custom) {
            errors.push(AnimationError::ShaderError(format!(
                "custom effect {name}: {error}"
            )));
        }
    }
    if spec.effects.is_empty() && spec.timeline.is_none() {
        errors.push(AnimationError::InvalidTimeline(
            "Animation must contain at least one effect or timeline entry".to_string(),
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn compute_effect_uniforms(effect: &Effect, progress: f32) -> EffectUniforms {
    let progress = progress.clamp(0.0, 1.0);
    let easing_index = |e: &Easing| match e {
        Easing::Linear => 0,
        Easing::EaseIn => 1,
        Easing::EaseOut => 2,
        Easing::EaseInOut => 3,
        Easing::Emphatic => 4,
        Easing::Spring => 5,
    };
    match effect {
        Effect::Fade(params) => EffectUniforms {
            effect_type: 0,
            progress,
            param_a: params.from,
            param_b: params.to,
            param_c: 0.0,
            param_d: 0.0,
            origin: [0.5, 0.5],
            direction: [0.0, 0.0],
            easing: easing_index(&params.easing),
        },
        Effect::Blur(params) => EffectUniforms {
            effect_type: 1,
            progress,
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
                effect_type: 2,
                progress,
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
                effect_type: 3,
                progress,
                param_a: 0.0,
                param_b: 0.0,
                param_c: 0.0,
                param_d: 0.0,
                origin,
                direction: dir_vec,
                easing: easing_index(&params.easing),
            }
        }
        Effect::Zoom(params) => {
            let orig = match params.origin {
                Origin::Center | Origin::Cursor => [0.5, 0.5],
                Origin::Custom(x, y) => [x, y],
            };
            EffectUniforms {
                effect_type: 4,
                progress,
                param_a: params.from,
                param_b: params.to,
                param_c: 0.0,
                param_d: 0.0,
                origin: orig,
                direction: [0.0, 0.0],
                easing: easing_index(&params.easing),
            }
        }
        Effect::Pixelate(params) => EffectUniforms {
            effect_type: 5,
            progress,
            param_a: params.from,
            param_b: params.to,
            param_c: 0.0,
            param_d: 0.0,
            origin: [0.5, 0.5],
            direction: [0.0, 0.0],
            easing: easing_index(&params.easing),
        },
        Effect::Ripple(params) => {
            let orig = match params.origin {
                Origin::Center | Origin::Cursor => [0.5, 0.5],
                Origin::Custom(x, y) => [x, y],
            };
            EffectUniforms {
                effect_type: 6,
                progress,
                param_a: params.frequency,
                param_b: params.amplitude,
                param_c: params.speed,
                param_d: 0.0,
                origin: orig,
                direction: [0.0, 0.0],
                easing: easing_index(&params.easing),
            }
        }
        Effect::Dissolve(params) => EffectUniforms {
            effect_type: 7,
            progress,
            param_a: params.scale,
            param_b: params.softness,
            param_c: 0.0,
            param_d: 0.0,
            origin: [0.5, 0.5],
            direction: [0.0, 0.0],
            easing: easing_index(&params.easing),
        },
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
                effect_type: 9,
                progress,
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
                effect_type: 10,
                progress,
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
                effect_type: 11,
                progress,
                param_a: 0.0,
                param_b: 0.0,
                param_c: 0.0,
                param_d: 0.0,
                origin: orig,
                direction: [0.0, 0.0],
                easing: easing_index(&params.easing),
            }
        }
        Effect::Shader(params) => EffectUniforms {
            effect_type: 8,
            progress,
            param_a: params.uniforms.get("strength").copied().unwrap_or(0.0) as f32,
            param_b: 0.0,
            param_c: 0.0,
            param_d: 0.0,
            origin: [0.5, 0.5],
            direction: [0.0, 0.0],
            easing: 3,
        },
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
        "simple" => Effect::Fade(FadeParams::default()),
        "fade" => Effect::Fade(FadeParams::default()),
        "blur" => Effect::Blur(BlurParams::default()),
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
        "zoom" => Effect::Zoom(ZoomParams::default()),
        "pixelate" => Effect::Pixelate(PixelateParams::default()),
        "ripple" => Effect::Ripple(RippleParams::default()),
        "dissolve" => Effect::Dissolve(DissolveParams::default()),
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

/// All effect names usable from the CLI / YAML.
pub fn effect_names() -> &'static [&'static str] {
    &[
        "simple", "fade", "blur", "wipe", "slide", "left", "right", "top", "bottom", "zoom",
        "pixelate", "ripple", "dissolve", "wave", "grow", "center", "outer", "any", "random",
    ]
}

/// Overrides that can be applied on top of an `Effect` from the CLI.
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
    pub speed: Option<f32>,
    pub softness: Option<f32>,
    pub scale: Option<f32>,
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
        Effect::Blur(p) => {
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
        Effect::Zoom(p) => {
            if let Some(v) = o.from {
                p.from = v;
            }
            if let Some(v) = o.to {
                p.to = v;
            }
            if let Some((x, y)) = origin {
                p.origin = Origin::Custom(x, y);
            }
            if let Some(e) = o.easing {
                p.easing = e;
            }
        }
        Effect::Pixelate(p) => {
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
        Effect::Ripple(p) => {
            if let Some(v) = o.frequency {
                p.frequency = v;
            }
            if let Some(v) = o.amplitude {
                p.amplitude = v;
            }
            if let Some(v) = o.speed {
                p.speed = v;
            }
            if let Some((x, y)) = origin {
                p.origin = Origin::Custom(x, y);
            }
            if let Some(e) = o.easing {
                p.easing = e;
            }
        }
        Effect::Dissolve(p) => {
            if let Some(v) = o.scale {
                p.scale = v;
            }
            if let Some(v) = o.softness {
                p.softness = v;
            }
            if let Some(e) = o.easing {
                p.easing = e;
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
        Effect::Shader(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_duration_is_required() {
        let spec = AnimationSpec {
            name: "missing-duration".into(),
            effects: vec![Effect::Fade(FadeParams::default())],
            ..Default::default()
        };
        assert!(validate_animation(&spec).is_err());
    }

    #[test]
    fn numeric_variables_are_expanded_before_deserialization() {
        let spec = parse_animation_yaml("name: vars\nduration: 1s\nvariables: {amount: 12}\neffects:\n  - blur: {from: \"${amount}\", to: 0}\n").expect("variable package should parse");
        match &spec.effects[0] {
            Effect::Blur(params) => assert_eq!(params.from, 12.0),
            _ => panic!("expected blur"),
        }
    }

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
}
