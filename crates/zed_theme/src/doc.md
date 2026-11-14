# zed_theme

Converts a Zed theme family file into gpui-component's theme set JSON, so an imported Zed
theme works as a full UI theme and as a syntax palette. Pure and GPUI-free: text in, text
out, no file I/O.

Not here: finding Zed's folders and writing the result into the themes folder
(`ui_resources/src/themes.rs`), choosing a syntax palette (`slopcop/src/settings_store.rs`).

## Files

- `lib.rs` — `convert(source, is_taken)` → `Converted` (family, `file_stem`, theme names and
  appearance, the JSON); the UI color map `UI_COLORS`, the copied highlight keys, and the
  `color` / `font_style` / `font_weight` normalizers.
- `jsonc.rs` — `strip`: removes `//` and `/* */` comments and trailing commas outside
  strings, for hand-written theme files.
- `tests.rs` — a family with dark and light themes, name collisions, JSONC, rejected files,
  normalizer cases.

## Spec

- Input is Zed's format: `themes[]` with `name`, `appearance` and a `style` object. A theme
  without `style` is skipped; no themes left is an error. Missing `appearance` counts as dark.
- Output is gpui-component's `ThemeSet`: `name`, `author`, `themes[]` with `name`, `mode`,
  `colors` (UI keys, each taken from the first Zed key in its list that has a valid color)
  and `highlight` (the editor and status keys Zed and gpui-component share, plus `syntax`).
- Colors become `#rrggbbaa`; anything else (null, names, bad hex) is dropped, because one
  value gpui-component cannot parse makes it reject the whole file. `font_style` maps
  `oblique` to `italic`; `font_weight` snaps to the nearest hundred in 100–900, the only
  values gpui-component accepts. A capture with nothing valid left is dropped; unknown
  capture names are kept (gpui-component ignores them).
- Theme names must be unique across the registry, which keeps the first theme of a name it
  reads: a name `is_taken` reports gets " (Zed)", then " (Zed 2)", … Names repeated inside
  the file are de-duplicated the same way.
- `file_stem` is `zed-` plus a slug of the family name (lowercased letters and digits of
  any script, other runs become `-`), so an import never overwrites a bundled theme file.
- Strict JSON is tried first; the comment-stripping parse is the fallback, and its error is
  not reported (the strict parser's is clearer).

## Decisions

- Import converts once into gpui-component's format instead of reading Zed files at
  startup: the registry only understands its own format, and a converted file keeps
  working if the Zed extension is removed.
