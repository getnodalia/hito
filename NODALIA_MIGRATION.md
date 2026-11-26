# Nodalia migration: Hito

Category: secondary app (new, built for Nodalia from the start).
Status: 0.1.0 on a pull request, 2026-11-18.

## Current state

Rust workspace: `hito-server` (Tokio, Axum, SQLx/SQLite) and `hito-desktop` (egui/eframe with
`nodalia-theme`), plus `hito-core`, `hito-protocol`, `hito-storage` and `hito-client`. CI calls
Foundation's `rust-gate`, `git-sources` and `agents-check` workflows, pinned by commit.

## Desired state

Enterprise suite: the window shell, top bar, sidebar, workspace switcher, account card, sign-in,
integration windows, About and update window are `nodalia_theme::enterprise`, shared with Ágora;
Home (project templates and recents) is the one page of its own, and Preferences, shortcuts and
About are dialogs. Released through Foundation's `release-tag`/`release-publish` with release notes, packages for
Linux, macOS (DMG with its own install window) and Windows, the shared in-app updater
(`nodalia-update`), and the Linux global menu over D-Bus.

## Inherited dependencies still pending

None: Hito has no upstream.

## Reusable Nodalia components

- `nodalia-theme` (egui, `creator` feature): palette with `Accent::Hito` (`#3D9A2B`), islands,
  buttons, fields, window controls, icons.
- `tools/app-portrait` in nodalia-design made the icon.

## NDL work pending

- `creator::search_field` draws its text at the top of a tall field; Hito draws its own centred
  copy (`widgets::search_field`) until nodalia-design centres it.
- The task board, cards and progress bars are drawn in Hito; if another app needs them, they move
  to nodalia-design.

## Missing tests

- Desktop app: the model reducer, settings and translations have unit tests; the views are checked
  by hand with screenshots. A headless render test of the main screens would catch layout
  regressions.
