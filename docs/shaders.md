# Raw WGSL shaders

Raw shaders are the escape hatch for multi-pass work or extra textures. They receive the same old/new wallpaper transition resources and uniform contract used by `wallr-core/shaders/effects.wgsl`. Prefer `custom_effects` first: it validates a smaller sandbox and keeps packages portable.
