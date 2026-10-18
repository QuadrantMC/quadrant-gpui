# AGENTS.md

Quadrant-Native, a native GPUI frontend for Quadrant. The Tauri app in `../quadrant` is the functional and visual spec: `src/components/Pages/*` for screens, `src/tools.ts` for which backend call a UI action makes, `Screenshots/*.png` and `.design-sync/conventions.md` for the look.

## Commands

| Task | Command |
|---|---|
| Build, test | `cargo build`, `cargo test` |
| Lint, format | `cargo clippy --all-targets` with no features, `--features updater`, `screenshot`, `screenshot,updater` and `perf`; `cargo fmt` |
| Fixture folders | `scripts\sandbox.ps1` (writes `%TEMP%\quadrant-gpui-sandbox`) |
| Visual check | `cargo run --features screenshot -- --data-dir <sandbox>\data --mc-folder <sandbox>\mc --screenshot <dir>` |
| Frame times | `scripts\perf.ps1 [-Scenario <name>] [-Runs <n>] [-Sandbox large\|small]` (release build with `perf`, off-screen window; see README) |
| Refresh icons | `node scripts/extract-icons.mjs` |
| Package | `cargo build --release --features updater`, then `cargo packager --release -f <nsis\|deb,appimage\|app>`; `cargo generate-rpm` (README, "Packaging") |
| Update end to end | `scripts\update-e2e.ps1` (installs for this user, then uninstalls; README, "Updates") |

Never run the app or the op test against real data. Without `--data-dir` and `--mc-folder` the app uses the Tauri app's data folder and Minecraft folder, and apply, create and delete change them. Installing a mod or updating one (`install_mod`, `install_remote_file`) also writes the host's shared download cache under the real user cache folder (`%LOCALAPPDATA%\mrquantumoff.dev\QuadrantNextCache`), whatever `--data-dir` says, so verification doesn't install; searching, update checks and registering are fine. Don't capture the screen for visual checks either: the user's other windows end up in the picture. Use `--screenshot`, which renders the window's own frames.

Builds embed `QUADRANT_OAUTH2_CLIENT_ID`, `QUADRANT_OAUTH2_CLIENT_SECRET` and `QUADRANT_API_KEY` whenever the environment sets them, and this machine's does. So a normal sandbox run can really sign in (the browser may complete it without a click), share and sync. A `--data-dir` keeps its own keyring entry, so the sandbox never sees the user's account, but don't press Sign in, Share or Sync there. `--screenshot` drops the credentials. With experimental features on and no `prismLauncherFolder`, the host finds the real Prism Launcher install; `scripts\sandbox.ps1` points it into the sandbox.

Worktrees share `refs/stash`, so `git stash pop` can take another agent's work. Use WIP commits on your branch instead.

## Architecture rules

- **`Backend::run` is the only tokio seam.** `Backend` (a gpui `Global`) owns the `QuadrantHost` and the tokio runtime. Views call `Backend::global(cx).run(cx, |host| async move { ... })` and await the gpui `Task` it returns. Nothing else spawns on tokio or calls the host directly; the one exception is `Backend::http_client`, which fetches gpui's remote images on the same runtime. Host events come from `Backend::subscribe_events`; `events.rs` shows them in the snackbar.
- **`Loadable<T>` for every fetched datum.** `Idle`, `Loading`, `Ready(T)`, `Failed(SharedString)`. No `is_loading` flag beside an `Option<T>` and an error string. `Loadable::from_result` turns a host error into English through `i18n::describe`.
- **Feedback goes through `snackbar::show(cx, kind, message)`.** One snackbar at a time, bottom left, as in the desktop app: `Success` after a finished action, `Error` with `i18n::describe` text for a failure. Don't use gpui-component's `push_notification`; it draws its own toast.
- **The route table drives navigation.** `Route::ALL` is the rail, in order; `label()` and `icon()` are matches on `Route`. Adding a page means a `Route` variant, its arms, and an arm in `pages::build`. `History` is a pure back/forward stack with unit tests.
- **Pages are lazy, kept-alive entities.** `QuadrantApp::page` builds a route's view on first visit through `pages::build` and keeps it, so page state survives navigation.
- **`Session::global(cx)` is the Quadrant ID sign-in every page shares.** Pages observe it; it starts the host's background workers once someone is signed in and stops them after signing out.
- **The updater is its own module behind the `updater` feature.** `updater.rs` is the gpui side (`Updater` global, title bar button, channel picker); `updater/protocol.rs` is the pure Tauri protocol (endpoint, answer, signature check) and is where tests go; `updater/install.rs` applies an update per install kind. Its network calls run inside `Backend::run`. Nothing is installed whose minisign signature doesn't check out against `protocol::PUBKEY`, Quadrant's key.
- **One change at a time per page.** A page's mutations are an `Op` enum whose `perform(host)` makes the host calls. The page holds `running: Option<Op>`, disables actions while one runs, then notifies and reloads. Test ops against a temp folder (see `ops_change_the_modpacks_folder`).

