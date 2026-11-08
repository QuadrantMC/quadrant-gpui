//! What the Search, mod install and current modpack pages share: the mod
//! card (the desktop app's `shared/Mod.tsx`) and the select option type.

use std::{
    cell::Cell,
    hash::{DefaultHasher, Hash as _, Hasher as _},
    rc::Rc,
};

use gpui::{
    AnyElement, Bounds, Context, Div, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, canvas, div, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{Icon, h_flex, select::SelectItem, v_flex};
use quadrant_core::{
    mc_mod::{Mod, ModType},
    models::ModSource,
};

use crate::{
    assets::MdIcon,
    i18n::{compact_count, t},
    pages::mod_grid::CardSlot,
    prefs::Prefs,
    theme::{self, Tone},
};

/// A card-sized box that centers a spinner or an error where content will go.
pub fn waiting(content: AnyElement) -> AnyElement {
    h_flex()
        .w_full()
        .p_4()
        .justify_center()
        .rounded(theme::radius_card())
        .bg(theme::palette().surface)
        .child(content)
        .into_any_element()
}

/// A select option: what it reads as and the value it stands for.
#[derive(Clone)]
pub struct Choice<V> {
    pub title: SharedString,
    /// A string key to show instead of `title`, looked up as it is shown,
    /// so a fixed choice follows a language change.
    key: Option<&'static str>,
    pub value: V,
}

impl<V> Choice<V> {
    pub fn new(title: impl Into<SharedString>, value: V) -> Self {
        Self {
            title: title.into(),
            key: None,
            value,
        }
    }

    /// A choice labelled by the string `key`.
    pub fn translated(key: &'static str, value: V) -> Self {
        Self {
            title: SharedString::default(),
            key: Some(key),
            value,
        }
    }
}

impl<V: Clone + PartialEq> SelectItem for Choice<V> {
    type Value = V;

    fn title(&self) -> SharedString {
        self.key.map_or_else(|| self.title.clone(), t)
    }

    fn value(&self) -> &V {
        &self.value
    }
}

/// The main action a card offers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Primary {
    /// Already in the target modpack; opens the install page to reinstall.
    Installed,
    /// Installs the newer file an update check found.
    Update,
    /// Installs straight away when the result is one-click installable,
    /// otherwise opens the install page.
    Download,
}

/// The buttons a card shows, after `Mod.tsx`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CardActions {
    pub delete: bool,
    pub primary: Option<Primary>,
    /// Picks this mod as what an unknown file is.
    pub select: bool,
    pub open_web: bool,
}

/// `installed` says the target modpack has the mod; `finished` that this
/// card's own install just completed, which retires its download button.
pub fn card_actions(item: &Mod, installed: bool, finished: bool) -> CardActions {
    let can_download =
        item.downloadable && !matches!(item.mod_type, ModType::Modpack | ModType::DataPack);
    let primary = if installed && can_download {
        Some(Primary::Installed)
    } else if can_download && !finished {
        Some(
            if item.new_version.is_some() && item.show_previous_version {
                Primary::Update
            } else {
                Primary::Download
            },
        )
    } else {
        None
    };
    CardActions {
        delete: item.deleteable,
        primary,
        select: item.selectable,
        open_web: !item.url.trim().is_empty(),
    }
}

/// What a press on a card asks its page to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardPress {
    Delete,
    Primary(Primary),
    Select,
    /// A double click on the card, which opens the install page.
    Open,
}

/// How a card is drawn right now.
#[derive(Clone, Copy, Default)]
pub struct CardStatus {
    pub installed: bool,
    pub finished: bool,
    /// This card's running install's progress. Other cards stay usable
    /// while it runs; the page ignores a second press on this one.
    pub progress: Option<i32>,
}

pub fn source_label(source: &ModSource) -> &'static str {
    match source {
        ModSource::CurseForge => "CurseForge",
        ModSource::Modrinth => "Modrinth",
        ModSource::Online => "",
    }
}

