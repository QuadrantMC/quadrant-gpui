# Quadrant-Native

Quadrant-Native is a native desktop frontend for [Quadrant](https://github.com/QuadrantMC/quadrant), the Minecraft mod and modpack manager, built with [GPUI](https://www.gpui.rs) and [gpui-component](https://crates.io/crates/gpui-component).

It drives the same backend as the Tauri app through the `quadrant-host` crate. It shares that app's data folder (`<data dir>/dev.mrquantumoff.mcmodpackmanager`), so settings and modpacks are the same in both. Everything else is its own: the binary `quadrant-native`, the bundle identifier `dev.mrquantumoff.quadrant-native`, the installers and the update feed, so it installs beside the Tauri app.

Status: phase 2. Every page of the rail works: Apply (with Quadrant Share, and Quadrant Sync once signed in), Current modpack, Search and mod install, Installed content, Account and Settings. The title bar's bell opens the desktop app's notifications panel, and closing the window hides it in the tray (see "Tray").

## Build and run

Rust 1.99 or newer, edition 2024.

On Linux the app runs on Wayland only. GPUI's X11 backend is not compiled in, so an X11 session can't open the window.

```sh
cargo run              # debug build
cargo build --release
cargo test
```

The default build needs no credentials. Quadrant ID, Quadrant Share and telemetry pick up `QUADRANT_OAUTH2_CLIENT_ID`, `QUADRANT_OAUTH2_CLIENT_SECRET` and `QUADRANT_API_KEY` at compile time when they are set, as the Quadrant CLI does.

At startup the app refreshes the Minecraft version list in the background and, when data collection is on in Settings, sends the usage report, as the desktop app does. Only an interactive run on the desktop app's data folder with `QUADRANT_API_KEY` built in reports usage: a `--data-dir` sandbox, screenshot, perf and `QUADRANT_NATIVE_BACKGROUND` runs never do.

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
| `--autostart` | | the window opens; with it the app starts in the tray, as its login item launches it |
| a link, such as `modrinth://mod/sodium` | | open it (see "Single instance and links") |

On Windows the exe is a GUI app, so starting it from the Start menu, the installer or Explorer opens no console window. Its output still reaches a parent that pipes or redirects it: `cargo run`, `Start-Process -RedirectStandardError`, or a PowerShell pipeline such as `& $exe ... 2>&1 | ForEach-Object { "$_" }`, which also waits for it and sets `$LASTEXITCODE`. A bare `& $exe ...` in PowerShell, or `cmd /c`, neither waits nor shows the output. `--debug` (or `--log`) attaches to the terminal it was started from, or opens a console of its own, and turns on debug logs for the app and the backend (unless `RUST_LOG` says otherwise); on Linux and macOS it only raises the log level. An error that stops the app before or while it opens its window goes to stderr, and on Windows also to a message box when nothing reads stderr. Set `QUADRANT_NATIVE_NO_DIALOG` to keep it out of a dialog, as tests and scripts should.

Flags win over the environment. The host saves `--mc-folder` as the `mcFolder` setting of the data folder in use, so the app refuses `--mc-folder` without `--data-dir`: it would otherwise change the Tauri app's setting. A `--data-dir` also gets its own Quadrant ID sign-in in the OS keyring, so a sandbox starts signed out and never uses the Tauri app's account.

Applying, creating and deleting modpacks changes the Minecraft folder. Try things against the sandbox instead:

```powershell
scripts\sandbox.ps1   # under %TEMP%\quadrant-gpui-sandbox: two fixture modpacks (one applied), packs, two Prism Launcher instances (one with Alpha Pack applied)
cargo run -- --data-dir "$env:TEMP\quadrant-gpui-sandbox\data" --mc-folder "$env:TEMP\quadrant-gpui-sandbox\mc"
cargo run --features screenshot -- --data-dir "$env:TEMP\quadrant-gpui-sandbox\data" --mc-folder "$env:TEMP\quadrant-gpui-sandbox\mc" --screenshot "$env:TEMP\quadrant-gpui-shots"
```

Applying a modpack links `mods` to its folder. On Windows that needs Developer Mode or an elevated shell.

Screenshot mode starts with the title bar: its pictures, the bell's panel, mouse back and forward, and on Windows a check that the window controls take clicks while the rest of the bar drags (it sends the hidden window `WM_NCHITTEST` and fails the run on a wrong answer). `QUADRANT_NATIVE_SCREENSHOT_ONLY=title-bar` stops there.

## Color themes

Settings' "Color theme" switches the whole window between Quadrant (the default), Dracula, Tokyo Night, Nord, Catppuccin Mocha, Vaporwave, Light, Gruvbox Dark and Ayu Dark. The choice is the `colorTheme` key, which settings sync carries; a theme another machine has and this one doesn't shows as Quadrant without changing the key.

Your own themes are JSON files in the data folder's `themes` folder. "Open themes folder" makes it, puts an `example.json` with every color in it while it has no theme, and opens it. A file looks like this; `mode` (`dark` or `light`) and `extends` (a built-in's id, Quadrant unless it says) are optional, and any color it leaves out comes from the theme it extends:

