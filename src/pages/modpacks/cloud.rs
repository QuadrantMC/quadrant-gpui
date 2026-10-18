//! Quadrant Sync and Quadrant Share on the Apply page, after the desktop app's
//! `mergeModpacks`, `syncDates`, `SyncedModpack`, `CloudMembersPanel` and the
//! share code banner. All of it needs a signed-in Quadrant ID.

use chrono::{Local, TimeZone as _};
use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{
    Disableable as _, WindowExt as _,
    button::Button,
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use quadrant_core::{
    account::{id::AccountInfo, quadrant_sync::SyncedModpack},
    models::{InstalledMod, InstalledModpack, LocalModpack},
};

use super::{ModpacksPage, Op, SyncTarget, View, pill};
use crate::{
    assets::MdIcon,
    i18n::{t, t_with},
    theme::{self, Tone},
};

/// The address a Quadrant Share code is opened at.
pub const SHARE_URL: &str = "https://usequadrant.dev/modpack/";

/// One row of the list: a local modpack, its Quadrant Sync record, or both.
#[derive(Clone, Copy, Debug)]
pub struct Row<'a> {
    pub local: Option<&'a LocalModpack>,
    pub synced: Option<&'a SyncedModpack>,
}

fn matches_query(query: &str, name: &str, version: &str, loader: &str) -> bool {
    query.is_empty()
        || [name, version, loader]
            .iter()
            .any(|field| field.to_lowercase().contains(query))
}

fn pairs_with(local: &LocalModpack, synced: &SyncedModpack) -> bool {
    match &local.modpack_id {
        Some(id) => *id == synced.modpack_id,
        None => local.name == synced.name,
    }
}

/// Pairs every local modpack with its cloud record (by modpack id, or by name
/// for packs that predate ids), then adds the cloud packs that aren't
/// installed, newest sync first. Pairing comes before the search, so a search
/// for a cloud name still finds its renamed local copy.
pub fn merge_modpacks<'a>(
    local: &'a [LocalModpack],
    synced: &'a [SyncedModpack],
    query: &str,
) -> Vec<Row<'a>> {
    let query = query.trim().to_lowercase();
    let local_ids: Vec<&str> = local
        .iter()
        .filter_map(|modpack| modpack.modpack_id.as_deref())
        .collect();
    let mut claimed: Vec<&str> = Vec::new();
    let mut rows: Vec<Row> = local
        .iter()
        .map(|modpack| {
            let counterpart = synced.iter().find(|candidate| {
                !claimed.contains(&candidate.modpack_id.as_str())
                    && (modpack.modpack_id.is_some()
                        || !local_ids.contains(&candidate.modpack_id.as_str()))
                    && pairs_with(modpack, candidate)
            });
            if let Some(counterpart) = counterpart {
                claimed.push(&counterpart.modpack_id);
            }
            Row {
                local: Some(modpack),
                synced: counterpart,
            }
        })
        .collect();
    let mut cloud_only: Vec<&SyncedModpack> = synced
        .iter()
        .filter(|modpack| !claimed.contains(&modpack.modpack_id.as_str()))
        .collect();
    cloud_only.sort_by_key(|modpack| std::cmp::Reverse(modpack.last_synced));
    rows.extend(cloud_only.into_iter().map(|synced| Row {
        local: None,
        synced: Some(synced),
    }));
    rows.retain(|row| {
        row.local.is_some_and(|local| {
            matches_query(
                &query,
                &local.name,
                &local.version,
                &local.mod_loader.to_string(),
            )
        }) || row.synced.is_some_and(|synced| {
            matches_query(
                &query,
                &synced.name,
                &synced.minecraft_version,
                &synced.mod_loader.to_string(),
            )
        })
    });
    rows
}

/// The code in a bare seven-digit code or a `usequadrant.dev/modpack/` link.
pub fn parse_share_code(input: &str) -> Option<String> {
    let input = input.trim();
    let is_code = |code: &str| code.len() == 7 && code.chars().all(|c| c.is_ascii_digit());
    if is_code(input) {
        return Some(input.to_string());
    }
    let url = url::Url::parse(input).ok()?;
    if !matches!(
        url.host_str(),
        Some("usequadrant.dev" | "www.usequadrant.dev")
    ) {
        return None;
    }
    let code = url.path().strip_prefix("/modpack/")?.trim_end_matches('/');
    is_code(code).then(|| code.to_string())
}

