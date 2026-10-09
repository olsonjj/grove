# Grove

macOS app that shows every Git worktree in your code folder as one tree and opens any of them in your editor or terminal in one action.

- **Main window** (⌥⌘G): repos → checked-out branches → worktrees, with state badges (changes, ahead/behind, upstream gone, locked, …) and a details pane.
- **Quick switcher** (⌥⌘K): fuzzy-find a worktree from anywhere and open it.
- **Menu bar popover**: same, from the status item.
- **Settings**: code folder, default editor/terminal, custom commands, launch at login.

Grove only reads repos, except `git fetch` (on request or background) and Remove worktree (always confirmed; never touches the branch).

Specs live in [`specs/`](specs/).

## Requirements

- macOS 14+
- Rust 1.85+ (edition 2024) via [rustup](https://rustup.rs)
- [`just`](https://github.com/casey/just): `brew install just`
- [`cargo-bundle`](https://github.com/burtonageo/cargo-bundle) for `.app` builds: `cargo install cargo-bundle`
- Git 2.20+ at runtime

Main crates: [GPUI Kit](https://gpui-kit.com) (UI), `tray-icon` (menu bar), `global-hotkey` (⌥⌘K/⌥⌘G), `notify` (file watching), `objc2-*` (AppKit, login item). All pinned in the workspace `Cargo.toml`.

## Layout

- `crates/grove-core`: discovery, Git state, settings, snapshot. No UI.
- `crates/grove-app`: GPUI app, windows, menu bar, hotkeys. Icon in `icons/`.

## Build

| Command | Does |
| --- | --- |
| `just run` | Debug build. `just run /tmp/s.json` uses a throwaway settings file instead of the real one. |
| `just run-release` | Release build |
| `just check` | fmt check, clippy (`-D warnings`), tests. Must pass before merging. |
| `just test` | Tests only |
| `just fmt` | Format |
| `just bundle` | Ad-hoc-signed `target/release/bundle/osx/Grove.app`; increments build number |
| `just install` | Bundle, copy to `/Applications` |
| `just zip` | Bundle, zip to `target/Grove.zip` for a tester |

Settings live at `~/Library/Application Support/Grove/settings.json` (override with `GROVE_SETTINGS_PATH`).

## Versioning

Shown in Settings as `Grove 0.1.0 (build 12)`.

- **Version** (semver, `CFBundleShortVersionString`): `version` under `[workspace.package]` in `Cargo.toml`. Change only with:
  - `just bump patch`: fixes (0.1.0 → 0.1.1)
  - `just bump minor`: features (0.1.0 → 0.2.0)
  - `just bump major`: breaking (0.1.0 → 1.0.0)
- **Build number** (`CFBundleVersion`): `crates/grove-app/build-number`. Every `just bundle` (and so `install`/`zip`) increments it. Never resets; commit it with the release.

Release flow: `just bump minor` → `just bundle` → commit `Cargo.toml`, `Cargo.lock`, `build-number`.