```json
{ "name": "Mine", "mode": "dark", "extends": "dracula", "colors": { "page": "#101010", "primary": "#ff79c6", "scrim": "#00000080" } }
```

Colors are `#rrggbb` or `#rrggbbaa`, under the names `example.json` lists. The folder is read again whenever Settings loads; a file that doesn't load is named under the picker with what is wrong with it. The built-ins are the same format, in `assets/themes`.

## Tray

As in the desktop app, closing the window keeps Quadrant-Native running in the tray, Quadrant ID's background work included; a left click on the icon brings the window back, and its menu has Show/Hide and Quit. Windows and macOS hide the window. On Linux the icon is a StatusNotifierItem (KDE, and GNOME with the AppIndicator extension); Wayland can't hide a window, so closing closes it and the tray opens a new one, with each page starting afresh. Where no tray is available, closing quits. Screenshot, perf and `QUADRANT_NATIVE_BACKGROUND` runs never add the icon.

## Single instance and links

One app runs per data folder, and Quadrant-Native and the desktop app count as one: both claim the same socket through quadrant-host's `single_instance`. A second launch of either hands its links to the running one over a local socket (a named pipe on Windows) and exits; the running one comes back from the tray, unminimized, centred (Windows only: gpui can't move a window on macOS, and Wayland leaves it to the compositor) and in front, then opens them. The socket's name hashes the home folder and the data folder, so a `--data-dir` sandbox runs beside the real app. Screenshot and perf runs don't take part. An app that starts its successor (an update, a switch) lets go of the socket first, and the successor, marked by `QUADRANT_LAUNCH=handoff`, waits to take it over rather than handing its links back.

Links follow the desktop app's `deepLinks.ts` (`src/deep_link.rs`, with its tests): `modrinth://mod/<slug>` (also `resourcepack` and `shader`, and a wrapped `modrinth://https://modrinth.com/...`), `curseforge://install?addonId=<id>[&fileId=<id>]` and `quadrantnext://modrinth|curseforge?modId=<id>[&fileId=<id>]` open the mod's install page on Search, installing that file when there is a `fileId`; `quadrantnext://modpack?code=<code>` and `https://usequadrant.dev/modpack/<code>` open the Quadrant Share import on Apply; `quadrantnext://login` finishes a Quadrant ID sign-in whose state matches. They arrive as arguments at launch, from a second launch, and on macOS through the system's open events. The packages register the three schemes: the NSIS setup under `HKCU\Software\Classes`, the .app's `CFBundleURLTypes`, and the desktop file's `x-scheme-handler` MIME types. To try one without registering anything, pass it to a sandboxed run: `cargo run -- modrinth://mod/sodium --data-dir ... --mc-folder ...`.

## Launch at login

