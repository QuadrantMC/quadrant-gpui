//! What a shared or synced pack deserves a warning for before it is
//! installed, after the desktop app's `modpackReview.ts` and
//! `ModpackReviewWarnings`: files Quadrant has no hash for, and mods that
//! download from a link instead of a provider's CDN.

use gpui::{
    ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, rems,
};
use gpui_component::v_flex;
use quadrant_core::{models::InstalledMod, modpacks::review_incoming_mods};

use crate::{i18n::t, theme};

/// A mod a warning names, and the host its file downloads from (empty when
/// the link doesn't parse).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reviewed {
    pub name: String,
    pub host: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Review {
    /// Mods that download from an arbitrary link.
    pub online: Vec<Reviewed>,
    /// Mods without a file hash, so their files can't be verified.
    pub unverified: Vec<Reviewed>,
}

impl Review {
    /// Reviews a whole pack, as nothing of it is installed yet. An Online mod
    /// without a hash is in both lists.
    pub fn of(mods: &[InstalledMod]) -> Self {
        let found = review_incoming_mods(&[], mods);
        let reviewed = |mods: Vec<InstalledMod>| mods.iter().map(reviewed).collect();
        Self {
            online: reviewed(found.online),
            unverified: reviewed(found.unverified),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.online.is_empty() && self.unverified.is_empty()
    }
}

fn reviewed(installed: &InstalledMod) -> Reviewed {
    Reviewed {
        name: installed.display_name().to_string(),
        host: url::Url::parse(&installed.download_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_default(),
    }
}

/// "Name (host)" for an Online mod, its name alone otherwise.
fn line(mod_: &Reviewed, with_host: bool) -> SharedString {
    if with_host && !mod_.host.is_empty() {
        format!("{} ({})", mod_.name, mod_.host).into()
    } else {
        mod_.name.clone().into()
    }
}

/// The warnings `review` earns, each with the mods it names in a list that
/// scrolls past ten lines. `id` tells the lists of one screen apart.
pub fn warnings(id: &str, review: &Review) -> impl IntoElement {
    let section = |which: &str, title: SharedString, mods: &[Reviewed], with_host: bool| {
        (!mods.is_empty()).then(|| {
            let p = theme::palette();
            v_flex()
                .w_full()
                .p_3()
                .rounded(theme::radius_card())
                .border_1()
                .border_color(p.warning_border)
                .bg(p.warning_fill)
                .text_color(p.warning_text)
                .text_sm()
                .child(div().font_weight(FontWeight::BOLD).child(title))
                .child(
                    v_flex()
                        .id(ElementId::Name(format!("{id}-{which}").into()))
                        .mt_1()
                        .pl_4()
                        .max_h(rems(10.))
                        .overflow_y_scroll()
                        .children(
                            mods.iter()
                                .map(|mod_| div().child(format!("• {}", line(mod_, with_host)))),
                        ),
                )
        })
    };
    v_flex()
        .w_full()
        .gap_3()
        .text_left()
        .children(section(
            "unverified",
            t("reviewUnverifiedWarning"),
            &review.unverified,
            false,
        ))
        .children(section(
            "online",
            t("reviewOnlineWarning"),
            &review.online,
            true,
        ))
}

#[cfg(test)]
mod tests {
    use quadrant_core::models::ModSource;

    use super::*;

    fn entry(id: &str, source: ModSource, url: &str, sha512: Option<&str>) -> InstalledMod {
        let mut installed = InstalledMod::minimal(id.into(), source, url.into());
        installed.sha512 = sha512.map(Into::into);
        installed
    }

    #[test]
    fn online_mods_name_their_host_and_unhashed_files_are_unverified() {
        let mut named = entry(
            "x",
            ModSource::Online,
            "https://files.example.com/a/linked.jar",
            None,
        );
        named.name = "Linked".into();
        let review = Review::of(&[
            entry(
                "sodium",
                ModSource::Modrinth,
                "https://cdn.modrinth.com/data/s/sodium.jar",
                Some("ab"),
            ),
            entry(
                "old",
                ModSource::CurseForge,
                "https://edge.forgecdn.net/files/1/2/old.jar",
                None,
            ),
            named,
        ]);
        let linked = Reviewed {
            name: "Linked".into(),
            host: "files.example.com".into(),
        };
        assert_eq!(review.online, std::slice::from_ref(&linked));
        assert_eq!(
            review.unverified,
            [
                Reviewed {
                    name: "old".into(),
                    host: "edge.forgecdn.net".into(),
                },
                linked.clone(),
            ]
        );
        assert_eq!(line(&linked, true), "Linked (files.example.com)");
        assert_eq!(line(&linked, false), "Linked");
    }

    #[test]
    fn a_pack_of_hashed_provider_files_needs_no_review() {
        let review = Review::of(&[entry(
            "sodium",
            ModSource::Modrinth,
            "https://cdn.modrinth.com/data/s/sodium.jar",
            Some("ab"),
        )]);
        assert!(review.is_empty());
        assert!(Review::of(&[]).is_empty());
    }
}
