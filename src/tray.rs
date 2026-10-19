//! The desktop app's tray (`lib.rs`, "Initializing tray..."): closing the
//! window hides it and Quadrant-Native keeps running, its background work
//! included, until the tray's Quit. A left click on the icon brings the
//! window back; its menu offers "Show/Hide" and "Quit", as the desktop app's
//! does.
//!
//! Windows and macOS hide the window itself. Wayland has no way for a
//! client to hide its window, so on Linux closing it closes it and the tray
//! opens a new one. Without a tray (no StatusNotifierWatcher on the Linux
//! desktop, or the tray failed to start) closing quits, so the app never
//! lingers out of reach.

use std::rc::Rc;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use gpui::{App, AsyncApp, Global, QuitMode, Window, WindowHandle};
use gpui_component::Root;

/// The desktop app's tray menu items, untranslated there too.
const SHOW_HIDE: &str = "Show/Hide";
const QUIT: &str = "Quit";

/// What the tray asks of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// A left click on the icon.
    Show,
    /// "Show/Hide"
    Toggle,
    /// "Quit"
    Quit,
}

/// Opens a new main window, for Linux, where hiding closes it.
type Reopen = Rc<dyn Fn(&mut App) -> Result<WindowHandle<Root>>>;

/// The tray and the window it shows and hides.
struct Tray {
    /// Removing the icon when dropped, which is never before the app quits.
    _icon: platform::Icon,
    /// The main window; `None` on Linux while it is closed to the tray.
    window: Option<WindowHandle<Root>>,
    hidden: bool,
    #[cfg_attr(not(target_os = "linux"), expect(dead_code))]
    reopen: Reopen,
}

impl Global for Tray {}

/// Puts the icon in the tray, so closing `window` hides it from now on.
/// When there is no tray to put it in, closing quits as before.
pub fn install(
    window: WindowHandle<Root>,
    reopen: impl Fn(&mut App) -> Result<WindowHandle<Root>> + 'static,
    cx: &mut App,
) {
    let (commands, mut received) = futures::channel::mpsc::unbounded();
    let icon = match platform::spawn(commands) {
        Ok(icon) => icon,
        Err(error) => {
            log::warn!("No tray icon, so closing the window quits: {error:#}");
            return;
        }
    };
    // The app outlives a window closed to the tray on Linux.
    cx.set_quit_mode(QuitMode::Explicit);
    cx.set_global(Tray {
        _icon: icon,
        window: Some(window),
        hidden: false,
        reopen: Rc::new(reopen),
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(command) = received.next().await {
            cx.update(|cx| run(command, cx));
        }
    })
    .detach();
}

/// Makes the platform's close (Alt+F4, the taskbar, the macOS close button,
/// the compositor) do what the title bar's close does.
pub fn watch_close(window: WindowHandle<Root>, cx: &mut App) {
    let _ = window.update(cx, |_, window, cx| {
        window.on_window_should_close(cx, |window, cx| {
            if !cx.has_global::<Tray>() {
                cx.quit();
                return true;
            }
            if cfg!(target_os = "linux") {
                // The window closes; the tray opens a new one.
                forget_window(cx);
                return true;
            }
            hide(window, cx);
            false
        });
    });
}

/// The title bar's close: hides the window into the tray, or quits when
/// there is none, as the desktop app's close button does.
pub fn close(window: &mut Window, cx: &mut App) {
    if cx.has_global::<Tray>() {
        hide(window, cx);
    } else {
        cx.quit();
    }
}

fn run(command: Command, cx: &mut App) {
    let hidden = cx.global::<Tray>().hidden;
    match command {
        Command::Quit => cx.quit(),
        Command::Show => show(cx),
        Command::Toggle if hidden => show(cx),
        Command::Toggle => {
            if let Some(window) = cx.global::<Tray>().window {
                let _ = window.update(cx, |_, window, cx| hide(window, cx));
            }
        }
    }
}

fn hide(window: &mut Window, cx: &mut App) {
    #[cfg(target_os = "linux")]
    {
        forget_window(cx);
        window.remove_window();
    }
    #[cfg(not(target_os = "linux"))]
    {
        platform::hide(window, cx);
        cx.global_mut::<Tray>().hidden = true;
    }
}

#[cfg(target_os = "linux")]
fn forget_window(cx: &mut App) {
    let tray = cx.global_mut::<Tray>();
    tray.window = None;
    tray.hidden = true;
}

#[cfg(not(target_os = "linux"))]
fn forget_window(_: &mut App) {}

fn show(cx: &mut App) {
    #[cfg(target_os = "linux")]
    if cx.global::<Tray>().window.is_none() {
        let reopen = cx.global::<Tray>().reopen.clone();
        match reopen(cx) {
            Ok(window) => {
                watch_close(window, cx);
                cx.global_mut::<Tray>().window = Some(window);
            }
            Err(error) => {
                log::error!("Failed to open the window again: {error:#}");
                return;
            }
        }
    }
    let Some(window) = cx.global::<Tray>().window else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| platform::show(window, cx));
    cx.global_mut::<Tray>().hidden = false;
}

/// The app's icon as straight RGBA rows.
fn icon_rgba() -> Result<(Vec<u8>, u32, u32)> {
    let image = image::load_from_memory(include_bytes!("../packaging/icons/32x32.png"))
        .context("the tray icon")?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Ok((image.into_raw(), width, height))
}

