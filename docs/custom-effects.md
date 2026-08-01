# Custom effects

Use `custom_effects` when composition of built-ins is not enough. The field is a sandboxed, WGSL-shaped expression with no loops, file access, network access, or user-defined functions.

```yaml
custom_effects:
  scanline:
    params: {strength: 0.08}
    field: |
      line = sin(uv.y * resolution.y * 0.5) * strength
      return mix(old, new, clamp(t + line, 0.0, 1.0))
```

Available context values include `t`, `uv`, `resolution`, `old`, `new`, and `time_absolute`. Built-ins include arithmetic, `mix`, `clamp`, `floor`, `length`, `hash`, `noise`, `smoothstep`, trigonometry, and vector helpers. Use raw `shader: {file, uniforms}` only for multi-pass or otherwise inexpressible effects.
