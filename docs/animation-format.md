# Animation format

Every package has a `name` and an explicit `duration`. Effects may be shorthand strings or typed maps.

```yaml
name: minimal
duration: 2000ms
effects:
  - fade: {easing: ease_out}
  - blur: {from: 20, to: 0, easing: ease_in_out}
```

For overlapping steps use `timeline`; each step accepts `at`, optional `duration`, and one typed effect. `variables` supplies package values and `extends` inherits a local or GitHub package.

Supported easing names are `linear`, `ease_in`, `ease_out`, `ease_in_out`, `emphatic`, and `spring`. Duration values accept milliseconds or fractional seconds.
