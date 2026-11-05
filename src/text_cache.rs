//! Shaped text that outlives a frame. GPUI keeps a line's shaping only while
//! each frame uses it, so a kept-alive page coming back, or anything showing
//! text it showed a while ago, shapes all of it again through the platform.
//! `platform` wraps the platform so its text system remembers the lines it
//! shaped recently, whichever frame asked for them. Everything else passes
//! through untouched.

use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use futures::channel::oneshot;
use gpui::{
    Action, ActivationPolicy, ActivityGuard, AnyWindowHandle, AppLifecyclePhase,
    BackgroundExecutor, Bounds, ClipboardItem, ClipboardReadError, CursorStyle, DevicePixels, Font,
    FontId, FontMetrics, FontRun, ForegroundExecutor, GlyphId, GraphicalEnvironment, Hsla, Keymap,
    LineLayout, Menu, MenuItem, MissingGlyphSink, OwnedMenu, PathPromptOptions, Pixels, Platform,
    PlatformDisplay, PlatformGestures, PlatformInfo, PlatformKeyboardLayout,
    PlatformKeyboardMapper, PlatformTextSystem, PlatformWindow, PowerMode, RenderGlyphParams,
    ScreenCaptureSource, ShapedRun, Size, SystemNotification, SystemNotificationResponse, Task,
    TextRenderingMode, ThermalState, WindowAppearance, WindowButtonLayout, WindowParams,
    WindowingRequest,
};
use smallvec::SmallVec;

/// `inner`, with its text system's line shaping cached across frames.
pub fn platform(inner: Rc<dyn Platform>) -> Rc<dyn Platform> {
    let text = Arc::new(CachedTextSystem {
        lines: Mutex::default(),
        inner: inner.text_system(),
    });
    Rc::new(CachingPlatform { inner, text })
}

/// How many distinct lines a generation holds before the older one is
/// dropped: a few pages' worth, so every page's text survives a round of
/// page switches while memory stays bounded.
const GENERATION: usize = 4096;

#[derive(PartialEq, Eq, Hash)]
struct Key {
    text: String,
    font_size: Pixels,
    runs: Vec<FontRun>,
}

/// The lines shaped lately, in two generations: a line found in the older
/// one moves to the newer, and when the newer fills up the older goes.
#[derive(Default)]
struct Lines {
    current: HashMap<Key, Arc<LineLayout>>,
    previous: HashMap<Key, Arc<LineLayout>>,
}

impl Lines {
    fn get(&mut self, key: &Key) -> Option<Arc<LineLayout>> {
        if let Some(line) = self.current.get(key) {
            return Some(line.clone());
        }
        let (key, line) = self.previous.remove_entry(key)?;
        self.insert(key, line.clone());
        Some(line)
    }

    fn insert(&mut self, key: Key, line: Arc<LineLayout>) {
        if self.current.len() >= GENERATION {
            self.previous = std::mem::take(&mut self.current);
        }
        self.current.insert(key, line);
    }
}

fn copy(line: &LineLayout) -> LineLayout {
    LineLayout {
        font_size: line.font_size,
        width: line.width,
        ascent: line.ascent,
        descent: line.descent,
        runs: line
            .runs
            .iter()
            .map(|run| ShapedRun {
                font_id: run.font_id,
                glyphs: run.glyphs.clone(),
            })
            .collect(),
        len: line.len,
    }
}

struct CachedTextSystem {
    lines: Mutex<Lines>,
    inner: Arc<dyn PlatformTextSystem>,
}

impl CachedTextSystem {
    fn lines(&self) -> std::sync::MutexGuard<'_, Lines> {
        self.lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl PlatformTextSystem for CachedTextSystem {
    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        let key = Key {
            text: text.to_owned(),
            font_size,
            runs: runs.to_vec(),
        };
        if let Some(line) = self.lines().get(&key) {
            return copy(&line);
        }
        let line = self.inner.layout_line(text, font_size, runs);
        self.lines().insert(key, Arc::new(copy(&line)));
        line
    }

    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        // A new font can change which font shapes a character.
        *self.lines() = Lines::default();
        self.inner.add_fonts(fonts)
    }

    fn set_missing_glyph_sink(&self, sink: Option<Arc<dyn MissingGlyphSink>>) {
        self.inner.set_missing_glyph_sink(sink)
    }

    fn all_font_names(&self) -> Vec<String> {
        self.inner.all_font_names()
    }

    fn font_id(&self, descriptor: &Font) -> Result<FontId> {
        self.inner.font_id(descriptor)
    }

    fn prewarm_fonts(&self, font_ids: &[FontId]) {
        self.inner.prewarm_fonts(font_ids)
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.inner.font_metrics(font_id)
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        self.inner.typographic_bounds(font_id, glyph_id)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.inner.advance(font_id, glyph_id)
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.inner.glyph_for_char(font_id, ch)
    }

    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        self.inner.glyph_raster_bounds(params)
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        raster_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        self.inner.rasterize_glyph(params, raster_bounds)
    }

    fn recommended_rendering_mode(&self, font_id: FontId, font_size: Pixels) -> TextRenderingMode {
        self.inner.recommended_rendering_mode(font_id, font_size)
    }

    fn glyph_dilation_for_color(&self, color: Hsla) -> u8 {
        self.inner.glyph_dilation_for_color(color)
    }
}

