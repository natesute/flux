# Project file migrations

When the project file schema changes incompatibly, bump the `version` field and document the migration here.

## v1 (current)

Initial schema.

### Non-breaking additions

- **`tone_map`** field on `Project` (default `aces`). Older projects without
  this field render with ACES, which is more saturated and brighter than the
  pre-tone-map renderer. To get the old look, set `tone_map: "reinhard"` in
  your project file or pass `--tone-map reinhard` to `flux render`.
