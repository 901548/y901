# Platform support tiers

Y901 is maintained by a very small team. To keep platform promises honest, each platform
has an explicit tier. The tier determines what CI enforces and what users can expect.

| Tier | Platform | Backend | Promise | CI evidence |
|---|---|---|---|---|
| **T1** | Windows 10/11 (x86_64) | Win32 + D3D11 + DirectWrite | Fully supported. Regressions are release blockers. | `gate.ps1` (MSRV / fmt / tests / inventory / clippy / pixel hash / docs / audit) |
| **T2** | Linux with X11 (x86_64) | raw X11 protocol | Best effort. Regressions are tracked and fixed, but a release may ship with known X11 gaps when clearly documented. | `scripts/linux-verify.sh` under Xvfb + real-server spot checks |
| **T3** | Browsers (wasm32) | single canvas + DOM events | **Demo/reach only.** Not production-ready. No compatibility promise; may break between 0.x releases. | compile + smoke tests |
| **Out of scope** | macOS, Wayland | none | No backend. Do not expect builds. Wayland users run the X11 backend through XWayland. | — |

## What each tier means in practice

### T1 — Windows

- Every change is validated by the full release gate before merge.
- Pixel-hash and DPI behaviour are contractual: a changed baseline requires explicit review.
- Accessibility: Windows UIA bridge is the first accessibility target (see roadmap).
- Known constraints: the default build targets the VC++ runtime; building does not require
  Visual Studio or the Windows SDK.

### T2 — Linux / X11

- The backend speaks the X11 wire protocol directly; no `libX11`, `xcb`, `xkbcommon` or
  other C libraries are linked.
- CI runs the whole workspace plus X11 integration tests under Xvfb.
- Real-hardware checks (HiDPI, multi-monitor RandR, IME/IBus) are performed before releases
  and recorded under `docs/evidence/`.
- Known gaps are listed in `docs/STATUS.md`; they are not blockers unless they break Tier 1.

### T3 — Web / wasm32

Explicit non-goals for now:

- full-frame `putImageData` upload (no partial upload / WebGL texture path);
- a single canvas alone exposes no semantics to the DOM. The optional `a11y` feature wires an
  ARIA mirror into the frame loop (it publishes on presented frames and skips unchanged
  semantics), but it has **no browser or assistive-technology validation** yet;
- IME composition is forwarded through a hidden `<input>`; the browser owns the candidate
  window and Y901 cannot position it;
- single window, no FileDrop, no multi-canvas.

See `docs/WASM.md` for the design and its gap table. Work to upgrade this tier is tracked
separately in `docs/ROADMAP.md`; it is not on the 0.3 critical path.

## Out of scope: macOS and Wayland

- **macOS**: no backend. The framework contract (`PlatformRuntime`, `WindowBackend`) is
  ready, but implementing Cocoa/Metal is a multi-week effort with no maintainer to validate
  it on real hardware. Do not advertise macOS support.
- **Wayland**: deliberately deferred. A correct Wayland backend would add C dependencies
  (e.g. `xkbcommon`) and break the zero-C invariant, on top of doubling the Linux backend
  surface. XWayland is the supported path.

## Reporting platform issues

Label issues with the tier and platform. T1 regressions are treated as release blockers;
T2 issues are triaged but may be scheduled behind the current milestone; T3 issues are
collected but not guaranteed to be fixed. If you need a platform moved up a tier, the
realistic path is to contribute CI coverage and a maintainer who can validate it.