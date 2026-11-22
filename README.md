# Y901

[![crates.io](https://img.shields.io/crates/v/y901.svg)](https://crates.io/crates/y901)
[![docs.rs](https://img.shields.io/docsrs/y901)](https://docs.rs/y901)
[![CI](https://github.com/901548/y901/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/901548/y901/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/y901.svg)](#license)

A pure-Rust cross-platform GUI framework with its own Win32/D3D11, X11 and Web (wasm)
backends. Text uses a **monospace cell model** (`0.5em × 1em`), which makes it a good fit
for terminal-style desktop tools, log viewers and file managers.

> [中文说明](README.zh-CN.md) · [Roadmap](docs/ROADMAP.md) · [Platform support](docs/PLATFORMS.md) · [v0.3.0 release notes (draft)](docs/RELEASE_NOTES_v0.3.0.md)

## Status

**Experimental (0.x).** Windows/D3D11 is the reference platform (Tier 1). Linux/X11 is
supported on a best-effort basis (Tier 2). The browser backend is a **demo/reach** tier
(Tier 3), not production-ready. macOS and Wayland have no backend. See
[docs/PLATFORMS.md](docs/PLATFORMS.md) for the exact support matrix and what is out of scope.

## Install

```toml
[dependencies]
y901 = "0.3.0"
```

`y901` is the **single entry point**: it re-exports the ten `y901-*` implementation crates.

```rust
use y901::prelude::*;

fn main() -> Result<(), String> {
    // No `#[cfg(target_os)]` in application code: platform selection happens inside y901.
    let runtime = y901::create_runtime();
    let _ = runtime; // Starting a window requires a display; see examples/
    Ok(())
}
```

## Quick start

The five-minute path (new project → running window) is in
[docs/QUICKSTART.md](docs/QUICKSTART.md).

```bash
# Minimal facade example (depends on `y901` only)
cargo run --example hello -p y901

# Component gallery (lives in y901-shell, hence -p)
cargo run -p y901-shell --example static_page   # layout / colours / rounded corners
cargo run -p y901-shell --example table         # multi-column table with row selection
cargo run -p y901-shell --example alert         # four alert variants
```

## Components

| Category | Widgets |
|---|---|
| Input | `Button` `TextInput` `TextArea` `Checkbox` `RadioGroup` `Switch` `Slider` |
| Layout | `Flex` (with `radius`/`border`) `Grid` `Stack` `ScrollView` `SplitView` `Tabs` |
| Data | `Table` `ListView` `TreeView` `Image` `RichText` `TextLabel` |
| Feedback | `Spinner` `ProgressBar` `EmptyState` `ErrorState` `Alert` |
| Navigation | `Menu` `Dialog` `ContextMenu` `Tooltip` |

## Known limitations

- **International text: partial, opt-in.** The default build keeps the original cell
  pipeline — one cmap glyph per character, laid out on the monospace cell grid. The
  `shaping` feature (off by default) adds a rustybuzz shaping pass, Unicode bidi
  (P2/P3 direction + visual reordering), grapheme-cluster segmentation and a platform
  fallback font chain (Windows: Segoe UI/Tahoma/YaHei/SimSun/Nirmala; Linux: DejaVu/Noto).
  Still **not** implemented: hyphenation, justification, kinsoku line breaking, emoji ZWJ
  sequence handling, per-run multi-face splitting, and any real-word validation of complex
  scripts. CJK renders because Han/Kana/Hangul mostly fit the "one wide glyph per two
  cells" contract — that is not full complex-text support.
- **Accessibility: semantics tree + AccessKit bridge.** `Widget::semantics` exposes roles,
  labels, values, actions and state; the optional `y901-a11y` crate (facade feature `a11y`)
  maps that to AccessKit, with Windows UIA and Linux AT-SPI adapters behind features.
  Backend auto-wiring and screen-reader acceptance runs are **not** done yet.
- **Monospace-only text model.** `cell_metrics_for` always derives cells at
  `0.5em × 1em`; the default font is `NSimSun`. `FontConfig::modern()` switches to
  `Microsoft YaHei UI`, but proportional fonts are visual experiments only.
- **macOS and Wayland are out of scope.** Wayland users run through XWayland.
- **Browser backend (Tier 3).** Single canvas, full-frame `putImageData`, hidden-input IME
  with no candidate-window positioning, no multi-window/FileDrop. DOM semantics exist only
  behind the opt-in `a11y` feature (a hidden ARIA mirror wired into the frame loop); it has
  **not** been validated in a real browser or with a screen reader. The `y901` facade does
  build for `wasm32`; `TerminalWidget` works there too, but `attach_shell` returns an error
  because a browser cannot spawn a process.
- **0.x API.** Breaking changes bump the minor version (0.3.0, 0.4.0 …) and ship migration
  notes in `docs/RELEASE_NOTES_v*.md`; published version numbers are never reused.

## Repository layout (12 crates)

```
y901                 single entry point: re-exports + prelude + create_runtime()
y901-core            pure logic: terminal grid / cell widths / intervals
y901-ui              framework: Widget / UiTree / layout / events / theme / animation / semantics
y901-a11y            semantics -> AccessKit bridge (Windows UIA / Linux AT-SPI, behind features)
y901-widgets         widgets: Button / Table / Alert / Switch / ...
y901-extras          subsystem widgets: TerminalWidget (grid + VT + PTY)
y901-render          software rasterizer + in-house TTF engine + atlas + text shaping
y901-runtime         platform-agnostic contracts: Host / PlatformRuntime / frames / fonts
y901-backend-win32   Win32 + D3D11 + DirectWrite + IME + DPI
y901-backend-x11     X11 backend (Xvfb + real-server spot checks)
y901-backend-web     browser backend (wasm32, single canvas)
y901-shell           app shell: create_runtime() + clipboard + secrets + screenshots
```

## Documentation

- [docs/QUICKSTART.md](docs/QUICKSTART.md) — install, first window, next steps
- [docs/PLATFORMS.md](docs/PLATFORMS.md) — support tiers and out-of-scope platforms
- [docs/ROADMAP.md](docs/ROADMAP.md) — plan for shaping, accessibility, API stability, components
- [docs/A11Y_ACCEPTANCE.md](docs/A11Y_ACCEPTANCE.md) — 10-minute screen-reader acceptance checklist (needs a human)
- [docs/STATUS.md](docs/STATUS.md) — current state, gates and evidence (Chinese)
- [docs/FRAMEWORK.md](docs/FRAMEWORK.md) — architecture, component model, rendering contract (Chinese)
- [docs/CRATES_IO.md](docs/CRATES_IO.md) — release process and pitfalls (Chinese)
- [docs/WASM.md](docs/WASM.md) — browser backend design and gaps (Chinese)

Internal design documents are written in Chinese; public entry points (README, quick start,
platform matrix) are in English.

## Reporting problems

This is a 0.x project with no support SLA, but reports are welcome and read:

- **Bugs / feature requests** — open an issue at
  <https://github.com/901548/y901/issues>. Templates are provided; please include the
  `y901` version, OS/backend (Win32 / X11 / Web) and a minimal reproduction.
- **Security issues** — do **not** open a public issue. Use the repository's
  **Security → Report a vulnerability** tab (see [SECURITY.md](SECURITY.md)). If that tab
  is not available to you, open an issue titled `[SECURITY]` with no details and you will
  be contacted privately.
- **Usage questions** — same issue tracker, `question` label.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) (Chinese). CI enforces the full gate; PRs must also
keep `cargo deny check` green. The repository favours pure-Rust implementations: the
zero-C-dependency guard in `crates/y901-shell/tests/no_c_deps.rs` rejects `-sys` crates and
C/C++ build scripts.

## License

Dual-licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.