Settings' "Launch at login" registers the app the way the desktop app's autostart plugin does, through the same `auto-launch` crate: a `Run` value under HKCU on Windows, a LaunchAgent on macOS, an XDG autostart desktop file on Linux, each starting it with `--autostart`, which opens it in the tray. The switch shows what the OS has, not a setting, and turning it off on Windows also removes Task Manager's `StartupApproved` note. A `--data-dir` run registers as `Quadrant-Native-<key>` with its own folders, so a sandbox never replaces the real entry; `cargo test autostart -- --ignored` registers and removes a test entry.

## Taskbar progress

An update download, an export and a modpack download show their progress on the app's taskbar button, as the desktop app's do. Windows draws it through `ITaskbarList3`. On Linux the app sends Unity's `com.canonical.Unity.LauncherEntry` signal for `quadrant-native.desktop`, which KDE's task manager and GNOME's Dash to Dock read. macOS shows none yet: Tauri draws its own view into the dock tile, and this app doesn't. Screenshot and perf runs leave the taskbar alone.

## Updates

With the `updater` feature, an installed copy updates itself the way the Tauri app does, through the same update server, and the update channel also switches between the two apps. The `channel` setting is shared with the desktop app, and quadrant-host's `updates` maps it to an app:

| `channel` | App | Server channel |
|---|---|---|
| `stable` (or unset, or unknown) | Quadrant | `stable` |
| `preview` | Quadrant | `preview`, then `stable` |
| `gpui-preview` | Quadrant-Native | `gpui-preview` |
| `gpui-experimental` (less stable) | Quadrant-Native | `gpui-experimental`, which the server answers with `gpui-preview`'s release when that is at least as new |

At launch, and whenever the channel changes in Settings, it asks

```
https://api.usequadrant.dev/api/any/{app}/updates/{channel}/{target}/{arch}/{version}?variant={bundle}
```

with Tauri's names (`windows`, `linux`, `darwin`; `x86_64`, `aarch64`) and its install kind (`nsis`, `appimage`, `deb`, `rpm`, `app`). On a Quadrant-Native channel `{app}` is `quadrant-native` and `{version}` is this one; Quadrant-Native never reads Quadrant's channels. On a Quadrant channel it asks for `quadrant` with version `0.0.0`, which answers with Quadrant's newest release there. The answer is Tauri's updater JSON. A different version counts as an update. The download is checked against the minisign signature in the answer with Quadrant's updater public key, the one in Quadrant's `tauri.conf.json`, before anything is kept. The title bar then shows "Install an update", or "Switch to Quadrant" for Quadrant's release. Pressing it runs the NSIS setup passively (it starts the installed app), writes the AppImage or unpacks the `.app` (over this one, or beside it under Quadrant's name), or installs the deb or rpm through `pkexec`, then starts the installed app and quits.

Each install records its path in the shared config's `appPaths` (machine-local, never synced). A launch whose channel names Quadrant starts Quadrant's recorded install with the same arguments and exits before claiming the single instance or opening a window, and the desktop app does the same the other way round. Sandboxes (`--data-dir`), `--noupdater`, screenshot and perf runs, and copies that aren't installs never redirect, and a redirected launch never redirects again.

The updater stays off for `--noupdater` (for package managers that own updates), for screenshot and perf runs, and for a copy that isn't an install it can replace: a build in `target/`, or a Linux binary no package owns. Checks and failures only log, as in the Tauri app.

For testing, debug builds read `QUADRANT_NATIVE_UPDATE_ENDPOINT` (an update server to ask instead, plain http allowed) and `QUADRANT_NATIVE_BACKGROUND` (open the window without focus). `QUADRANT_UPDATER_PUBKEY` at compile time swaps the public key (quadrant-host reads it). `scripts\update-e2e.ps1` uses all three to run the whole Windows update against a local server and a throwaway key. It installs the old build for this user, lets it download and verify the new one, presses the button through UI Automation, checks the new version took over, refuses a tampered setup and one signed with another key, uninstalls and checks nothing is left. It needs `cargo-packager` and the Tauri CLI from `../quadrant/node_modules`.

## Packaging

