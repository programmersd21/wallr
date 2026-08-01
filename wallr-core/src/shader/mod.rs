pub const EFFECTS_SHADER: &str = include_str!("../../../shaders/effects.wgsl");

#[cfg(test)]
mod tests {
    use super::EFFECTS_SHADER;

    #[test]
    fn bundled_effect_shader_validates() {
        let module =
            naga::front::wgsl::parse_str(EFFECTS_SHADER).expect("bundled WGSL should parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("bundled WGSL should validate");
    }
}
