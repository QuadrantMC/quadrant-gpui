# AGENTS.md

Native GPUI frontend for Quadrant. The Tauri app in `../quadrant` is the functional and visual spec: `src/components/Pages/*` for screens, `src/tools.ts` for which backend call a UI action makes, `Screenshots/*.png` and `.design-sync/conventions.md` for the look.

## Commands

| Task | Command |
|---|---|
| Build, test | `cargo build`, `cargo test` |
| Lint, format | `cargo clippy --all-targets --features screenshot` and `--features perf`, `cargo fmt` |
| Fixture folders | `scripts\sandbox.ps1` (writes `%TEMP%\quadrant-gpui-sandbox`) |
| Visual check | `cargo run --features screenshot -- --data-dir <sandbox>\data --mc-folder <sandbox>\mc --screenshot <dir>` |
| Frame times | `scripts\perf.ps1 [-Scenario <name>] [-Runs <n>] [-Sandbox large\|small]` (release build with `perf`; see README) |
| Refresh icons | `node scripts/extract-icons.mjs` |

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
- **One change at a time per page.** A page's mutations are an `Op` enum whose `perform(host)` makes the host calls. The page holds `running: Option<Op>`, disables actions while one runs, then notifies and reloads. Test ops against a temp folder (see `ops_change_the_modpacks_folder`).

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
- Don't build or run anything else while it measures; other CPU load shows up as dropped frames. A run that warns about real input or lost focus was disturbed; run it again.
- `perf::frame_drawn` in `QuadrantApp::render` marks a tick as drawn. That relies on no view being `cached()`; a cached root view would hide frames from the harness.
- On Windows, GPUI draws inside `WM_PAINT`, which its vsync thread requests every refresh through `DwmFlush` even when nothing changed. An unfocused window's animations are capped at 30 fps (`inactive_frame_interval`); perf mode turns that off.
