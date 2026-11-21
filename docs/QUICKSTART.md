# Quick start (5 minutes)

This walks you from an empty directory to a running Y901 window. It is the same program
as [`crates/y901/examples/hello.rs`](../crates/y901/examples/hello.rs), which CI compiles
on every change.

## 1. Prerequisites

| Requirement | Notes |
|---|---|
| Rust **1.92+** | The repository pins the toolchain with `rust-toolchain.toml`. |
| Windows 10/11 | Tier 1 platform. Building does not need Visual Studio or the Windows SDK. The default build produces an executable that uses the VC++ runtime; install the [VC++ redistributable](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist) if the exe fails to start. |
| Linux with X11 | Tier 2. Run inside an X session (or Xvfb for headless). Wayland works through XWayland. |

No C/C++ compiler is required for Y901 itself: the framework is pure Rust end to end.

## 2. Create a project

```bash
cargo new hello-y901
cd hello-y901
```

Add the single entry crate:

```toml
# Cargo.toml
[dependencies]
y901 = "0.2.6"
```

> Working inside this repository instead? Use a path dependency:
> `y901 = { path = "crates/y901" }`.

## 3. Write `src/main.rs`

```rust
use y901::prelude::*;
use y901::ui::{Command, Edges, Flex, FlexItem, Json};

struct DemoHost {
    clicks: u32,
}

impl Host for DemoHost {
    fn idle(&mut self, _tree: &mut UiTree) {}

    fn command(&mut self, tree: &mut UiTree, cmd: Command) -> bool {
        let Command::App { name, .. } = &cmd else {
            return false;
        };
        if name != "demo.click" {
            return false;
        }
        self.clicks += 1;
        if let Some(label) = tree.node_as_mut::<TextLabel>("label") {
            label.set_text(format!("Clicked {} time(s)", self.clicks));
        }
        tree.invalidate_all();
        true
    }
}

fn build() -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(12, 12, 16))
        .padding(Edges::all(24.0))
        .gap(12.0)
        .push(Box::new(
            TextLabel::new("label", "Not clicked yet")
                .color(Color::rgba(0.85, 0.9, 1.0, 1.0))
                .flex_item(FlexItem::height(28.0)),
        ))
        .push(Box::new(
            Button::new("btn", "Click me")
                .on_click("demo.click", Json::obj())
                .flex_item(FlexItem::height(36.0)),
        ));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    let options = Options {
        title: "Y901 · hello".to_string(),
        width: 520,
        height: 220,
        ..Default::default()
    };
    // Platform selection happens inside `create_runtime()`:
    // application code contains no `#[cfg(target_os)]`.
    y901::create_runtime().run(Box::new(DemoHost { clicks: 0 }), Box::new(build), options)
}
```

## 4. Run it

```bash
cargo run
```

A 520×220 window opens with a label and a button. Click the button and the label counts
clicks. The loop redraws only what changed.

## 5. Next steps

- Component gallery (from a checkout of this repository):
  `cargo run -p y901-shell --example static_page`, then try `table`, `alert`, `tree`.
- Learn the widget model: `docs/FRAMEWORK_GUIDE.md` (component development guide) and
  `docs/FRAMEWORK.md` (architecture).
- Check the support matrix before choosing a platform: `docs/PLATFORMS.md`.
- Current gaps and their plan: `docs/ROADMAP.md`.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `DISPLAY 未设置` / `DISPLAY is not set` | No X server reachable. Run inside an X session, or use `xvfb-run -a cargo run` for a headless smoke test. |
| Window opens but the exe fails to start on a clean Windows machine | The VC++ runtime is missing; install the redistributable or build with the GNU toolchain. |
| Text is misaligned with a proportional font | The cell model assumes monospace. Keep the default `NSimSun`, or use a monospace family. |
| Arabic/Indic text renders in isolated glyphs | Expected: there is no shaping engine yet (see README "Known limitations"). |