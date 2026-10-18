# AGENTS.md

Native GPUI frontend for Quadrant. The Tauri app in `../quadrant` is the functional and visual spec: `src/components/Pages/*` for screens, `src/tools.ts` for which backend call a UI action makes, `Screenshots/*.png` and `.design-sync/conventions.md` for the look.

## Commands

| Task | Command |
|---|---|
| Build, test | `cargo build`, `cargo test` |
| Lint, format | `cargo clippy --all-targets --features screenshot`, `cargo fmt` |
| Fixture folders | `scripts\sandbox.ps1` (writes `%TEMP%\quadrant-gpui-sandbox`) |
| Visual check | `cargo run --features screenshot -- --data-dir <sandbox>\data --mc-folder <sandbox>\mc --screenshot <dir>` |
| Refresh icons | `node scripts/extract-icons.mjs` |

Never run the app or the op test against real data. Without `--data-dir` and `--mc-folder` the app uses the Tauri app's data folder and Minecraft folder, and apply, create and delete change them. Don't capture the screen for visual checks either: the user's other windows end up in the picture. Use `--screenshot`, which renders the window's own frames.

## Architecture rules

- **`Backend::run` is the only tokio seam.** `Backend` (a gpui `Global`) owns the `QuadrantHost` and the tokio runtime. Views call `Backend::global(cx).run(cx, |host| async move { ... })` and await the gpui `Task` it returns. Nothing else spawns on tokio or calls the host directly. Host events come from `Backend::subscribe_events`; `events.rs` shows them in the snackbar.
- **`Loadable<T>` for every fetched datum.** `Idle`, `Loading`, `Ready(T)`, `Failed(SharedString)`. No `is_loading` flag beside an `Option<T>` and an error string. `Loadable::from_result` turns a host error into English through `i18n::describe`.
- **Feedback goes through `snackbar::show(cx, kind, message)`.** One snackbar at a time, bottom left, as in the desktop app: `Success` after a finished action, `Error` with `i18n::describe` text for a failure. Don't use gpui-component's `push_notification`; it draws its own toast.
- **The route table drives navigation.** `Route::ALL` is the rail, in order; `label()` and `icon()` are matches on `Route`. Adding a page means a `Route` variant, its arms, and an arm in `pages::build`. `History` is a pure back/forward stack with unit tests.
- **Pages are lazy, kept-alive entities.** `QuadrantApp::page` builds a route's view on first visit through `pages::build` and keeps it, so page state survives navigation. Unported routes return the shared `Placeholder`; replace their arm with the real page.
- **One change at a time per page.** A page's mutations are an `Op` enum whose `perform(host)` makes the host calls. The page holds `running: Option<Op>`, disables actions while one runs, then notifies and reloads. Test ops against a temp folder (see `ops_change_the_modpacks_folder`).

## Look

- All visual tokens live in `theme.rs`: the Tailwind slate palette as named roles (`PAGE`, `SURFACE`, `CONTROL`, `TEXT_MUTED`, ...), Inter, and the shapes (`pill_button`, `toolbar_button`, `window_button`, `rail_button`, `badge`, `title_pill`, `dialog_surface`). Views name a role or a helper, never a hex value.
- The window's rem size is the desktop app's 16.8px, so gpui's Tailwind-named helpers (`h_10`, `px_4`, `text_2xl`) match the React classes one to one. Port a React class list by using the same names.
- Icons are `MdIcon` (Material Design, as in the desktop app). Add one by listing it in `scripts/extract-icons.mjs` and the `md_icons!` table in `assets.rs`.
- Strings come from `locales/en.json` through `i18n::t` and `t_with`, keyed like the desktop app.

## gpui-component 0.7.3 gotchas

- A `ButtonCustomVariant` paints its resting color at 20% strength. `theme::filled` sets the background explicitly; build buttons through the theme helpers.
- `Button::icon` resizes the icon to the button's size. For a large icon in a round button, add the `Icon` as a child.
- Opening or closing a dialog reads the window's `Root`. Inside `WindowHandle::update` the root is already borrowed, so defer with `window.defer`.
- `Input` and `Select` draw their own border and background unless `.appearance(false)`.
- `Window::render_to_image` exists only with gpui's `test-support` feature, which the `screenshot` feature turns on.