/// What a card's press runs: the press, the card's mod and where the card
/// was on screen, which an install page it opens grows out of.
pub type OnCardPress<P> =
    Rc<dyn Fn(&mut P, CardPress, &Mod, Bounds<Pixels>, &mut Window, &mut Context<P>)>;

/// What decides a card's height besides its width: the rows that can wrap
/// (downloads, provider and "Installed"; the buttons and their labels).
/// The name keeps to one line and the description to two.
pub fn card_key(item: &Mod, status: CardStatus) -> u64 {
    let mut hasher = DefaultHasher::new();
    item.id.hash(&mut hasher);
    item.download_count.hash(&mut hasher);
    source_label(&item.source).hash(&mut hasher);
    card_actions(item, status.installed, status.finished).hash(&mut hasher);
    (status.installed, status.progress).hash(&mut hasher);
    hasher.finish()
}

/// A mod card: icon, name, downloads, provider, description, and its
/// actions on the right of the bottom row.
/// In a grid that keeps its row heights (`slot`), a card and its header
/// take their heights from earlier frames while nothing that decides them
/// changed, so each lays out once.
pub fn mod_card<P: 'static>(
    index: usize,
    item: &Rc<Mod>,
    status: CardStatus,
    slot: Option<CardSlot>,
    on_press: OnCardPress<P>,
    cx: &mut Context<P>,
) -> Stateful<Div> {
    let actions = card_actions(item, status.installed, status.finished);
    // Where the card was drawn last, for the press to hand on.
    let origin = Rc::new(Cell::new(Bounds::default()));
    let press = |what: CardPress| {
        let on_press = on_press.clone();
        let item = item.clone();
        let origin = origin.clone();
        cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
            on_press(this, what, &item, origin.get(), window, cx)
        })
    };
    let label = |key: &str| -> SharedString {
        match status.progress {
            Some(progress) => format!("{progress}%").into(),
            None => t(key),
        }
    };
    let mut buttons: Vec<AnyElement> = Vec::new();
    if actions.delete {
        buttons.push(
            theme::card_button(
                ("delete", index),
                Tone::Destructive,
                t("delete"),
                MdIcon::Delete,
            )
            .on_click(press(CardPress::Delete))
            .into_any_element(),
        );
    }
    if let Some(primary) = actions.primary {
        let button = match primary {
            Primary::Installed => theme::card_button(
                ("primary", index),
                Tone::Neutral,
                t("installed"),
                MdIcon::Check,
            ),
            Primary::Update => theme::card_button(
                ("primary", index),
                Tone::Positive,
                label("update"),
                MdIcon::FileDownload,
            ),
            Primary::Download => theme::card_button(
                ("primary", index),
                Tone::Positive,
                label("download"),
                MdIcon::FileDownload,
            ),
        };
        buttons.push(
            button
                .on_click(press(CardPress::Primary(primary)))
                .into_any_element(),
        );
    }
    if actions.select {
        buttons.push(
            theme::card_button(("select", index), Tone::Primary, t("select"), MdIcon::Check)
                .on_click(press(CardPress::Select))
                .into_any_element(),
        );
    }
    if actions.open_web {
        let url = item.url.clone();
        buttons.push(
            theme::card_icon_button(("open-web", index), MdIcon::OpenInNew, cx)
                .tooltip(t("openInTheWeb"))
                .on_click(move |_, _, cx| crate::web::open_external(&url, cx))
                .into_any_element(),
        );
    }

    let source = source_label(&item.source);
    let open = press(CardPress::Open);
    let key = slot.as_ref().map(|_| card_key(item, status));
    let sized = |part: u64, element| match (&slot, key) {
        (Some(slot), Some(key)) => slot.heights.apply(slot.part(key, part), true, element),
        _ => element,
    };
    let card = v_flex()
        .id(("mod-card", index))
        .size_full()
        .gap_2p5()
        .p_4()
        .rounded_2xl()
        .bg(theme::palette().page)
        .hover(|this| this.bg(theme::palette().card_hover))
        .on_click(move |event, window, cx| {
            if event.click_count() == 2 {
                open(event, window, cx);
            }
        })
        .child(
            canvas(move |bounds, _, _| origin.set(bounds), |_, _, _, _| {})
                .absolute()
                .size_full(),
        )
        .child(sized(
            HEADER,
            h_flex()
                .items_start()
                .gap_3()
                .child(theme::mod_icon(
                    &item.mod_icon_url,
                    &item.name,
                    rems(3.5),
                    Prefs::global(cx).clip_icons,
                    cx,
                ))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BLACK)
                                .truncate()
                                .child(item.name.clone()),
                        )
                        // Wraps rather than spilling out of a narrow card.
                        .child(sized(
                            META,
                            h_flex()
                                .flex_wrap()
                                .gap_1p5()
                                .items_center()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::palette().text_muted)
                                .child(Icon::new(MdIcon::Download).size(rems(0.8125)))
                                .child(compact_count(item.download_count))
                                .when(!source.is_empty(), |this| {
                                    this.child(dotted(theme::source_pill(
                                        source,
                                        item.source == ModSource::CurseForge,
                                    )))
                                })
                                .when(status.installed, |this| {
                                    this.child(dotted(theme::installed_pill(t("installed"))))
                                }),
                        ))
                        .child(
                            div()
                                .h_9()
                                .text_xs()
                                .line_height(rems(1.125))
                                .text_color(theme::palette().text_muted)
                                .line_clamp(2)
                                .text_ellipsis()
                                .overflow_hidden()
                                .child(item.description.trim().to_string()),
                        ),
                ),
        ))
        .child(
            h_flex()
                .mt_auto()
                .flex_wrap()
                .gap_2()
                .items_center()
                .justify_end()
                .children(buttons),
        );
    match slot {
        Some(slot) => slot.heights.apply(slot.row, slot.first, card),
        None => card,
    }
}

