struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var t_diffuse1: texture_2d<f32>;
@group(0) @binding(1) var s_diffuse1: sampler;
@group(1) @binding(0) var t_diffuse2: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse2: sampler;

struct Uniforms {
    time: f32,
    progress: f32, // 0.0 to 1.0
    effect_type: u32,
    padding: u32,
    resolution: vec2<f32>,
    image_resolution: vec2<f32>,
    old_image_resolution: vec2<f32>,
    param_a: f32,
    param_b: f32,
    param_c: f32,
    param_d: f32,
    origin: vec2<f32>,
    direction: vec2<f32>,
    easing: u32,
    padding2: u32,
};

@group(2) @binding(0) var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let u = f32((vertex_index << 1u) & 2u);
    let v = f32(vertex_index & 2u);
    out.uv = vec2<f32>(u, 1.0 - v);
    out.clip_position = vec4<f32>(u * 2.0 - 1.0, v * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// Canonical smoothstep cubic ease-in-out. Its tail keeps visible motion
// almost to the end (progress is 0.972 at t = 0.9), so a blur radius or
// crossfade never freezes for a noticeable stretch before the transition
// finishes. A 2t^3 / 1 - 2(1-t)^3 piecewise cubic saturates at 0.996 by
// t = 0.9, which makes effects appear to stagger to a halt.
fn ease_in_out(t: f32) -> f32 {
    return t * t * (3.0 - 2.0 * t);
}

fn ease_in(t: f32) -> f32 {
    return t * t * t;
}

fn ease_out(t: f32) -> f32 {
    let u = 1.0 - t;
    return 1.0 - u * u * u;
}

// 0 = linear, 1 = ease_in, 2 = ease_out, 3 = ease_in_out
fn apply_easing(t: f32, mode: u32) -> f32 {
    if mode == 0u {
        return t;
    }
    if mode == 1u {
        return ease_in(t);
    }
    if mode == 2u {
        return ease_out(t);
    }
    if mode == 4u {
        // Back-out curve: starts at zero, settles at one, with restrained overshoot.
        let u = t - 1.0;
        let c1 = 1.70158;
        let c3 = c1 + 1.0;
        return 1.0 + c3 * u * u * u + c1 * u * u;
    }
    if mode == 5u {
        let e = exp(-5.0 * t);
        return 1.0 - e * cos(12.0 * t);
    }
    return ease_in_out(t);
}

fn cover_uv(uv: vec2<f32>, image_resolution: vec2<f32>, screen_resolution: vec2<f32>) -> vec2<f32> {
    let screen_ratio = screen_resolution.x / max(screen_resolution.y, 1.0);
    let image_ratio = image_resolution.x / max(image_resolution.y, 1.0);
    var result = uv;
    if (screen_ratio > image_ratio) {
        result.y = (uv.y - 0.5) * (image_ratio / screen_ratio) + 0.5;
    } else {
        result.x = (uv.x - 0.5) * (screen_ratio / image_ratio) + 0.5;
    }
    return result;
}

// Quintic interpolation keeps the edge velocity at zero at both ends. It is
// less mechanical than smoothstep for a large, visible wipe feather.
fn smootherstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let width = max(edge1 - edge0, 0.0001);
    let t = clamp((value - edge0) / width, 0.0, 1.0);
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn circular_distance(uv: vec2<f32>, origin: vec2<f32>, resolution: vec2<f32>) -> f32 {
    let aspect = resolution.x / max(resolution.y, 1.0);
    return distance(vec2<f32>(uv.x * aspect, uv.y), vec2<f32>(origin.x * aspect, origin.y));
}

fn circular_max_radius(origin: vec2<f32>, resolution: vec2<f32>) -> f32 {
    let aspect = resolution.x / max(resolution.y, 1.0);
    let far = vec2<f32>(
        select(0.0, aspect, origin.x < 0.5),
        select(0.0, 1.0, origin.y < 0.5)
    );
    return distance(far, vec2<f32>(origin.x * aspect, origin.y));
}

// The shared mask for Wallr's reveal effects. Its geometry is always a true
// screen-space circle; `edge_offset` only ripples the boundary, never the
// source wallpaper coordinates. The radius overshoots the farthest corner by
// the feather width so the reveal completes exactly at progress 1.0: without
// it the far corner stays mid-feather (half blended) at the end of the eased
// range and only the endpoint frame snaps it to the new image, which reads as
// a staggered pop at the end of a wipe.
fn circular_reveal(
    uv: vec2<f32>,
    origin: vec2<f32>,
    resolution: vec2<f32>,
    progress: f32,
    feather: f32,
    edge_offset: f32,
) -> f32 {
    let max = circular_max_radius(origin, resolution);
    let radius = progress * (max + feather) + edge_offset;
    let distance_from_origin = circular_distance(uv, origin, resolution);
    return 1.0 - smootherstep(radius - feather, radius + feather, distance_from_origin);
}

fn circular_outer_reveal(
    uv: vec2<f32>,
    origin: vec2<f32>,
    resolution: vec2<f32>,
    progress: f32,
    feather: f32,
) -> f32 {
    let radius = (1.0 - progress) * circular_max_radius(origin, resolution);
    let distance_from_origin = circular_distance(uv, origin, resolution);
    return smootherstep(radius - feather, radius + feather, distance_from_origin);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let p = apply_easing(clamp(uniforms.progress, 0.0, 1.0), uniforms.easing);
    
    let screen_ratio = uniforms.resolution.x / max(uniforms.resolution.y, 1.0);
    // Keep the two source images in their own cover rectangles. Reusing the
    // incoming image's crop for the outgoing image causes visible geometry
    // shifts whenever consecutive wallpapers have different aspect ratios.
    let uv_old = cover_uv(uv, uniforms.old_image_resolution, uniforms.resolution);
    let uv_new = cover_uv(uv, uniforms.image_resolution, uniforms.resolution);

    // Exact endpoint frames prevent a soft mask from leaving a one-pixel seam
    // at an edge or corner after the daemon promotes the incoming texture.
    if (uniforms.progress <= 0.0) {
        return textureSample(t_diffuse1, s_diffuse1, uv_old);
    }
    if (uniforms.progress >= 1.0) {
        return textureSample(t_diffuse2, s_diffuse2, uv_new);
    }

    // 0: Fade (opacity = `from` -> `to`, eased)
    if (uniforms.effect_type == 0u) {
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        let opacity = mix(uniforms.param_a, uniforms.param_b, p);
        return mix(color1, color2, clamp(opacity, 0.0, 1.0));
    } 
    
    // 1: Multi-sample Radial Blur (swww style, radius = `from` -> `to`, eased)
    else if (uniforms.effect_type == 1u) {
        var c1 = vec4<f32>(0.0);
        var c2 = vec4<f32>(0.0);
        let blur_radius = max(mix(uniforms.param_a, uniforms.param_b, p), 0.0);
        let max_blur = blur_radius / max(min(uniforms.resolution.x, uniforms.resolution.y), 1.0);
        let blur_amount = max_blur;
        
        let samples = 8;
        let step_rad = 6.28318 / f32(samples);
        
        for (var i = 0; i < 8; i = i + 1) {
            let angle = f32(i) * step_rad;
            let offset = vec2<f32>(cos(angle) / max(screen_ratio, 0.001), sin(angle)) * blur_amount;
            c1 = c1 + textureSample(t_diffuse1, s_diffuse1, uv_old + offset);
            c2 = c2 + textureSample(t_diffuse2, s_diffuse2, uv_new + offset);
        }
        
        c1 = c1 / 8.0;
        c2 = c2 / 8.0;
        return mix(c1, c2, p);
    }
    
    // 2: Soft circular wipe. The legacy name remains package-compatible, but
    // the reveal is radial to avoid a hard bar or diagonal wedge.
    else if (uniforms.effect_type == 2u) {
        let softness = clamp(max(uniforms.param_a, 0.09), 0.09, 0.22);
        let reveal = circular_reveal(uv, uniforms.origin, uniforms.resolution, p, softness, 0.0);
        
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        return mix(color1, color2, reveal);
    }
    
    // 3: Circular reveal (legacy slide alias). No texture translation.
    else if (uniforms.effect_type == 3u) {
        let reveal = circular_reveal(uv, uniforms.origin, uniforms.resolution, p, 0.065, 0.0);
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        return mix(color1, color2, reveal);
    }
    
    // 4: Stationary focus crossfade. `zoom` intentionally keeps source UVs
    // fixed; moving the full wallpaper reads as a slideshow, not a premium
    // desktop transition.
    else if (uniforms.effect_type == 4u) {
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        return mix(color1, color2, p);
    }
    
    // 5: Circular focus reveal (legacy pixelate alias). Avoids a grid that
    // reads as a slideshow artifact while keeping images perfectly registered.
    else if (uniforms.effect_type == 5u) {
        let reveal = circular_reveal(uv, uniforms.origin, uniforms.resolution, p, 0.09, 0.0);
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        return mix(color1, color2, reveal);
    }
    
    // 6: Expanding liquid reveal. The ripple modulates the mask, never the
    // source UVs, so landmarks in both wallpapers cannot drift or wobble.
    else if (uniforms.effect_type == 6u) {
        let origin = uniforms.origin;
        let dist = circular_distance(uv, origin, uniforms.resolution);
        
        let freq = max(uniforms.param_a, 0.1);
        let amp = max(uniforms.param_b, 0.0);
        let speed = max(uniforms.param_c, 0.0);
        
        let ripple = sin(dist * freq * 6.28318 - p * speed * 6.28318) * amp * sin(p * 3.14159);
        let reveal = circular_reveal(uv, origin, uniforms.resolution, p, 0.045, ripple);
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        return mix(color1, color2, reveal);
    }
    
    // 7: Concentric dissolve. It preserves the fixed circular language rather
    // than revealing random square noise cells.
    else if (uniforms.effect_type == 7u) {
        let scale = uniforms.param_a;
        let softness = max(uniforms.param_b, 0.001);
        let dist = circular_distance(uv, uniforms.origin, uniforms.resolution);
        let rings = sin(dist * max(scale, 1.0) * 6.28318) * softness * 0.5;
        let reveal = circular_reveal(uv, uniforms.origin, uniforms.resolution, p, max(softness, 0.06), rings);
        
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        
        return mix(color1, color2, reveal);
    }
    
    // 8: Custom shader file
    else if (uniforms.effect_type == 8u) {
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        return mix(color1, color2, p);
    }
    
    // 9: Circular wave. The oscillation lives only on the radial boundary.
    else if (uniforms.effect_type == 9u) {
        let wave_freq = max(uniforms.param_a, 0.1);
        let wave_amp = max(uniforms.param_b, 0.0);
        let dist = circular_distance(uv, uniforms.origin, uniforms.resolution);
        let wave = sin(dist * wave_freq * 6.28318 - p * 6.28318) * wave_amp * sin(p * 3.14159);
        let edge = circular_reveal(uv, uniforms.origin, uniforms.resolution, p, 0.06, wave);
        
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        return mix(color1, color2, edge);
    }
    
    // 10: Grow (expanding circle from origin, swww-style, true circle on screen)
    else if (uniforms.effect_type == 10u) {
        let origin = uniforms.origin;
        let edge = circular_reveal(uv, origin, uniforms.resolution, p, 0.045, 0.0);
        
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        // Inside the growing circle = new image
        return mix(color1, color2, edge);
    }
    
    // 11: Outer (shrinking circle, swww-style, true circle on screen)
    else if (uniforms.effect_type == 11u) {
        let origin = uniforms.origin;
        let edge = circular_outer_reveal(uv, origin, uniforms.resolution, p, 0.045);
        
        let color1 = textureSample(t_diffuse1, s_diffuse1, uv_old);
        let color2 = textureSample(t_diffuse2, s_diffuse2, uv_new);
        // Outside the shrinking circle = new image
        return mix(color1, color2, edge);
    }
    
    return textureSample(t_diffuse1, s_diffuse1, uv_old);
}
