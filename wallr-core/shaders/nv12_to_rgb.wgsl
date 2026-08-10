struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct Conversion {
    range: vec4<f32>,
    red: vec4<f32>,
    green: vec4<f32>,
    blue: vec4<f32>,
};

@group(0) @binding(0) var luma_texture: texture_2d<f32>;
@group(0) @binding(1) var chroma_texture: texture_2d<f32>;
@group(0) @binding(2) var plane_sampler: sampler;
@group(0) @binding(3) var<uniform> conversion: Conversion;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let u = f32((vertex_index << 1u) & 2u);
    let v = f32(vertex_index & 2u);
    out.uv = vec2<f32>(u, 1.0 - v);
    out.clip_position = vec4<f32>(u * 2.0 - 1.0, v * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

fn sdr_to_linear(encoded: f32) -> f32 {
    let value = clamp(encoded, 0.0, 1.0);
    if value < 0.08145 {
        return value / 4.5;
    }
    return pow((value + 0.0993) / 1.0993, 1.0 / 0.45);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_sample = textureSample(luma_texture, plane_sampler, in.uv).r;
    let uv_sample = textureSample(chroma_texture, plane_sampler, in.uv).rg;
    let y = (y_sample - conversion.range.x) * conversion.range.y;
    let cbcr = (uv_sample - vec2<f32>(conversion.range.z)) * conversion.range.w;
    let yuv = vec4<f32>(y, cbcr.x, cbcr.y, 1.0);
    let encoded = vec3<f32>(
        dot(conversion.red, yuv),
        dot(conversion.green, yuv),
        dot(conversion.blue, yuv),
    );

    // The sRGB render target applies its encoding after this linear output.
    return vec4<f32>(
        sdr_to_linear(encoded.r),
        sdr_to_linear(encoded.g),
        sdr_to_linear(encoded.b),
        1.0,
    );
}