/// Timestamps up to this are seconds (older records); later ones milliseconds.
const SECONDS_THRESHOLD: i64 = 170_406_720_000;

/// "Saturday, 09/19/26, 14:11", for a sync time in either unit.
pub fn format_sync_date(timestamp: i64) -> String {
    let millis = if timestamp <= SECONDS_THRESHOLD {
        timestamp.saturating_mul(1000)
    } else {
        timestamp
    };
    Local
        .timestamp_millis_opt(millis)
        .single()
        .map(|date| date.format("%A, %D, %H:%M").to_string())
        .unwrap_or_default()
}

/// The manifest a Quadrant Sync record describes, named `name` on disk.
pub fn installable(synced: &SyncedModpack, name: &str) -> InstalledModpack {
    InstalledModpack {
        mod_config_version: "2".into(),
        quadrant_version: String::new(),
        name: name.to_string(),
        version: synced.minecraft_version.clone(),
        mod_loader: synced.mod_loader,
        mods: serde_json::from_str::<Vec<InstalledMod>>(&synced.mods).unwrap_or_default(),
    }
}

/// Whether `account` may kick `username`: admins may kick anyone but
/// themselves.
pub fn can_kick(synced: &SyncedModpack, account: &AccountInfo, username: &str) -> bool {
    username != account.login
        && synced
            .owners
            .iter()
            .any(|owner| owner.username == account.login && owner.admin)
}

