pub const EFFECTS_SHADER: &str = include_str!("../../shaders/effects.wgsl");
pub const NV12_TO_RGB_SHADER: &str = include_str!("../../shaders/nv12_to_rgb.wgsl");

#[cfg(test)]
mod tests {
    use super::{EFFECTS_SHADER, NV12_TO_RGB_SHADER};

    fn validate(source: &str) {
        let module = naga::front::wgsl::parse_str(source).expect("bundled WGSL should parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("bundled WGSL should validate");
    }

    #[test]
    fn bundled_effect_shader_validates() {
        validate(EFFECTS_SHADER);
    }

    #[test]
    fn bundled_nv12_shader_validates() {
        validate(NV12_TO_RGB_SHADER);
    }
}
