# Quadrant-Native

Quadrant-Native is a native desktop frontend for [Quadrant](https://github.com/QuadrantMC/quadrant), the Minecraft mod and modpack manager, built with [GPUI](https://www.gpui.rs) and [gpui-component](https://crates.io/crates/gpui-component).

It drives the same backend as the Tauri app through the `quadrant-host` crate. It shares that app's data folder (`<data dir>/dev.mrquantumoff.mcmodpackmanager`), so settings and modpacks are the same in both. Everything else is its own: the binary `quadrant-native`, the bundle identifier `dev.mrquantumoff.quadrant-native`, the installers and the update feed, so it installs beside the Tauri app.

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
| `updater` | The self-updater (see "Updates"). Release builds turn it on; dev builds leave it off. |
| `perf` | `--perf <scenario\|all> --perf-out <dir> [--perf-runs <n>]`: drives a maximized window through fixed scenarios and records every frame's time. Needs `--data-dir` and `--mc-folder`, runs without credentials, and ignores real mouse and keyboard input. Windows only so far. |

### Pointing it somewhere else

| Flag | Environment variable | Default |
|---|---|---|
| `--data-dir <path>` | `QUADRANT_GPUI_DATA_DIR` | the Tauri app's data folder |
| `--mc-folder <path>` | `QUADRANT_GPUI_MC_FOLDER` | the `mcFolder` setting, else the platform's `.minecraft` |
| `--noupdater` | | the updater runs in installed release builds |
| `--debug` or `--log` | | quiet: no console on Windows, info-level logs |

On Windows the exe is a GUI app, so starting it from the Start menu, the installer or Explorer opens no console window. Its output still reaches a parent that pipes or redirects it: `cargo run`, `Start-Process -RedirectStandardError`, or a PowerShell pipeline such as `& $exe ... 2>&1 | ForEach-Object { "$_" }`, which also waits for it and sets `$LASTEXITCODE`. A bare `& $exe ...` in PowerShell, or `cmd /c`, neither waits nor shows the output. `--debug` (or `--log`) attaches to the terminal it was started from, or opens a console of its own, and turns on debug logs for the app and the backend (unless `RUST_LOG` says otherwise); on Linux and macOS it only raises the log level. An error that stops the app before or while it opens its window goes to stderr, and on Windows also to a message box when nothing reads stderr. Set `QUADRANT_NATIVE_NO_DIALOG` to keep it out of a dialog, as tests and scripts should.

Flags win over the environment. The host saves `--mc-folder` as the `mcFolder` setting of the data folder in use, so the app refuses `--mc-folder` without `--data-dir`: it would otherwise change the Tauri app's setting. A `--data-dir` also gets its own Quadrant ID sign-in in the OS keyring, so a sandbox starts signed out and never uses the Tauri app's account.

Applying, creating and deleting modpacks changes the Minecraft folder. Try things against the sandbox instead:

```powershell
scripts\sandbox.ps1   # under %TEMP%\quadrant-gpui-sandbox: two fixture modpacks (one applied), packs, a Prism Launcher instance
cargo run -- --data-dir "$env:TEMP\quadrant-gpui-sandbox\data" --mc-folder "$env:TEMP\quadrant-gpui-sandbox\mc"
cargo run --features screenshot -- --data-dir "$env:TEMP\quadrant-gpui-sandbox\data" --mc-folder "$env:TEMP\quadrant-gpui-sandbox\mc" --screenshot "$env:TEMP\quadrant-gpui-shots"
```

Applying a modpack links `mods` to its folder. On Windows that needs Developer Mode or an elevated shell.

## Updates

With the `updater` feature, an installed copy updates itself the way the Tauri app does, through the same update server. At launch, and whenever the update channel changes in Settings, it asks

```
https://api.usequadrant.dev/api/any/quadrant-native/updates/{channel}/{target}/{arch}/{version}?variant={bundle}
```

with Tauri's names (`windows`, `linux`, `darwin`; `x86_64`, `aarch64`) and its install kind (`nsis`, `appimage`, `deb`, `rpm`, `app`). The channel is the `channel` setting the Tauri app keeps in the shared config, `stable` unless set; a non-stable channel falls back to stable's URL when its own fails. The answer is Tauri's updater JSON. A different version counts as an update, so switching back to stable moves to stable's release. The download is checked against the minisign signature in the answer with Quadrant's updater public key, the one in Quadrant's `tauri.conf.json`, before anything is kept. The title bar then shows "Install an update". Pressing it runs the NSIS setup passively (it relaunches the app), swaps the AppImage or the macOS `.app` and relaunches, or upgrades the deb or rpm through `pkexec`.

The updater stays off for `--noupdater` (for package managers that own updates), for screenshot and perf runs, and for a copy that isn't an install it can replace: a build in `target/`, or a Linux binary no package owns. Checks and failures only log, as in the Tauri app.

For testing, debug builds read `QUADRANT_NATIVE_UPDATE_ENDPOINT` (an update server to ask instead, plain http allowed) and `QUADRANT_NATIVE_BACKGROUND` (open the window without focus). `QUADRANT_NATIVE_UPDATER_PUBKEY` at compile time swaps the public key. `scripts\update-e2e.ps1` uses all three to run the whole Windows update against a local server and a throwaway key. It installs the old build for this user, lets it download and verify the new one, presses the button through UI Automation, checks the new version took over, refuses a tampered setup and one signed with another key, uninstalls and checks nothing is left. It needs `cargo-packager` and the Tauri CLI from `../quadrant/node_modules`.

## Packaging

[cargo-packager](https://github.com/crabnebula-dev/cargo-packager) 0.11 builds the installers from `[package.metadata.packager]` in `Cargo.toml`, and [cargo-generate-rpm](https://github.com/cat-in-136/cargo-generate-rpm) builds the rpm. Build the binary first, then package it:

```sh
cargo install cargo-packager --locked --version 0.11.8
cargo build --release --locked --features updater
cargo packager --release -f nsis             # Windows: per-user setup in target/release
cargo packager --release -f deb,appimage     # Linux
cargo generate-rpm -s 'version = "26.10.0"' -s 'release = "stable"'   # rpm versions can't contain '-'
cargo packager --release -f app              # macOS; release.yml signs it and makes the dmg
```

If Scoop's NSIS sets `NSISDIR`, unset it for `cargo packager`, or makensis can't find cargo-packager's NSIS plugins.

The icons in `packaging/icons` are Quadrant's (`src-tauri/icons`). `build.rs` embeds `icon.ico` and a version resource in the Windows exe, and gpui's window class loads it for the title bar and the taskbar. On Linux the Wayland app id is `quadrant-native`, the name of the packages' desktop file (`packaging/linux/quadrant-native.desktop`) and of the hicolor icons. Linux builds need `libxkbcommon-dev libwayland-dev libvulkan-dev libfontconfig-dev libfreetype-dev pkg-config`. Build release packages on the oldest distribution you support: the binary needs at least the glibc it was built with (CI uses Ubuntu 24.04).

The NSIS uninstaller removes the app, its Start menu entry and its uninstall entry. Like Tauri's, it keeps `HKCU\Software\MrQuantumOFF (Demir Yerli)\Quadrant-Native`, the install folder it offers next time.

## Releasing

Versions follow Quadrant's `YY.MM.REVISION` with the channel as the pre-release: `26.10.0-stable`, `26.10.1-preview.1`. Set it in `Cargo.toml`, commit, and push the tag `v<version>`. `.github/workflows/release.yml` then checks the tag against `Cargo.toml` and builds Linux x64 and arm64 (deb, AppImage, rpm), Windows x64 and arm64 (NSIS) and macOS arm64 (an ad-hoc signed `.app` in a dmg, plus `.app.tar.gz` for the updater). It signs every updater artifact with Tauri's signer, uploads it all to the release, assembles `latest.json` and `PUT`s it to `/api/any/quadrant-native/add_updates/{channel}/{tag}`. Stable releases arrive unpublished, and RelMan publishes them like Quadrant's.

Secrets:

| Secret | Used for |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Quadrant's updater key; the same secret signs both apps |
| `VERSION_UPDATE_TRIGGER_TOKEN` | the `Authorization` header of `add_updates` |
| `QUADRANT_OAUTH2_CLIENT_ID`, `QUADRANT_OAUTH2_CLIENT_SECRET`, `QUADRANT_API_KEY` | Quadrant ID, Share and telemetry, baked in at compile time |
| `ETERNAL_API_TOKEN` | CurseForge; the build adds the `curseforge` feature when it is set |
| `RELEASES_REPO_TOKEN` | write access to `RELEASES_REPO`, when that is another repository |

Variables:

| Variable | Default | Meaning |
|---|---|---|
| `RELEASES_REPO` | this repository | the repository whose release carries the downloads |
| `ASSET_BASE_URL` | `https://github.com/<RELEASES_REPO>/releases/download` | where `latest.json` says the files are, as `<base>/<tag>/<file>` |

This repository is private, so its release assets can't be downloaded by users or by the updater. Before the first release, point `RELEASES_REPO` at a public repository (for example a `quadrant-native-releases` repository under QuadrantMC) with a `RELEASES_REPO_TOKEN` that can write to it, or serve the files elsewhere and set `ASSET_BASE_URL`.

One-time server-side steps: add `quadrant-native` to RelMan's `APPS` so its stable releases can be published, and make sure `VERSION_UPDATE_TRIGGER_TOKEN` here holds an update-server API key (product `updater`, scope `update`). The server needs no other change. It is generic over the app name, and `quadrant-native` already answers `404 No supported versions` until the first release is published.

## Frame times

The app has to keep up with the display: a frame every refresh, and never less than 120 a second, so the budget is `1000 / max(120, refresh rate)` ms. `scripts\perf.ps1` measures it in one command. It builds a release binary with `perf`, rebuilds a large sandbox (`scripts\sandbox.ps1 -Large`: 300 modpacks, 500 mods in the applied one, about 1000 resource and shader packs) and runs every scenario five times, then prints the median of each statistic.

```powershell
scripts\perf.ps1                                   # every scenario, 5 runs, large sandbox
scripts\perf.ps1 -Scenario scroll-modpacks -Runs 3 -Sandbox small
scripts\perf.ps1 -Placement visible                # maximized on screen, for a final check
```

By default the window is off screen: the primary display's work-area size (the client area a maximized window gets, so the same resolution and scale), placed past the right edge of the virtual desktop and shown without activation. It never takes focus, never appears on screen or in the taskbar, and real input can't reach it, so a run doesn't disturb whoever is using the machine. GPUI's vsync thread still asks it for a frame every refresh: off screen, ticks arrive every 6.94 ms on a 144 Hz display with less jitter than on screen, so `DwmFlush` paces them and GPUI never falls back to sleeping. On four scenarios the off-screen medians matched the visible ones within the run-to-run noise of a busy machine (`scroll-modpacks` 129 vs 126 ms p50, `search` 10.4 vs 10.4 ms, `snackbar` 135 vs 121 ms, `idle` 0 vs 0 frames), the gaps following the background load. The process also opts out of Windows' power throttling, which otherwise slows down processes whose windows are hidden or unfocused.

Each frame is timed from the window procedure, around the `WM_PAINT` that GPUI's vsync thread asks for every refresh: next-frame callbacks, render, layout, paint and `Present`. A frame presented more than 1.5 refresh periods after the one before counts as dropped. The scenarios step once per frame through GPUI (scroll-wheel and mouse-move events, navigation, snackbars, dialogs, window resizes) and invalidate only what the change would: a scroll step notifies its page, so the page redraws every refresh, while a snackbar, a dialog or the pointer redraw what they touch. A refresh with nothing new to draw is not a drop. `idle` leaves each page alone for five seconds and counts what it draws anyway. Results go to `perf-log\runs\` (ignored by git): a JSON file per scenario with every frame, and `summary.txt`. Each run also records how many cores other processes used meanwhile (`load` in the summary) and warns above two: frame times rise with background load, so compare runs taken under similar load.

## Developing against a local quadrant checkout

`quadrant-host` and `quadrant-core` come from a pinned upstream revision. To build against a sibling checkout instead, uncomment the `[patch."https://github.com/QuadrantMC/quadrant"]` block at the end of `Cargo.toml`. It expects the checkout at `../quadrant`.

## Assets

- `assets/fonts/Inter.ttf` is the desktop app's Inter, by the Inter Project Authors, under the SIL Open Font License 1.1.
- `assets/icons/*.svg` are Google's Material Design icons, the set the desktop app draws through `react-icons/md`, under the Apache License 2.0. `node scripts/extract-icons.mjs` regenerates them from a quadrant checkout with its dependencies installed.
- `locales/en.json` is the desktop app's English strings. Copy it again from `quadrant/src/locales/en.json` when the pinned revision moves.
- `packaging/icons` holds Quadrant's app icons, from `quadrant/src-tauri/icons`.

## License

MPL-2.0, like Quadrant.
