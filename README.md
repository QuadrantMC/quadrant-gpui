# Quadrant GPUI

A native desktop frontend for [Quadrant](https://github.com/QuadrantMC/quadrant), the Minecraft mod and modpack manager, built with [GPUI](https://www.gpui.rs) and [gpui-component](https://crates.io/crates/gpui-component).

It drives the same backend as the Tauri app through the `quadrant-host` crate. It shares that app's data folder (`<data dir>/dev.mrquantumoff.mcmodpackmanager`), so settings and modpacks are the same in both.

Status: phase 2. Every page of the rail works: Apply (with Quadrant Share, and Quadrant Sync once signed in), Current modpack, Search and mod install, Installed content, Account and Settings. The title bar's notification bell does nothing yet.

## Build and run

Rust 1.99 or newer, edition 2024.

On Linux the app runs on Wayland only. GPUI's X11 backend is not compiled in, so an X11 session can't open the window.

```sh
cargo run              # debug build
cargo build --release
cargo test
```

The default build needs no credentials. Quadrant ID, Quadrant Share and telemetry pick up `QUADRANT_OAUTH2_CLIENT_ID`, `QUADRANT_OAUTH2_CLIENT_SECRET` and `QUADRANT_API_KEY` at compile time when they are set, as the Quadrant CLI does.

### Features

| Feature | What it adds |
|---|---|
| `curseforge` | CurseForge support in the backend. Needs `ETERNAL_API_TOKEN` at compile time, so it is off by default. |
| `screenshot` | `--screenshot <dir>`: renders every page and its main states to PNGs and quits. The window stays hidden, so clicks can't reach it, and it runs without the Quadrant ID client and API key, so it never signs in, syncs or shares. It reads GPUI's scene, not the screen. |
| `perf` | `--perf <scenario\|all> --perf-out <dir> [--perf-runs <n>]`: drives a maximized window through fixed scenarios and records every frame's time. Needs `--data-dir` and `--mc-folder`, runs without credentials, and ignores real mouse and keyboard input. Windows only so far. |

### Pointing it somewhere else

| Flag | Environment variable | Default |
|---|---|---|
| `--data-dir <path>` | `QUADRANT_GPUI_DATA_DIR` | the Tauri app's data folder |
| `--mc-folder <path>` | `QUADRANT_GPUI_MC_FOLDER` | the `mcFolder` setting, else the platform's `.minecraft` |

Flags win over the environment. The host saves `--mc-folder` as the `mcFolder` setting of the data folder in use, so the app refuses `--mc-folder` without `--data-dir`: it would otherwise change the Tauri app's setting. A `--data-dir` also gets its own Quadrant ID sign-in in the OS keyring, so a sandbox starts signed out and never uses the Tauri app's account.

Applying, creating and deleting modpacks changes the Minecraft folder. Try things against the sandbox instead:

```powershell
scripts\sandbox.ps1   # under %TEMP%\quadrant-gpui-sandbox: two fixture modpacks (one applied), packs, a Prism Launcher instance
cargo run -- --data-dir "$env:TEMP\quadrant-gpui-sandbox\data" --mc-folder "$env:TEMP\quadrant-gpui-sandbox\mc"
cargo run --features screenshot -- --data-dir "$env:TEMP\quadrant-gpui-sandbox\data" --mc-folder "$env:TEMP\quadrant-gpui-sandbox\mc" --screenshot "$env:TEMP\quadrant-gpui-shots"
```

Applying a modpack links `mods` to its folder. On Windows that needs Developer Mode or an elevated shell.

## Frame times

The app has to keep up with the display: a frame every refresh, and never less than 120 a second, so the budget is `1000 / max(120, refresh rate)` ms. `scripts\perf.ps1` measures it in one command. It builds a release binary with `perf`, rebuilds a large sandbox (`scripts\sandbox.ps1 -Large`: 300 modpacks, 500 mods in the applied one, about 1000 resource and shader packs) and runs every scenario five times in a visible, maximized window, then prints the median of each statistic.

```powershell
scripts\perf.ps1                                   # every scenario, 5 runs, large sandbox
scripts\perf.ps1 -Scenario scroll-modpacks -Runs 3 -Sandbox small
```

Each frame is timed from the window procedure, around the `WM_PAINT` that GPUI's vsync thread asks for every refresh: next-frame callbacks, render, layout, paint and `Present`. A frame presented more than 1.5 refresh periods after the one before counts as dropped. The scenarios step once per frame through GPUI (scroll-wheel and mouse-move events, navigation, snackbars, dialogs, window resizes) and invalidate only what the change would: a scroll step notifies its page, so the page redraws every refresh, while a snackbar, a dialog or the pointer redraw what they touch. A refresh with nothing new to draw is not a drop. `idle` leaves each page alone for five seconds and counts what it draws anyway. Results go to `perf-log\runs\` (ignored by git): a JSON file per scenario with every frame, and `summary.txt`. Keep the mouse and keyboard away while it runs; the window ignores them but warns when it sees any.

## Developing against a local quadrant checkout

`quadrant-host` and `quadrant-core` come from a pinned upstream revision. To build against a sibling checkout instead, uncomment the `[patch."https://github.com/QuadrantMC/quadrant"]` block at the end of `Cargo.toml`. It expects the checkout at `../quadrant`.

## Assets

- `assets/fonts/Inter.ttf` is the desktop app's Inter, by the Inter Project Authors, under the SIL Open Font License 1.1.
- `assets/icons/*.svg` are Google's Material Design icons, the set the desktop app draws through `react-icons/md`, under the Apache License 2.0. `node scripts/extract-icons.mjs` regenerates them from a quadrant checkout with its dependencies installed.
- `locales/en.json` is the desktop app's English strings. Copy it again from `quadrant/src/locales/en.json` when the pinned revision moves.

## License

MPL-2.0, like Quadrant.
