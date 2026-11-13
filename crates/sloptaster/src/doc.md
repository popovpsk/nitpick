# sloptaster

The application binary: process startup (logging, gpui-component, language, updater), the
app menu and its actions, and the main window. For now the window is an empty view under
gpui-component's `TitleBar`.

Not here: logging (`diagnostics`), auto-update (`updater`), icons and strings
(`ui_resources`), packaging (`Cargo.toml` `[package.metadata.packager]`, `packaging/`,
`.github/workflows/release.yml`).

## Files

- `main.rs` — `main`, window options, opening the main window, app-level action handlers.
- `app_menu.rs` — the actions and the menu bar (`set_menus`); "Check for Updates" only when
  `updater::is_supported()`.
- `main_view.rs` — `MainView`, the root view inside gpui-component's `Root`.
- `../build.rs` — embeds `icon.ico` into the Windows executable.
- `../Info.plist` — merged into the macOS bundle's plist by `cargo packager`.

## Rules

- Every window goes through `Root::new`: it draws the Linux client-side border and resize
  edges and mounts the notification, dialog and sheet layers the updater toasts need.
- Window options: transparent, client-decorated windows on Linux (`TitleBar` draws the
  controls), `app_owns_titlebar_drag` everywhere so the title bar handles drag and double
  click itself.
- `Info.plist` sets `LSMultipleInstancesProhibited`: the macOS updater relaunches with plain
  `open` and relies on it (`updater/src/install/doc.md`).
- The `self-update` feature only forwards to `updater/self-update`; the portable Windows build
  turns it off and this crate has no `#[cfg]` for it.
- Quit on last window closed everywhere except macOS, where the app stays in the dock and
  `on_reopen` opens a new window.