/// ARGB32 in network byte order, as a StatusNotifierItem's pixmap wants it.
#[cfg_attr(not(any(target_os = "linux", test)), expect(dead_code))]
fn argb(rgba: &[u8]) -> Vec<u8> {
    let (pixels, _) = rgba.as_chunks::<4>();
    pixels
        .iter()
        .flat_map(|&[r, g, b, a]| [a, r, g, b])
        .collect()
}

#[cfg(any(windows, target_os = "macos"))]
mod platform {
    use anyhow::Result;
    use futures::channel::mpsc::UnboundedSender;
    use gpui::{App, Window};
    use tray_icon::{
        MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{Menu, MenuEvent, MenuItem},
    };

    use super::{Command, QUIT, SHOW_HIDE};

    pub type Icon = TrayIcon;

    pub fn spawn(commands: UnboundedSender<Command>) -> Result<TrayIcon> {
        let show = MenuItem::with_id("show", SHOW_HIDE, true, None);
        let quit = MenuItem::with_id("quit", QUIT, true, None);
        let menu = Menu::with_items(&[&show, &quit])?;
        let (rgba, width, height) = super::icon_rgba()?;
        let menu_commands = commands.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id().0.as_str() {
                "show" => Command::Toggle,
                "quit" => Command::Quit,
                _ => return,
            };
            let _ = menu_commands.unbounded_send(command);
        }));
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = commands.unbounded_send(Command::Show);
            }
        }));
        Ok(TrayIconBuilder::new()
            .with_icon(tray_icon::Icon::from_rgba(rgba, width, height)?)
            .with_tooltip(crate::PRODUCT_NAME)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?)
    }

    #[cfg(windows)]
    fn hwnd(window: &Window) -> Option<windows::Win32::Foundation::HWND> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => {
                Some(windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _))
            }
            _ => None,
        }
    }

    /// Takes the window off the screen and the taskbar.
    pub fn hide(window: &mut Window, cx: &mut App) {
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
            let _ = cx;
            if let Some(hwnd) = hwnd(window) {
                let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = window;
            cx.hide();
        }
    }

    /// Puts the window back, restored and in front.
    pub fn show(window: &mut Window, cx: &mut App) {
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{SW_SHOW, ShowWindow};
            let _ = cx;
            if let Some(hwnd) = hwnd(window) {
                let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
            }
        }
        #[cfg(target_os = "macos")]
        cx.activate(true);
        window.activate_window();
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use anyhow::Result;
    use futures::channel::mpsc::UnboundedSender;
    use gpui::{App, Window};
    use ksni::{
        MenuItem,
        blocking::{Handle, TrayMethods as _},
        menu::StandardItem,
    };

    use super::{Command, QUIT, SHOW_HIDE};

    pub type Icon = Handle<StatusNotifier>;

    /// The StatusNotifierItem. ksni calls it on its own thread; it only
    /// passes commands on.
    pub struct StatusNotifier {
        commands: UnboundedSender<Command>,
        icon: ksni::Icon,
    }

    impl StatusNotifier {
        fn send(&self, command: Command) {
            let _ = self.commands.unbounded_send(command);
        }
    }

    impl ksni::Tray for StatusNotifier {
        fn id(&self) -> String {
            crate::WAYLAND_APP_ID.into()
        }

        fn title(&self) -> String {
            crate::PRODUCT_NAME.into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            vec![self.icon.clone()]
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(Command::Show);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let item = |label: &str, command: Command| {
                StandardItem {
                    label: label.into(),
                    activate: Box::new(move |this: &mut Self| this.send(command)),
                    ..Default::default()
                }
                .into()
            };
            vec![item(SHOW_HIDE, Command::Toggle), item(QUIT, Command::Quit)]
        }
    }

    /// Fails when no StatusNotifierWatcher runs, e.g. GNOME without its
    /// AppIndicator extension: there is no tray to come back from.
    pub fn spawn(commands: UnboundedSender<Command>) -> Result<Handle<StatusNotifier>> {
        let (rgba, width, height) = super::icon_rgba()?;
        let tray = StatusNotifier {
            commands,
            icon: ksni::Icon {
                width: width as i32,
                height: height as i32,
                data: super::argb(&rgba),
            },
        };
        // A Flatpak sandbox may not own a well-known D-Bus name.
        let sandboxed = std::path::Path::new("/.flatpak-info").exists();
        Ok(tray.disable_dbus_name(sandboxed).spawn()?)
    }

    pub fn show(window: &mut Window, _: &mut App) {
        window.activate_window();
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod platform {
    use anyhow::{Result, bail};
    use futures::channel::mpsc::UnboundedSender;
    use gpui::{App, Window};

    use super::Command;

    pub type Icon = ();

    pub fn spawn(_: UnboundedSender<Command>) -> Result<()> {
        bail!("no tray on this platform")
    }

    pub fn hide(_: &mut Window, _: &mut App) {}

    pub fn show(window: &mut Window, _: &mut App) {
        window.activate_window();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_decodes() {
        let (rgba, width, height) = icon_rgba().unwrap();
        assert_eq!((width, height), (32, 32));
        assert_eq!(rgba.len(), 32 * 32 * 4);
    }

    #[test]
    fn pixmaps_put_alpha_first() {
        assert_eq!(argb(&[1, 2, 3, 4, 5, 6, 7, 8]), [4, 1, 2, 3, 8, 5, 6, 7]);
    }
}
