# Local verification

Base: PhotoCraft 0.8.2, upstream commit `99e1138877e6aa7e856c6883b415e708d042fc59`.
Environment: Linux x86_64 (Amazon Linux 2023), Rust 1.99.0.

## Passed

| Check | Actual result |
|---|---|
| Full `photocraft-engine` library tests | **1611 passed, 0 failed, 13 ignored** |
| New social-export engine tests (included above) | **8 passed** |
| Full `photocraft-cli` process integration suite | **31 passed, 0 failed** |
| New native UI tests | **2 selected tests passed**; the other 2695 UI library tests were not run |
| Real CLI exports | **8 runs passed**: every preset × crop/fit, exact dimensions, source hash unchanged |
| Clippy | All targets of engine, CLI and ui-egui passed with `-D warnings` using one build worker |
| Rust formatting / whitespace | `cargo fmt --all -- --check` and `git diff --check` passed |
| Architecture | `cargo xtask layers`: **29 crates, no layering violations** |
| Menu catalogue | `cargo xtask parity` completed; generated checklist updated |
| Browser compatibility | Engine + ui-egui `cargo check --target wasm32-unknown-unknown` passed |
| Native desktop build | `cargo build -p photocraft` passed; stripped packaged binary answers `--version` |
| Native CLI build | Process tests passed; stripped packaged binary answers `--version` and `social-export --help` |

Total across the full engine run, CLI process suite and selected new UI tests: **1644 passed**. This includes upstream tests; it is not a claim that 1644 new tests were written.

## Visual inspection

The actual egui application was rendered offscreen using its `snapshot` example, Vulkan llvmpipe and the CPU canvas path (`--safe-gpu`). The Social Batch Export dialog was opened through the real menu/control route.

- **1440 × 900:** dialog labels, fields, preset choices and action buttons were inspected; no overlap or clipping inside the new dialog.
- **800 × 600:** the new dialog remains inside the window; its controls and buttons fit.
- **1280 × 440:** the original project/profile SVG cover was rendered and visually inspected.

These are real renders, not a web mock-up. This is not a claim that a physical GPU, a full interactive desktop session, or Windows/macOS were tested. No FPS benchmark is claimed.

## Nonfatal warning and limits

The wasm check emits one dead-code warning for upstream `PAR_PIXELS`; that file was not modified by this feature. The native strict Clippy run passed.

The full 2697-test UI library suite, corpus/fuzz suites, optional HEIF/AVIF features, signed installers and Windows/macOS execution were not verified here. Thirteen upstream engine tests remain ignored by their normal test configuration. The new feature is native-only; the wasm check confirms the fork still compiles for the browser, not that browsers can access local folders.

Cancellation is checked between files/stages; it does not undo already written outputs, and an in-flight resize/encode can finish before cancellation is noticed.

## Artifacts

The chat delivery includes the complete source ZIP. Desktop and CLI Linux test binaries were successfully built and checked in the earlier local run, but the temporary computer reset on continuation before those binaries could be delivered; the binary bundle is therefore not available in this handoff. The results above record that earlier run, not a second run after the reset. Build locally using the README or use the configured CI workflow. GitHub contains source and documentation; a remote CI pass, signed installer or new GitHub Release is not claimed.

Reproduce the eight export checks by using each listed preset with both resize modes. Their measured dimensions/results are recorded in [export-qa.json](export-qa.json).
