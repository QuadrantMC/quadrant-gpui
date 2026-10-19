//! What the Search, mod install and current modpack pages share: the mod
//! card (the desktop app's `shared/Mod.tsx`) and the select option type.

use std::{
    hash::{DefaultHasher, Hash as _, Hasher as _},
    rc::Rc,
};

use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{Icon, h_flex, select::SelectItem, v_flex};
use quadrant_core::{
    mc_mod::{Mod, ModType},
    models::ModSource,
};

use crate::{
    assets::MdIcon,
    i18n::t,
    pages::{mod_grid::CardSlot, search::logic::compact_count},
    theme::{self, Tone},
};

/// A select option: what it reads as and the value it stands for.
#[derive(Clone)]
pub struct Choice<V> {
    pub title: SharedString,
    pub value: V,
}

impl<V> Choice<V> {
    pub fn new(title: impl Into<SharedString>, value: V) -> Self {
        Self {
            title: title.into(),
            value,
        }
    }
}

impl<V: Clone + PartialEq> SelectItem for Choice<V> {
    type Value = V;

    fn title(&self) -> SharedString {
        self.title.clone()
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

pub type OnCardPress<P> = Rc<dyn Fn(&mut P, CardPress, &Mod, &mut Window, &mut Context<P>)>;

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
    item: &Mod,
    status: CardStatus,
    slot: Option<CardSlot>,
    on_press: OnCardPress<P>,
    cx: &mut Context<P>,
) -> Stateful<Div> {
    let actions = card_actions(item, status.installed, status.finished);
    let press = |what: CardPress| {
        let on_press = on_press.clone();
        let item = item.clone();
        cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
            on_press(this, what, &item, window, cx)
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
                cx,
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
                cx,
            ),
            Primary::Update => theme::card_button(
                ("primary", index),
                Tone::Positive,
                label("update"),
                MdIcon::FileDownload,
                cx,
            ),
            Primary::Download => theme::card_button(
                ("primary", index),
                Tone::Positive,
                label("download"),
                MdIcon::FileDownload,
                cx,
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
            theme::card_button(
                ("select", index),
                Tone::Primary,
                t("select"),
                MdIcon::Check,
                cx,
            )
            .on_click(press(CardPress::Select))
            .into_any_element(),
        );
    }
    if actions.open_web {
        let url = item.url.clone();
        buttons.push(
            theme::card_icon_button(("open-web", index), MdIcon::OpenInNew, cx)
                .tooltip(t("openInTheWeb"))
                .on_click(move |_, _, cx| cx.open_url(&url))
                .into_any_element(),
        );
    }

    let source = source_label(&item.source);
    let open = press(CardPress::Open);
    let sized = |part: u64, element| match &slot {
        Some(slot) => slot
            .heights
            .apply(slot.part(card_key(item, status), part), true, element),
        None => element,
    };
    let card = v_flex()
        .id(("mod-card", index))
        .size_full()
        .gap_2p5()
        .p_4()
        .rounded_2xl()
        .bg(rgb(theme::PAGE))
        .hover(|this| this.bg(rgb(theme::CARD_HOVER)))
        .on_click(move |event, window, cx| {
            if event.click_count() == 2 {
                open(event, window, cx);
            }
        })
        .child(sized(
            HEADER,
            h_flex()
                .items_start()
                .gap_3()
                .child(theme::mod_icon(
                    &item.mod_icon_url,
                    &item.name,
                    px(3.5 * theme::FONT_SIZE),
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
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap_1p5()
                                .items_center()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child(Icon::new(MdIcon::Download).size(px(13.6)))
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
                        )
                        .child(
                            div()
                                .h_9()
                                .text_xs()
                                .line_height(px(18.9))
                                .text_color(rgb(theme::TEXT_MUTED))
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

/// `pill` after a separating dot, the two wrapping as one.
fn dotted(pill: impl IntoElement) -> impl IntoElement {
    h_flex()
        .gap_1p5()
        .items_center()
        .child(div().text_color(rgb(theme::TEXT_DISABLED)).child("·"))
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
            size: 0,
        });
        assert_eq!(
            card_actions(&item, false, false).primary,
            Some(Primary::Update)
        );
    }
}