impl ModpacksPage {
    /// The download button of a cloud pack, showing the install's progress
    /// while it runs.
    pub(super) fn download_button(
        &self,
        id: impl Into<gpui::ElementId>,
        label: SharedString,
        modpack: InstalledModpack,
        sync: Option<SyncTarget>,
        cx: &mut Context<Self>,
    ) -> Button {
        let installing = match &self.running {
            Some(Op::Install {
                modpack: running, ..
            }) => running.name == modpack.name,
            _ => false,
        };
        let (tone, label) = match (installing, self.install_progress) {
            (true, Some(progress)) => (Tone::Neutral, format!("{:.2}%", progress * 100.).into()),
            (true, None) => (Tone::Neutral, "0.00%".into()),
            (false, _) => (Tone::Positive, label),
        };
        theme::pill_button(id, tone, label, Some(MdIcon::Download), cx)
            .disabled(self.running.is_some())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run(
                    Op::Install {
                        modpack: Box::new(modpack.clone()),
                        sync: sync.clone(),
                    },
                    cx,
                )
            }))
    }

    /// A Quadrant Sync pack that isn't installed here.
    pub(super) fn render_cloud_card(
        &self,
        index: usize,
        synced: &SyncedModpack,
        account: &AccountInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let modpack = installable(synced, &synced.name);
        let target = SyncTarget::of(synced);
        let line = format!(
            "{} | {} | {} | {}",
            synced.minecraft_version,
            synced.mod_loader,
            t_with("modCount", &[("amount", &modpack.mods.len().to_string())]),
            t_with(
                "cloudSyncDate",
                &[("date", &format_sync_date(synced.last_synced))]
            ),
        );
        let share = {
            let modpack = modpack.clone();
            theme::pill_button(
                ("cloud-share", index),
                Tone::Primary,
                t("share"),
                Some(MdIcon::Share),
                cx,
            )
            .disabled(self.running.is_some())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run(Op::ShareRaw(Box::new(modpack.clone())), cx)
            }))
        };
        let details = {
            let modpack = modpack.clone();
            let target = target.clone();
            theme::pill_button(
                ("cloud-details", index),
                Tone::Neutral,
                t("details"),
                Some(MdIcon::Info),
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.show(
                    View::Shared {
                        modpack: Box::new(modpack.clone()),
                        sync: Some(target.clone()),
                    },
                    cx,
                )
            }))
        };
        v_flex()
            .mx_5()
            .p_4()
            .rounded(theme::radius_card())
            .bg(rgb(theme::PAGE))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child(synced.name.clone()),
                    )
                    .child(theme::cloud_badge(t("badgeCloud")))
                    .child(theme::synced_badge(t("badgeSynced"))),
            )
            .child(div().text_color(rgb(theme::TEXT_MUTED)).child(line))
            .child(
                h_flex()
                    .my_2()
                    .flex_wrap()
                    .gap_4()
                    .child(self.download_button(
                        ("cloud-download", index),
                        t("download"),
                        modpack,
                        Some(target),
                        cx,
                    ))
                    .child(share)
                    .child(details),
            )
            .child(self.render_sync_panel(synced, None, account, cx))
    }

    /// The folding Quadrant Sync panel of a card: members, kicking, inviting,
    /// deleting the cloud copy, and for an installed pack a force pull.
    pub(super) fn render_sync_panel(
        &self,
        synced: &SyncedModpack,
        local_name: Option<&str>,
        account: &AccountInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = synced.modpack_id.clone();
        let open = self.open_panels.contains(&id);
        let busy = self.running.is_some();
        let element_id = |what: &str| gpui::ElementId::Name(format!("{what}:{id}").into());
        let header = h_flex()
            .id(element_id("sync-panel"))
            .h_12()
            .p_2()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener({
                let id = id.clone();
                move |this, _, _, cx| {
                    match this.open_panels.iter().position(|each| *each == id) {
                        Some(index) => {
                            this.open_panels.remove(index);
                        }
                        None => this.open_panels.push(id.clone()),
                    }
                    cx.notify();
                }
            }))
            .child(
                div()
                    .ml_2()
                    .flex_1()
                    .font_weight(FontWeight::BOLD)
                    .child(t("cloudMembers")),
            )
            .child(
                gpui_component::Icon::new(MdIcon::ExpandMore)
                    .size_6()
                    .when(open, |icon| {
                        icon.rotate(gpui::radians(std::f32::consts::PI))
                    }),
            );
        let body = open.then(|| {
            let members = synced.owners.iter().map(|owner| {
                let label: SharedString = if owner.admin {
                    t_with("owner", &[("username", &owner.username)])
                } else {
                    owner.username.clone().into()
                };
                let kick = can_kick(synced, account, &owner.username).then(|| {
                    let op = Op::Kick {
                        modpack_id: id.clone(),
                        username: owner.username.clone(),
                    };
                    theme::kick_button(
                        element_id(&format!("kick-{}", owner.username)),
                        t("kick"),
                        cx,
                    )
                    .mr_4()
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.run(op.clone(), cx)))
                });
                h_flex()
                    .my_3()
                    .h(gpui::rems(3.5))
                    .p_2()
                    .items_center()
                    .rounded_full()
                    .bg(rgb(theme::SURFACE))
                    .font_weight(FontWeight::BOLD)
                    .child(div().flex_1().px_4().child(label))
                    .children(kick)
                    .into_any_element()
            });
            let members: Vec<_> = members.collect();
            let force_pull = local_name.map(|name| {
                self.download_button(
                    element_id("force-pull"),
                    t("forcePull"),
                    installable(synced, name),
                    Some(SyncTarget::of(synced)),
                    cx,
                )
                .flex_1()
            });
            let delete = {
                let op = Op::DeleteSynced(id.clone());
                theme::pill_button(
                    element_id("delete-synced"),
                    Tone::Destructive,
                    t("delete"),
                    Some(MdIcon::Delete),
                    cx,
                )
                .flex_1()
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| this.run(op.clone(), cx)))
            };
            let invite = {
                let id = id.clone();
                theme::pill_button(
                    element_id("invite"),
                    Tone::Positive,
                    t("invite"),
                    Some(MdIcon::PersonAdd),
                    cx,
                )
                .flex_1()
                .disabled(busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_invite_dialog(id.clone(), window, cx)
                }))
            };
            v_flex().p_4().children(members).child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap_2()
                    .children(force_pull)
                    .child(delete)
                    .child(invite),
            )
        });
        v_flex()
            .w_full()
            .rounded(theme::radius_card())
            .bg(rgb(theme::CONTROL))
            .child(header)
            .children(body)
    }

    /// Asks who to invite to a Quadrant Sync pack, and whether as an admin.
    pub fn open_invite_dialog(
        &mut self,
        modpack_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let username = cx.new(|cx| InputState::new(window, cx).placeholder(t("username")));
        let admin: Entity<bool> = cx.new(|_| false);
        let page = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let invite = {
                let (username, admin, page, modpack_id) = (
                    username.clone(),
                    admin.clone(),
                    page.clone(),
                    modpack_id.clone(),
                );
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let op = Op::Invite {
                        modpack_id: modpack_id.clone(),
                        username: username.read(cx).value().trim().to_string(),
                        admin: *admin.read(cx),
                    };
                    page.update(cx, |page, cx| page.run(op, cx));
                    window.close_dialog(cx);
                }
            };
            let toggle_admin = {
                let admin = admin.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    admin.update(cx, |admin, _| *admin = !*admin);
                    window.refresh();
                }
            };
            theme::dialog_surface(dialog)
                .w(px(560.))
                .title(theme::dialog_title(t("invite")))
                .child(
                    v_flex()
                        .gap_4()
                        .child(pill(Input::new(&username).appearance(false)))
                        .child(
                            h_flex()
                                .items_center()
                                .child(
                                    theme::setting_switch("invite-admin", *admin.read(cx))
                                        .on_click(toggle_admin),
                                )
                                .child(
                                    div()
                                        .mx_2()
                                        .font_weight(FontWeight::BLACK)
                                        .child(t("admin")),
                                ),
                        ),
                )
                .footer(
                    DialogFooter::new()
                        .justify_start()
                        .child(
                            theme::pill_button(
                                "send-invite",
                                Tone::Positive,
                                t("invite"),
                                None,
                                cx,
                            )
                            .on_click(invite),
                        )
                        .child(
                            theme::cancel_button("cancel", t("cancel"), cx)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
        });
    }

    /// The desktop app's question when the cloud copy is newer than this one.
    pub(super) fn confirm_sync_anyway(
        &mut self,
        modpack: LocalModpack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let sync = {
                let (page, modpack) = (page.clone(), modpack.clone());
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let op = Op::Sync {
                        modpack: Box::new(modpack.clone()),
                        overwrite: true,
                    };
                    page.update(cx, |page, cx| page.run(op, cx));
                    window.close_dialog(cx);
                }
            };
            theme::dialog_surface(dialog)
                .w(px(560.))
                .title(theme::dialog_title(t("cloudSyncNewerQuestion")))
                .child(t_with(
                    "cloudSyncNewerQuestionText",
                    &[("name", &modpack.name)],
                ))
                .footer(
                    DialogFooter::new()
                        .justify_start()
                        .child(
                            theme::pill_button(
                                "sync-anyway",
                                Tone::Positive,
                                t("syncAnyway"),
                                None,
                                cx,
                            )
                            .on_click(sync),
                        )
                        .child(
                            theme::cancel_button("cancel", t("cancel"), cx)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
        });
    }

    /// The banner a Quadrant Share code in the search box brings up.
    pub(super) fn render_share_banner(
        &self,
        code: String,
        signed_in: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let resolving = matches!(self.running, Some(Op::ResolveShare(_)));
        let label = if resolving {
            t("loadingMore")
        } else {
            t("download")
        };
        let tone = if resolving {
            Tone::Control
        } else {
            Tone::Positive
        };
        let button = theme::pill_button("share-download", tone, label, None, cx)
            .w_full()
            .mt_4()
            .disabled(self.running.is_some() || !signed_in)
            .when(!signed_in, |this| this.tooltip(t("errorSignedOut")))
            .on_click(cx.listener({
                let code = code.clone();
                move |this, _, _, cx| this.run(Op::ResolveShare(code.clone()), cx)
            }));
        v_flex()
            .mx_8()
            .mb_2()
            .p_4()
            .items_center()
            .rounded(theme::radius_card())
            .bg(rgb(theme::SURFACE))
            .font_weight(FontWeight::BOLD)
            .child(t_with("shareCodeDetected", &[("code", &code)]))
            .child(button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_core::{account::quadrant_sync::ModpackOwner, models::ModLoader};

    fn local(name: &str, id: Option<&str>) -> LocalModpack {
        LocalModpack {
            name: name.into(),
            version: "1.21.1".into(),
            mod_loader: ModLoader::Fabric,
            mods: Vec::new(),
            unknown_mods: false,
            is_applied: false,
            last_synced: 0,
            modpack_id: id.map(Into::into),
        }
    }

    fn cloud(name: &str, id: &str, last_synced: i64) -> SyncedModpack {
        SyncedModpack {
            name: name.into(),
            minecraft_version: "1.20.1".into(),
            mod_loader: ModLoader::Forge,
            mods:
                r#"[{"id":"jei","source":"ModSource.modRinth","downloadUrl":"https://x/jei.jar"}]"#
                    .into(),
            owners: vec![
                ModpackOwner {
                    username: "alex".into(),
                    admin: true,
                },
                ModpackOwner {
                    username: "sam".into(),
                    admin: false,
                },
            ],
            last_synced,
            modpack_id: id.into(),
        }
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match (row.local, row.synced) {
                (Some(local), Some(synced)) => format!("{}+{}", local.name, synced.modpack_id),
                (Some(local), None) => local.name.clone(),
                (None, Some(synced)) => format!("cloud:{}", synced.modpack_id),
                (None, None) => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn pairs_by_id_then_by_name_and_lists_the_rest_newest_first() {
        let locals = [
            local("Renamed", Some("one")),
            local("Two", None),
            local("Lonely", None),
        ];
        let clouds = [
            cloud("One", "one", 10),
            cloud("Two", "two", 20),
            cloud("Old", "old", 5),
            cloud("New", "new", 50),
        ];
        let rows = merge_modpacks(&locals, &clouds, "");
        assert_eq!(
            names(&rows),
            ["Renamed+one", "Two+two", "Lonely", "cloud:new", "cloud:old"]
        );
    }

    #[test]
    fn a_search_for_the_cloud_name_finds_the_renamed_copy() {
        let locals = [local("Renamed", Some("one"))];
        let clouds = [cloud("Original", "one", 10)];
        assert_eq!(
            names(&merge_modpacks(&locals, &clouds, "orig")),
            ["Renamed+one"]
        );
        assert!(merge_modpacks(&locals, &clouds, "nothing").is_empty());
        assert_eq!(names(&merge_modpacks(&locals, &[], "FABRIC")), ["Renamed"]);
    }

    #[test]
    fn share_codes_come_bare_or_as_links() {
        assert_eq!(parse_share_code(" 1234567 "), Some("1234567".into()));
        assert_eq!(
            parse_share_code("https://usequadrant.dev/modpack/7654321/"),
            Some("7654321".into())
        );
        assert_eq!(
            parse_share_code("https://example.com/modpack/7654321"),
            None
        );
        assert_eq!(parse_share_code("123456"), None);
        assert_eq!(parse_share_code("Alpha Pack"), None);
    }

    #[test]
    fn sync_dates_read_seconds_and_milliseconds_alike() {
        let seconds = format_sync_date(1_700_000_000);
        assert_eq!(seconds, format_sync_date(1_700_000_000_000));
        assert!(seconds.contains("/23, "), "{seconds}");
    }

    #[test]
    fn a_cloud_pack_installs_under_its_local_name() {
        let pack = installable(&cloud("Cloud", "one", 1), "Local");
        assert_eq!(pack.name, "Local");
        assert_eq!(pack.mods.len(), 1);
        assert_eq!(pack.mod_loader, ModLoader::Forge);
    }

    #[test]
    fn only_admins_kick_and_never_themselves() {
        let pack = cloud("Cloud", "one", 1);
        let account = |login: &str| AccountInfo {
            id: String::new(),
            name: String::new(),
            email: String::new(),
            quadrant_sync_limit: 1,
            quadrant_share_limit: 1,
            login: login.into(),
            notifications: Vec::new(),
        };
        assert!(can_kick(&pack, &account("alex"), "sam"));
        assert!(!can_kick(&pack, &account("alex"), "alex"));
        assert!(!can_kick(&pack, &account("sam"), "alex"));
    }
}