## Releases and updates

- Identity: product `Quadrant-Native`, binary `quadrant-native`, bundle identifier `dev.mrquantumoff.quadrant-native`, update app `quadrant-native`, Wayland app id `quadrant-native` (the desktop file's name). Only the data folder is shared with the Tauri app.
- Versions are Quadrant's `YY.MM.REVISION-<channel>` in `Cargo.toml`; the release tag is `v` plus that version, and CI refuses a tag that doesn't match.
- Never call the production `add_updates` endpoint, create tags or releases, or set secrets by hand; `release.yml` does that from a tag. GETs to the production updates endpoint are harmless.
- Test updates with `scripts\update-e2e.ps1`, never against production. The debug-only `QUADRANT_NATIVE_UPDATE_ENDPOINT` and `QUADRANT_NATIVE_BACKGROUND` exist for it; release builds ignore both.
- The Windows exe is GUI-subsystem: run it from scripts through a pipe or redirection, or PowerShell won't wait for it or see its output. Set `QUADRANT_NATIVE_NO_DIALOG` whenever a test or script starts it, and never start it with bad arguments just to see the error dialog; `console.rs` has unit tests for that path.
- Windows that only need to exist in a test (update runs, sandbox runs) must not take the user's focus or screen: set `QUADRANT_NATIVE_BACKGROUND`, hide the console, move the window off-screen, as the e2e script does. Use `--screenshot` for anything visual.

## Look

- All visual tokens live in `theme.rs`: the Tailwind slate palette as named roles (`PAGE`, `SURFACE`, `CONTROL`, `TEXT_MUTED`, ...), Inter, and the shapes (`pill_button`, `toolbar_button`, `window_button`, `rail_button`, `badge`, `title_pill`, `dialog_surface`). Views name a role or a helper, never a hex value.
- The window's rem size is the desktop app's 16.8px, so gpui's Tailwind-named helpers (`h_10`, `px_4`, `text_2xl`) match the React classes one to one. Port a React class list by using the same names.
- Icons are `MdIcon` (Material Design, as in the desktop app). Add one by listing it in `scripts/extract-icons.mjs` and the `md_icons!` table in `assets.rs`.
- Strings come from `locales/en.json` through `i18n::t` and `t_with`, keyed like the desktop app.
- Remote images (mod icons, screenshots) are plain `img(url)`. gpui fetches them through `Backend::http_client`, which `main` installs and which runs on the backend's runtime.

## gpui-component 0.7.3 gotchas

- A `ButtonCustomVariant` paints its resting color at 20% strength. `theme::filled` sets the background explicitly; build buttons through the theme helpers.
- `Button::icon` resizes the icon to the button's size. For a large icon in a round button, add the `Icon` as a child.
- Opening or closing a dialog reads the window's `Root`. Inside `WindowHandle::update` the root is already borrowed, so defer with `window.defer`.
- A `Button` sets its label's text size from its `Size`, overriding a text size on the button itself. Set the size on the child you pass in, as the theme helpers do.
- `Input` and `Select` draw their own border and background unless `.appearance(false)`.
- `Window::render_to_image` exists only with gpui's `test-support` feature, which the `screenshot` feature turns on.
- The `--screenshot` window is hidden, so the platform never asks it for frames: each capture draws one. Work a view defers to after it renders (a dialog it opens from `render`) needs an extra `draw` before the capture.

## Frame times

- The target is a frame every refresh, at least 120 a second: a budget of `1000 / max(120, Hz)` ms. `scripts\perf.ps1` is the ruler; its scenarios, durations and the dropped-frame rule are frozen, since changing them invalidates every earlier number. Compare a change against a run of the same command at its parent commit, on the same sandbox.
- Judge a change by `draw` and `over` (frames whose draw exceeded the budget). Off screen, Windows sometimes stalls `Present`, so `frame`, `dropped` and `missed` count only in a visible run.
- Runs are off screen by default and never take focus. Don't use `-Placement visible` (a maximized, focused window over the user's screen) without asking the user first; it is for a final confirmation only.
- Don't build or run anything else while it measures; other CPU load slows frames down. The summary's `load` is the cores other processes used; a run that warns about load, real input or lost focus was disturbed, so run it again or compare it only with runs under similar load.
- `perf::frame_drawn` in `QuadrantApp::render` marks a tick as drawn. That relies on no view being `cached()`; a cached root view would hide frames from the harness.
- On Windows, GPUI draws inside `WM_PAINT`, which its vsync thread requests every refresh through `DwmFlush` even when nothing changed. An unfocused window's animations are capped at 30 fps (`inactive_frame_interval`); perf mode turns that off.