/// The card's top part (icon, name, downloads, description), as a part of
/// [`CardSlot::part`].
const HEADER: u64 = 1;

/// The downloads and provider row under the name, which wraps in a narrow
/// card and is laid out from its content several times over otherwise.
const META: u64 = 2;

/// `pill` after a separating dot, the two wrapping as one.
fn dotted(pill: impl IntoElement) -> impl IntoElement {
    h_flex()
        .gap_1p5()
        .items_center()
        .child(div().text_color(theme::palette().text_disabled).child("·"))
        .child(pill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::search::logic::tests::result;
    use quadrant_core::mc_mod::UniversalModFile;

    #[test]
    fn a_search_result_downloads_until_it_is_installed() {
        let item = result("Sodium", ModSource::Modrinth, 0, "");
        let actions = card_actions(&item, false, false);
        assert_eq!(actions.primary, Some(Primary::Download));
        assert!(actions.open_web);
        assert!(!actions.delete);
        assert_eq!(
            card_actions(&item, true, false).primary,
            Some(Primary::Installed)
        );
        assert_eq!(card_actions(&item, false, true).primary, None);
    }

    #[test]
    fn modpacks_and_installed_mods_are_not_downloadable() {
        let mut item = result("Pack", ModSource::Modrinth, 0, "");
        item.mod_type = ModType::Modpack;
        assert_eq!(card_actions(&item, false, false).primary, None);

        let mut installed = result("Lithium", ModSource::Modrinth, 0, "");
        installed.downloadable = false;
        installed.deleteable = true;
        installed.url = String::new();
        let actions = card_actions(&installed, false, false);
        assert_eq!(actions.primary, None);
        assert!(actions.delete);
        assert!(!actions.open_web);
    }

    #[test]
    fn an_update_check_result_offers_the_update() {
        let mut item = result("Iris", ModSource::Modrinth, 0, "");
        item.show_previous_version = true;
        item.new_version = Some(UniversalModFile {
            id: None,
            file_name: "iris-2.jar".into(),
            download_url: "https://cdn.modrinth.com/iris-2.jar".into(),
            sha1: String::new(),
            sha512: None,
            size: 0,
        });
        assert_eq!(
            card_actions(&item, false, false).primary,
            Some(Primary::Update)
        );
    }
}
