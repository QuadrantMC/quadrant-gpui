use std::borrow::Cow;

use gpui::{AssetSource, SharedString};
use gpui_component::IconNamed;

macro_rules! md_icons {
    ($($name:ident => $file:literal,)*) => {
        /// The Material Design icons the desktop app draws (react-icons/md),
        /// extracted into `assets/icons` by `scripts/extract-icons.mjs`.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum MdIcon {
            $($name,)*
        }

        impl IconNamed for MdIcon {
            fn path(self) -> SharedString {
                match self {
                    $(Self::$name => concat!("icons/md/", $file, ".svg"),)*
                }
                .into()
            }
        }

        const MD_ICON_FILES: &[(&str, &[u8])] = &[
            $((concat!("icons/md/", $file, ".svg"), include_bytes!(concat!("../assets/icons/", $file, ".svg"))),)*
        ];
    };
}

md_icons! {
    AccountCircle => "account-circle",
    Add => "add",
    Archive => "archive",
    ArrowBack => "arrow-back",
    Check => "check",
    Clear => "clear",
    Close => "close",
    Computer => "computer",
    Delete => "delete",
    Description => "description",
    Edit => "edit",
    Folder => "folder",
    Info => "info",
    Minimize => "minimize",
    Notifications => "notifications",
    Refresh => "refresh",
    Search => "search",
    Settings => "settings",
    Share => "share",
    Sync => "sync",
    // Search and mod pages
    ArrowForward => "arrow-forward",
    ChevronLeft => "chevron-left",
    Download => "download",
    ExpandMore => "expand-more",
    Extension => "extension",
    FileDownload => "file-download",
    FilterAlt => "filter-alt",
    FirstPage => "first-page",
    GridView => "grid-view",
    LastPage => "last-page",
    OpenInNew => "open-in-new",
    PermIdentity => "perm-identity",
    Person => "person",
    Update => "update",
    WbSunny => "wb-sunny",
    // Content, settings and account pages
    ContentCopy => "content-copy",
    OpenInBrowser => "open-in-browser",
    Cloud => "cloud",
    CloudDone => "cloud-done",
    PersonAdd => "person-add",
}

/// The app's own icons, falling back to gpui-component's for its widgets.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        match MD_ICON_FILES.iter().find(|(file, _)| *file == path) {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = gpui_kit_assets::Assets.list(path)?;
        paths.extend(
            MD_ICON_FILES
                .iter()
                .filter(|(file, _)| file.starts_with(path))
                .map(|(file, _)| SharedString::from(*file)),
        );
        Ok(paths)
    }
}