struct CachingPlatform {
    inner: Rc<dyn Platform>,
    text: Arc<CachedTextSystem>,
}

/// Every method but `text_system` forwards, the defaulted ones too, so the
/// platform's own overrides still apply.
impl Platform for CachingPlatform {
    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.text.clone()
    }

    fn platform_info(&self) -> PlatformInfo {
        self.inner.platform_info()
    }

    fn power_mode(&self) -> PowerMode {
        self.inner.power_mode()
    }

    fn on_power_mode_change(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_power_mode_change(callback)
    }

    fn background_executor(&self) -> BackgroundExecutor {
        self.inner.background_executor()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.inner.foreground_executor()
    }

    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        self.inner.run(on_finish_launching)
    }

    fn quit(&self) {
        self.inner.quit()
    }

    fn set_initial_windowing(&self, request: WindowingRequest) {
        self.inner.set_initial_windowing(request)
    }

    fn request_windowing(&self, request: WindowingRequest) -> Task<anyhow::Result<()>> {
        self.inner.request_windowing(request)
    }

    fn restart(&self, binary_path: Option<PathBuf>, arguments: Vec<OsString>) {
        self.inner.restart(binary_path, arguments)
    }

    fn activate(&self, ignoring_other_apps: bool) {
        self.inner.activate(ignoring_other_apps)
    }

    fn set_activation_policy(&self, policy: ActivationPolicy) {
        self.inner.set_activation_policy(policy)
    }

    fn hide(&self) {
        self.inner.hide()
    }

    fn hide_other_apps(&self) {
        self.inner.hide_other_apps()
    }

    fn unhide_other_apps(&self) {
        self.inner.unhide_other_apps()
    }

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        self.inner.displays()
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.inner.primary_display()
    }

    fn on_displays_changed(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_displays_changed(callback)
    }

    fn active_window(&self) -> Option<AnyWindowHandle> {
        self.inner.active_window()
    }

    fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        self.inner.window_stack()
    }

    fn is_screen_capture_supported(&self) -> bool {
        self.inner.is_screen_capture_supported()
    }

    fn screen_capture_sources(
        &self,
    ) -> oneshot::Receiver<anyhow::Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
        self.inner.screen_capture_sources()
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>> {
        self.inner.open_window(handle, options)
    }

    fn window_appearance(&self) -> WindowAppearance {
        self.inner.window_appearance()
    }

    fn set_window_appearance(&self, appearance: Option<WindowAppearance>) {
        self.inner.set_window_appearance(appearance)
    }

    fn button_layout(&self) -> Option<WindowButtonLayout> {
        self.inner.button_layout()
    }

    fn open_url(&self, url: &str) {
        self.inner.open_url(url)
    }

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.inner.on_open_urls(callback)
    }

    fn register_url_scheme(&self, url: &str) -> Task<Result<()>> {
        self.inner.register_url_scheme(url)
    }

    fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        self.inner.prompt_for_paths(options)
    }

    fn prompt_for_new_path(
        &self,
        directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        self.inner.prompt_for_new_path(directory, suggested_name)
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        self.inner.can_select_mixed_files_and_dirs()
    }

    fn reveal_path(&self, path: &Path) {
        self.inner.reveal_path(path)
    }

    fn open_with_system(&self, path: &Path) {
        self.inner.open_with_system(path)
    }

    fn on_quit(&self, callback: Box<dyn FnMut() -> bool>) {
        self.inner.on_quit(callback)
    }

    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_reopen(callback)
    }

    fn on_system_sleep(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_system_sleep(callback)
    }

    fn on_system_wake(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_system_wake(callback)
    }

    fn on_app_lifecycle(&self, callback: Box<dyn FnMut(AppLifecyclePhase)>) {
        self.inner.on_app_lifecycle(callback)
    }

    fn on_memory_warning(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_memory_warning(callback)
    }

    fn gestures(&self) -> Option<Rc<dyn PlatformGestures>> {
        self.inner.gestures()
    }

    fn set_menus(&self, menus: Vec<Menu>, keymap: &Keymap) {
        self.inner.set_menus(menus, keymap)
    }

    fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
        self.inner.get_menus()
    }

    fn set_dock_menu(&self, menu: Vec<MenuItem>, keymap: &Keymap) {
        self.inner.set_dock_menu(menu, keymap)
    }

    fn perform_dock_menu_action(&self, action: usize) {
        self.inner.perform_dock_menu_action(action)
    }

    fn add_recent_document(&self, path: &Path) {
        self.inner.add_recent_document(path)
    }

    fn update_jump_list(
        &self,
        menus: Vec<MenuItem>,
        entries: Vec<SmallVec<[PathBuf; 2]>>,
    ) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
        self.inner.update_jump_list(menus, entries)
    }

    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>) {
        self.inner.on_app_menu_action(callback)
    }

    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_will_open_app_menu(callback)
    }

    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>) {
        self.inner.on_validate_app_menu_command(callback)
    }

    fn thermal_state(&self) -> ThermalState {
        self.inner.thermal_state()
    }

    fn on_thermal_state_change(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_thermal_state_change(callback)
    }

    fn prevent_idle_sleep(&self, reason: &str) -> Task<Result<ActivityGuard>> {
        self.inner.prevent_idle_sleep(reason)
    }

    fn set_app_identity(&self, identifier: &str, name: &str) {
        self.inner.set_app_identity(identifier, name)
    }

    fn show_system_notification(&self, notification: SystemNotification) {
        self.inner.show_system_notification(notification)
    }

    fn dismiss_system_notification(&self, tag: &str) {
        self.inner.dismiss_system_notification(tag)
    }

    fn on_system_notification_response(
        &self,
        callback: Box<dyn FnMut(SystemNotificationResponse)>,
    ) {
        self.inner.on_system_notification_response(callback)
    }

    fn compositor_name(&self) -> &'static str {
        self.inner.compositor_name()
    }

    fn graphical_environment(&self) -> Option<GraphicalEnvironment> {
        self.inner.graphical_environment()
    }

    fn app_path(&self) -> Result<PathBuf> {
        self.inner.app_path()
    }

    fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        self.inner.path_for_auxiliary_executable(name)
    }

    fn set_cursor_style(&self, style: CursorStyle) {
        self.inner.set_cursor_style(style)
    }

    fn hide_cursor_until_mouse_moves(&self) {
        self.inner.hide_cursor_until_mouse_moves()
    }

    fn is_cursor_visible(&self) -> bool {
        self.inner.is_cursor_visible()
    }

    fn should_auto_hide_scrollbars(&self) -> bool {
        self.inner.should_auto_hide_scrollbars()
    }

    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.inner.read_from_clipboard()
    }

    fn write_to_clipboard(&self, item: ClipboardItem) {
        self.inner.write_to_clipboard(item)
    }

    fn read_from_clipboard_async(&self) -> Task<Result<Option<ClipboardItem>, ClipboardReadError>> {
        self.inner.read_from_clipboard_async()
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn read_from_primary(&self) -> Option<ClipboardItem> {
        self.inner.read_from_primary()
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn write_to_primary(&self, item: ClipboardItem) {
        self.inner.write_to_primary(item)
    }

    #[cfg(target_os = "macos")]
    fn read_from_find_pasteboard(&self) -> Option<ClipboardItem> {
        self.inner.read_from_find_pasteboard()
    }

    #[cfg(target_os = "macos")]
    fn write_to_find_pasteboard(&self, item: ClipboardItem) {
        self.inner.write_to_find_pasteboard(item)
    }

    fn write_credentials(&self, url: &str, username: &str, password: &[u8]) -> Task<Result<()>> {
        self.inner.write_credentials(url, username, password)
    }

    fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        self.inner.read_credentials(url)
    }

    fn delete_credentials(&self, url: &str) -> Task<Result<()>> {
        self.inner.delete_credentials(url)
    }

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        self.inner.keyboard_layout()
    }

    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        self.inner.keyboard_mapper()
    }

    fn on_keyboard_layout_change(&self, callback: Box<dyn FnMut()>) {
        self.inner.on_keyboard_layout_change(callback)
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::*;

    fn key(text: &str) -> Key {
        Key {
            text: text.to_owned(),
            font_size: px(14.),
            runs: vec![FontRun {
                len: text.len(),
                font_id: FontId(0),
            }],
        }
    }

    #[test]
    fn a_line_stays_while_used_and_goes_a_generation_after_its_last_use() {
        let mut lines = Lines::default();
        lines.insert(key("kept"), Arc::default());
        lines.insert(key("dropped"), Arc::default());
        for n in 0..GENERATION * 2 {
            lines.insert(key(&n.to_string()), Arc::default());
            if n % (GENERATION / 2) == 0 {
                assert!(lines.get(&key("kept")).is_some());
            }
        }
        assert!(lines.get(&key("kept")).is_some());
        assert!(lines.get(&key("dropped")).is_none());
        assert!(lines.current.len() + lines.previous.len() <= GENERATION * 2);
    }
}