[cargo-packager](https://github.com/crabnebula-dev/cargo-packager) 0.11 builds the installers from `[package.metadata.packager]` in `Cargo.toml`, and [cargo-generate-rpm](https://github.com/cat-in-136/cargo-generate-rpm) builds the rpm. Build the binary first, then package it:

```sh
cargo install cargo-packager --locked --version 0.11.8
cargo build --release --locked --features updater
cargo packager --release -f nsis             # Windows: per-user setup in target/release
cargo packager --release -f deb,appimage     # Linux
cargo generate-rpm -s 'version = "26.11.1"' -s 'release = "gpui_preview.1"'   # rpm versions and releases can't contain '-'
cargo packager --release -f app              # macOS; release.yml signs it and makes the dmg
```

If Scoop's NSIS sets `NSISDIR`, unset it for `cargo packager`, or makensis can't find cargo-packager's NSIS plugins.

The icons in `packaging/icons` are Quadrant's (`src-tauri/icons`). `build.rs` embeds `icon.ico` and a version resource in the Windows exe, and gpui's window class loads it for the title bar and the taskbar. On Linux the Wayland app id is `quadrant-native`, the name of the packages' desktop file (`packaging/linux/quadrant-native.desktop`) and of the hicolor icons. Linux builds need `libxkbcommon-dev libwayland-dev libvulkan-dev libfontconfig-dev libfreetype-dev pkg-config`. Build release packages on the oldest distribution you support: the binary needs at least the glibc it was built with (CI uses Ubuntu 24.04).

The NSIS uninstaller removes the app, its Start menu entry and its uninstall entry. Like Tauri's, it keeps `HKCU\Software\MrQuantumOFF (Demir Yerli)\Quadrant-Native`, the install folder it offers next time.

## Releasing

Versions follow Quadrant's `YY.MM.REVISION` with the update channel as the pre-release: `26.11.1-gpui-preview.1`, `26.11.1-gpui-experimental.1`. Only `gpui-preview` and `gpui-experimental` tags start a release; `stable` and `preview` are Quadrant's channels, and the workflow refuses anything else. Set it in `Cargo.toml`, commit, and push the tag `v<version>`. `.github/workflows/release.yml` then checks the tag against `Cargo.toml` and builds Linux x64 and arm64 (deb, AppImage, rpm), Windows x64 and arm64 (NSIS) and macOS arm64 (an ad-hoc signed `.app` in a dmg, plus `.app.tar.gz` for the updater). It signs every updater artifact with Tauri's signer, uploads it all to the release, and assembles `latest.json`. A `gpui-preview` release is a full GitHub release and becomes the latest one, since that channel is Quadrant-Native's stable line; a `gpui-experimental` release is a pre-release. Without `TAURI_SIGNING_PRIVATE_KEY` it skips signing and `latest.json`. When the repository variable `SUBMIT_UPDATES` is `true`, as it is on QuadrantMC/quadrant-gpui, it `PUT`s `latest.json` to `/api/any/quadrant-native/add_updates/{channel}/{tag}`. As with Quadrant's releases, it arrives unpublished, and reaches updaters once RelMan publishes it. Without the variable, nothing is submitted.

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
| `SUBMIT_UPDATES` | unset (`true` here) | `true` submits each release to the update server, for RelMan to publish |
| `RELEASES_REPO` | this repository | optional: another repository whose release carries the downloads |
| `ASSET_BASE_URL` | `https://github.com/<RELEASES_REPO>/releases/download` | optional: where `latest.json` says the files are, as `<base>/<tag>/<file>` |

This repository is public, so its own release assets serve users and the updater, and `RELEASES_REPO`, `RELEASES_REPO_TOKEN` and `ASSET_BASE_URL` stay unset. They remain for serving the files from somewhere else.

`VERSION_UPDATE_TRIGGER_TOKEN` must hold an update-server API key (product `updater`, scope `update`), and RelMan must list `quadrant-native` among its apps to publish its releases. The server (quadrant_api 12.5.5 or later) treats `gpui-preview` as a base channel like `stable`, and `quadrant-native` answers `404 No supported versions` on a channel until its first release there.

## Frame times

The app has to keep up with the display: a frame every refresh, and never less than 120 a second, so the budget is `1000 / max(120, refresh rate)` ms. `scripts\perf.ps1` measures it in one command. It builds a release binary with `perf`, rebuilds a large sandbox (`scripts\sandbox.ps1 -Large`: 300 modpacks, 500 mods in the applied one, about 1000 resource and shader packs) and runs every scenario five times, then prints the median of each statistic.

```powershell
scripts\perf.ps1                                   # every scenario, 5 runs, large sandbox
scripts\perf.ps1 -Scenario scroll-modpacks -Runs 3 -Sandbox small
scripts\perf.ps1 -Placement visible                # maximized on screen, for a final check
```

By default the window is off screen: the primary display's work-area size (the client area a maximized window gets, so the same resolution and scale), placed past the right edge of the virtual desktop and shown without activation. It never takes focus, never appears on screen or in the taskbar, and real input can't reach it, so a run doesn't disturb whoever is using the machine. GPUI's vsync thread still asks it for a frame every refresh: off screen, ticks arrive every 6.94 ms on a 144 Hz display with less jitter than on screen, so `DwmFlush` paces them and GPUI never falls back to sleeping. The app's own work per frame matches: on four scenarios the off-screen medians were within the run-to-run noise of a busy machine of the visible ones (`scroll-modpacks` 129 vs 126 ms p50, `search` 10.4 vs 10.4 ms, `snackbar` 135 vs 121 ms, `idle` 0 vs 0 frames), the gaps following the background load. `Present` does not match: off screen, Windows sometimes holds it back for 10 to 30 ms every few frames, which a visible window never showed, so off screen the whole-frame time, the present intervals and the drop counts are unreliable. The process also opts out of Windows' power throttling, which otherwise slows down processes whose windows are hidden or unfocused.

Each frame is timed from the window procedure, around the `WM_PAINT` that GPUI's vsync thread asks for every refresh. `draw` is the app's work, from the start of the tick to an element GPUI paints after everything else: next-frame callbacks, render, layout, prepaint and paint. `frame` adds GPUI rendering the scene and `Present`. `over` counts frames whose draw alone exceeded the budget; that is the verdict off screen. On screen, a frame presented more than 1.5 refresh periods after the one before also counts as dropped, and `missed` counts refreshes that showed no new frame. The scenarios step once per frame through GPUI (scroll-wheel and mouse-move events, navigation, snackbars, dialogs, window resizes) and invalidate only what the change would: a scroll step notifies its page, so the page redraws every refresh, while a snackbar, a dialog or the pointer redraw what they touch. A refresh with nothing new to draw is not a drop. `idle` leaves each page alone for five seconds and counts what it draws anyway. Results go to `perf-log\runs\` (ignored by git): a JSON file per scenario with every frame, and `summary.txt`. Each run also records how many cores other processes used meanwhile (`load` in the summary) and warns above two: frame times rise with background load, so compare runs taken under similar load.

## Developing against a local quadrant checkout

`quadrant-host` and `quadrant-core` come from a pinned upstream revision. To build against a sibling checkout instead, uncomment the `[patch."https://github.com/QuadrantMC/quadrant"]` block at the end of `Cargo.toml`. It expects the checkout at `../quadrant`.

## Assets

- `assets/fonts/Inter.ttf` is the desktop app's Inter, by the Inter Project Authors, under the SIL Open Font License 1.1.
- `assets/icons/*.svg` are Google's Material Design icons, the set the desktop app draws through `react-icons/md`, under the Apache License 2.0. `node scripts/extract-icons.mjs` regenerates them from a quadrant checkout with its dependencies installed.
- `locales/en.json` is the desktop app's English strings. Copy it again from `quadrant/src/locales/en.json` when the pinned revision moves.
- `packaging/icons` holds Quadrant's app icons, from `quadrant/src-tauri/icons`.

## License

MPL-2.0, like Quadrant.
