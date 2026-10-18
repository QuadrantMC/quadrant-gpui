//! The Installed content page: resource packs and shader packs in the
//! Minecraft folder and in each Prism Launcher instance, after
//! `InstalledContentPage` in the desktop app.

use std::{collections::HashSet, time::Duration};

use chrono::{Local, TimeZone as _};
use gpui::{
    ClickEvent, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Task,
    Window, div, prelude::FluentBuilder as _, rgb,
};
use gpui_component::{
    Disableable as _, Icon, Sizable as _,
    button::Button,
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    spinner::Spinner,
    v_flex,
};
use quadrant_core::{
    content::{ContentFile, ContentLocation, ContentLocationKind},
    mc_mod::ModType,
};
use quadrant_host::QuadrantHost;

use crate::{
    assets::MdIcon,
    backend::Backend,
    i18n::{describe, t, t_count, t_with},
    loadable::Loadable,
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

/// How long an armed delete waits for its confirming click.
const ARM_TIMEOUT: Duration = Duration::from_secs(4);

/// One section of one location: everything an action needs to address it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionRef {
    pub location: String,
    pub mod_type: ModType,
}

/// One ticked file, remembered with the section it was ticked in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedFile {
    pub section: SectionRef,
    pub file_name: String,
}

/// Files of one section, as one `copy_content` or `delete_content` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionFiles {
    pub section: SectionRef,
    pub file_names: Vec<String>,
}

/// A location a copy could go to, and the work it would do there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyTarget {
    pub location: String,
    pub title: SharedString,
    /// One copy per source section, never from the destination itself.
    pub groups: Vec<SectionFiles>,
}

impl CopyTarget {
    /// How many files would be copied, across every group.
    pub fn count(&self) -> usize {
        count_files(&self.groups)
    }
}

/// The control an action was started from, so only it shows as busy or armed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    File(SectionRef, String),
    CopyAll(SectionRef),
    /// The page-wide selection bar.
    Selection,
}

impl Control {
    fn id(&self, what: &str) -> ElementId {
        let key = match self {
            Control::File(section, name) => {
                format!("{}|{:?}|{name}", section.location, section.mod_type)
            }
            Control::CopyAll(section) => {
                format!("{}|{:?}|#all", section.location, section.mod_type)
            }
            Control::Selection => "#selection".to_string(),
        };
        ElementId::Name(format!("{what}:{key}").into())
    }
}

/// What the user calls a location: the Minecraft folder, or the instance name.
pub fn location_title(location: &ContentLocation) -> SharedString {
    match location.kind {
        ContentLocationKind::Minecraft => t("installedContentMinecraft"),
        ContentLocationKind::Prism => location.name.clone().into(),
    }
}

/// What the user calls one section.
pub fn section_title(mod_type: ModType) -> SharedString {
    match mod_type {
        ModType::ResourcePack => t("contentResourcePacks"),
        ModType::ShaderPack => t("contentShaders"),
        other => format!("{other:?}").into(),
    }
}

fn names_in<'a>(locations: &'a [ContentLocation], section: &SectionRef) -> HashSet<&'a str> {
    locations
        .iter()
        .filter(|location| location.id == section.location)
        .flat_map(|location| &location.sections)
        .filter(|each| each.mod_type == section.mod_type)
        .flat_map(|each| each.files.iter().map(|file| file.file_name.as_str()))
        .collect()
}

/// The selection as one request per section, in first-ticked order.
pub fn group_by_section(selection: &[SelectedFile]) -> Vec<SectionFiles> {
    let mut groups: Vec<SectionFiles> = Vec::new();
    for file in selection {
        match groups
            .iter_mut()
            .find(|group| group.section == file.section)
        {
            Some(group) => group.file_names.push(file.file_name.clone()),
            None => groups.push(SectionFiles {
                section: file.section.clone(),
                file_names: vec![file.file_name.clone()],
            }),
        }
    }
    groups
}

pub fn count_files(groups: &[SectionFiles]) -> usize {
    groups.iter().map(|group| group.file_names.len()).sum()
}

/// Every location a copy of `files` could go to, minus the files that come
/// from there and the ones it already holds. A destination every file came
/// from is not offered.
pub fn copy_targets(locations: &[ContentLocation], files: &[SelectedFile]) -> Vec<CopyTarget> {
    let sources = group_by_section(files);
    locations
        .iter()
        .filter(|location| {
            sources
                .iter()
                .any(|source| source.section.location != location.id)
        })
        .map(|location| {
            let groups = sources
                .iter()
                .filter(|source| source.section.location != location.id)
                .filter_map(|source| {
                    let present = names_in(
                        locations,
                        &SectionRef {
                            location: location.id.clone(),
                            mod_type: source.section.mod_type,
                        },
                    );
                    let file_names: Vec<String> = source
                        .file_names
                        .iter()
                        .filter(|name| !present.contains(name.as_str()))
                        .cloned()
                        .collect();
                    (!file_names.is_empty()).then(|| SectionFiles {
                        section: source.section.clone(),
                        file_names,
                    })
                })
                .collect();
            CopyTarget {
                location: location.id.clone(),
                title: location_title(location),
                groups,
            }
        })
        .collect()
}

/// Ticks or unticks one file, leaving every tick made elsewhere alone.
pub fn toggle_selection(selection: &mut Vec<SelectedFile>, file: SelectedFile) {
    match selection.iter().position(|each| *each == file) {
        Some(index) => {
            selection.remove(index);
        }
        None => selection.push(file),
    }
}

/// Adds every one of a section's files, keeping the ticks made elsewhere.
pub fn select_all_in(selection: &mut Vec<SelectedFile>, section: &SectionRef, names: &[String]) {
    for name in names {
        let file = SelectedFile {
            section: section.clone(),
            file_name: name.clone(),
        };
        if !selection.contains(&file) {
            selection.push(file);
        }
    }
}

/// Forgets every tick a reload no longer reports.
pub fn prune_selection(selection: &mut Vec<SelectedFile>, locations: &[ContentLocation]) {
    selection.retain(|file| names_in(locations, &file.section).contains(file.file_name.as_str()));
}

/// "371.7kB": the size in the largest binary unit it reaches, as the desktop
/// app's narrow `Intl.NumberFormat` units print it.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [("B", 1), ("kB", 1 << 10), ("MB", 1 << 20), ("GB", 1 << 30)];
    let (unit, size) = UNITS
        .iter()
        .rev()
        .find(|(_, size)| bytes >= *size)
        .copied()
        .unwrap_or(UNITS[0]);
    let value = (bytes as f64 / size as f64 * 10.).round() / 10.;
    let whole = value.trunc() as u64;
    let grouped = whole
        .to_string()
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(",");
    let tenths = ((value - value.trunc()) * 10.).round() as u64;
    if tenths == 0 {
        format!("{grouped}{unit}")
    } else {
        format!("{grouped}.{tenths}{unit}")
    }
}

/// "371.7kB · 09/19/26, 14:46", or "Folder" for a directory.
pub fn describe_file(file: &ContentFile) -> String {
    let mut line = if file.is_directory {
        t("installedContentFolderEntry").to_string()
    } else {
        format_size(file.size)
    };
    if file.modified != 0
        && let Some(date) = Local.timestamp_millis_opt(file.modified).single()
    {
        line.push_str(" · ");
        line.push_str(&date.format("%m/%d/%y, %H:%M").to_string());
    }
    line
}

/// A copy or delete. At most one runs at a time.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Copy {
        control: Control,
        to: String,
        to_title: SharedString,
        groups: Vec<SectionFiles>,
    },
    Delete {
        control: Control,
        groups: Vec<SectionFiles>,
    },
}

impl Op {
    /// Runs one host call per group, in order, stopping at the first failure,
    /// and answers with how many files it touched.
    pub async fn perform(self, host: QuadrantHost) -> anyhow::Result<usize> {
        let mut done = 0;
        match self {
            Op::Copy { to, groups, .. } => {
                for group in groups {
                    done += host.copy_content(
                        &group.section.location,
                        &to,
                        group.section.mod_type,
                        group.file_names,
                    )?;
                }
            }
            Op::Delete { groups, .. } => {
                for group in groups {
                    done += host.delete_content(
                        &group.section.location,
                        group.section.mod_type,
                        group.file_names,
                    )?;
                }
            }
        }
        Ok(done)
    }

    fn control(&self) -> &Control {
        match self {
            Op::Copy { control, .. } | Op::Delete { control, .. } => control,
        }
    }

    fn done_text(&self, done: usize) -> SharedString {
        match self {
            Op::Copy { to_title, .. } => {
                t_count("installedContentCopied", done, &[("name", to_title)])
            }
            Op::Delete { groups, .. } => match groups.as_slice() {
                [only] if only.file_names.len() == 1 => {
                    t_with("installedContentDeleted", &[("file", &only.file_names[0])])
                }
                _ => t_count("installedContentDeletedCount", done, &[]),
            },
        }
    }
}

pub struct InstalledContentPage {
    locations: Loadable<Vec<ContentLocation>>,
    running: Option<Op>,
    selection: Vec<SelectedFile>,
    /// The delete control waiting for its confirming click.
    armed: Option<Control>,
    disarm: Task<()>,
    /// Sections the user opened. Every section starts folded.
    open_sections: Vec<SectionRef>,
    /// Locations after the first whose card the user folded or unfolded;
    /// the rest start open when they hold anything.
    toggled_locations: HashSet<String>,
    /// The newest listing request; replacing it drops a stale one's answer.
    listing: Task<()>,
}

impl InstalledContentPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            locations: Loadable::Idle,
            running: None,
            selection: Vec::new(),
            armed: None,
            disarm: Task::ready(()),
            open_sections: Vec::new(),
            toggled_locations: HashSet::new(),
            listing: Task::ready(()),
        };
        page.reload(cx);
        page
    }

    /// Replaces the listing in place, so the sections the user opened stay open.
    fn reload(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.locations, Loadable::Ready(_)) {
            self.locations = Loadable::Loading;
        }
        let task =
            Backend::global(cx).run(cx, |host| async move { host.get_installed_content(true) });
        self.listing = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(locations) = &result {
                    prune_selection(&mut this.selection, locations);
                }
                this.locations = Loadable::from_result(result);
                cx.notify();
            });
        });
        cx.notify();
    }

    fn run(&mut self, op: Op, cx: &mut Context<Self>) {
        let empty = match &op {
            Op::Copy { groups, .. } | Op::Delete { groups, .. } => groups.is_empty(),
        };
        if self.running.is_some() || empty {
            return;
        }
        self.armed = None;
        self.running = Some(op.clone());
        let task = Backend::global(cx).run(cx, {
            let op = op.clone();
            move |host| op.perform(host)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                match result {
                    Ok(done) => snackbar::show(cx, SnackbarKind::Success, op.done_text(done)),
                    Err(error) => snackbar::show(cx, SnackbarKind::Error, describe(error)),
                }
                // Also after a failure: an earlier group may already have moved.
                this.reload(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Arms the control, or runs the delete when it is already armed.
    fn ask_delete(&mut self, control: Control, groups: Vec<SectionFiles>, cx: &mut Context<Self>) {
        if self.running.is_some() || groups.is_empty() {
            return;
        }
        if self.armed.as_ref() != Some(&control) {
            self.armed = Some(control);
            self.disarm = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(ARM_TIMEOUT).await;
                let _ = this.update(cx, |this, cx| {
                    this.armed = None;
                    cx.notify();
                });
            });
            cx.notify();
            return;
        }
        self.run(Op::Delete { control, groups }, cx);
    }

    fn open_folder(&mut self, section: SectionRef, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, move |host| async move {
            let folder = host.content_folder(&section.location, section.mod_type)?;
            std::fs::create_dir_all(&folder)?;
            Ok(folder)
        });
        cx.spawn(async move |_, cx| match task.await {
            Ok(folder) => cx.update(|cx| cx.open_with_system(&folder)),
            Err(error) => cx.update(|cx| snackbar::show(cx, SnackbarKind::Error, describe(error))),
        })
        .detach();
    }

    fn toggle_section(&mut self, section: SectionRef, cx: &mut Context<Self>) {
        match self.open_sections.iter().position(|each| *each == section) {
            Some(index) => {
                self.open_sections.remove(index);
            }
            None => self.open_sections.push(section),
        }
        cx.notify();
    }

    /// Opens every section and ticks the first file, so a screenshot shows
    /// the rows and the selection bar.
    #[cfg(feature = "screenshot")]
    pub fn preview_selection(&mut self, cx: &mut Context<Self>) {
        let Loadable::Ready(locations) = &self.locations else {
            return;
        };
        for location in locations {
            for section in &location.sections {
                let section_ref = SectionRef {
                    location: location.id.clone(),
                    mod_type: section.mod_type,
                };
                if let Some(file) = section.files.first()
                    && self.selection.is_empty()
                {
                    self.selection.push(SelectedFile {
                        section: section_ref.clone(),
                        file_name: file.file_name.clone(),
                    });
                }
                if !self.open_sections.contains(&section_ref) {
                    self.open_sections.push(section_ref);
                }
            }
        }
        cx.notify();
    }

    /// A "copy to…" button with a menu of the targets. Without `label` it is
    /// the compact per-file control; with one it is a bulk control and each
    /// entry counts what would be copied there.
    fn copy_menu(
        &self,
        control: Control,
        label: Option<SharedString>,
        targets: Vec<CopyTarget>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = self.running.is_some();
        let bulk = label.is_some();
        let button = match label {
            Some(label) => theme::small_pill_button(
                control.id("copy"),
                Tone::Control,
                label,
                Some(MdIcon::ContentCopy),
                cx,
            ),
            None => {
                theme::toolbar_button(control.id("copy"), Tone::Control, MdIcon::ContentCopy, cx)
            }
        }
        .loading(self.running.as_ref().map(Op::control) == Some(&control))
        .disabled(busy);
        let page = cx.entity();
        button.dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |mut menu, _, _| {
            for target in &targets {
                let count = target.count();
                let text: SharedString = if bulk {
                    t_with(
                        "installedContentCopyMissing",
                        &[("name", &target.title), ("missing", &count.to_string())],
                    )
                } else {
                    target.title.clone()
                };
                let op = Op::Copy {
                    control: control.clone(),
                    to: target.location.clone(),
                    to_title: target.title.clone(),
                    groups: target.groups.clone(),
                };
                let page = page.clone();
                menu = menu.item(
                    PopupMenuItem::new(text)
                        .disabled(count == 0)
                        .when(count == 0, |item| item.icon(Icon::new(MdIcon::Check)))
                        .on_click(move |_, _, cx| {
                            page.update(cx, |page, cx| page.run(op.clone(), cx))
                        }),
                );
            }
            menu
        })
    }

    /// The delete control: icon-only per file until armed, labelled in bulk.
    fn delete_button(
        &self,
        control: Control,
        text: Option<SharedString>,
        armed_text: SharedString,
        groups: Vec<SectionFiles>,
        cx: &mut Context<Self>,
    ) -> Button {
        let armed = self.armed.as_ref() == Some(&control);
        let id = control.id("delete");
        let button = match (armed, text) {
            (true, _) => {
                theme::small_pill_button(id, Tone::Danger, armed_text, Some(MdIcon::Delete), cx)
            }
            (false, Some(text)) => {
                theme::small_pill_button(id, Tone::Destructive, text, Some(MdIcon::Delete), cx)
            }
            (false, None) => theme::toolbar_button(id, Tone::Destructive, MdIcon::Delete, cx),
        };
        let loading = self.running.as_ref().map(Op::control) == Some(&control);
        button
            .loading(loading)
            .disabled(self.running.is_some())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.ask_delete(control.clone(), groups.clone(), cx)
            }))
    }

    fn render_selection_bar(
        &self,
        locations: &[ContentLocation],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let count = self.selection.len();
        let groups = group_by_section(&self.selection);
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .mx_6()
            .mt_1()
            .px_4()
            .py_3()
            .rounded(theme::radius_card())
            .bg(rgb(theme::PAGE))
            .child(
                div()
                    .mr_auto()
                    .text_sm()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(t_count("installedContentSelected", count, &[])),
            )
            .child(
                theme::small_pill_button(
                    "clear-selection",
                    Tone::Control,
                    t("installedContentClearSelection"),
                    None,
                    cx,
                )
                .disabled(self.running.is_some())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.selection.clear();
                    cx.notify();
                })),
            )
            .when(locations.len() > 1, |this| {
                this.child(self.copy_menu(
                    Control::Selection,
                    Some(t("installedContentCopySelected")),
                    copy_targets(locations, &self.selection),
                    cx,
                ))
            })
            .child(self.delete_button(
                Control::Selection,
                Some(t("installedContentDeleteSelected")),
                t_with(
                    "installedContentDeleteSelectedConfirm",
                    &[("amount", &count.to_string())],
                ),
                groups,
                cx,
            ))
    }

    fn render_file(
        &self,
        locations: &[ContentLocation],
        section: &SectionRef,
        file: &ContentFile,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entry = SelectedFile {
            section: section.clone(),
            file_name: file.file_name.clone(),
        };
        let control = Control::File(section.clone(), file.file_name.clone());
        let selected = self.selection.contains(&entry);
        let busy = self.running.is_some();
        h_flex()
            .items_center()
            .gap_3()
            .my_1()
            .px_4()
            .py_2()
            .rounded(theme::radius_card())
            .bg(rgb(theme::PAGE))
            .child(
                theme::checkbox(control.id("tick"), selected)
                    .when(busy, |this| this.opacity(0.5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.running.is_none() {
                            toggle_selection(&mut this.selection, entry.clone());
                            cx.notify();
                        }
                    })),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(file.file_name.clone()),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(describe_file(file)),
                    ),
            )
            .when(locations.len() > 1, |this| {
                let files = [SelectedFile {
                    section: section.clone(),
                    file_name: file.file_name.clone(),
                }];
                this.child(self.copy_menu(
                    control.clone(),
                    None,
                    copy_targets(locations, &files),
                    cx,
                ))
            })
            .child(self.delete_button(
                control,
                None,
                t("installedContentDeleteConfirm"),
                vec![SectionFiles {
                    section: section.clone(),
                    file_names: vec![file.file_name.clone()],
                }],
                cx,
            ))
    }

    fn render_section(
        &self,
        locations: &[ContentLocation],
        location: &ContentLocation,
        files: &[ContentFile],
        section: SectionRef,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = self.open_sections.contains(&section);
        let title = section_title(section.mod_type);
        let names: Vec<String> = files.iter().map(|file| file.file_name.clone()).collect();
        let all: Vec<SelectedFile> = names
            .iter()
            .map(|name| SelectedFile {
                section: section.clone(),
                file_name: name.clone(),
            })
            .collect();
        let header = h_flex()
            .flex_wrap()
            .items_center()
            .gap_x_3()
            .gap_y_2()
            .child(
                h_flex()
                    .id(Control::CopyAll(section.clone()).id("fold"))
                    .min_w_0()
                    .flex_1()
                    .items_center()
                    .gap_x_3()
                    .cursor_pointer()
                    .on_click(cx.listener({
                        let section = section.clone();
                        move |this, _, _, cx| this.toggle_section(section.clone(), cx)
                    }))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(files.len().to_string()),
                    )
                    .child(expand_icon(open).size_5()),
            )
            .when(locations.len() > 1 && !files.is_empty(), |this| {
                this.child(self.copy_menu(
                    Control::CopyAll(section.clone()),
                    Some(t("installedContentCopyAll")),
                    copy_targets(locations, &all),
                    cx,
                ))
            })
            .child(
                theme::small_pill_button(
                    Control::CopyAll(section.clone()).id("open"),
                    Tone::Control,
                    t("installedContentOpenFolder"),
                    Some(MdIcon::Folder),
                    cx,
                )
                .tooltip(t_with(
                    "installedContentOpenFolderLabel",
                    &[
                        ("section", &section_title(section.mod_type)),
                        ("name", &location_title(location)),
                    ],
                ))
                .on_click(cx.listener({
                    let section = section.clone();
                    move |this, _, _, cx| this.open_folder(section.clone(), cx)
                })),
            );
        let body = if !open {
            None
        } else if files.is_empty() {
            Some(
                div()
                    .mt_3()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(t("installedContentEmpty"))
                    .into_any_element(),
            )
        } else {
            Some(
                v_flex()
                    .child(
                        h_flex().mt_3().child(
                            theme::small_pill_button(
                                Control::CopyAll(section.clone()).id("select-all"),
                                Tone::Control,
                                t("installedContentSelectAll"),
                                None,
                                cx,
                            )
                            .disabled(self.running.is_some())
                            .on_click(cx.listener({
                                let section = section.clone();
                                move |this, _, _, cx| {
                                    select_all_in(&mut this.selection, &section, &names);
                                    cx.notify();
                                }
                            })),
                        ),
                    )
                    .child(
                        v_flex().mt_3().children(
                            files
                                .iter()
                                .map(|file| {
                                    self.render_file(locations, &section, file, cx)
                                        .into_any_element()
                                })
                                .collect::<Vec<_>>(),
                        ),
                    )
                    .into_any_element(),
            )
        };
        v_flex()
            .my_3()
            .p_4()
            .rounded(theme::radius_card())
            .bg(rgb(theme::SURFACE))
            .child(header)
            .children(body)
    }

    fn render_location(
        &self,
        locations: &[ContentLocation],
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let location = &locations[index];
        let has_content = location
            .sections
            .iter()
            .any(|section| !section.files.is_empty());
        let collapsible = index > 0;
        let open = !collapsible || has_content != self.toggled_locations.contains(&location.id);
        let header = v_flex()
            .min_w_0()
            .flex_1()
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x_3()
                    .gap_y_1()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child(location_title(location)),
                    )
                    .when(location.kind == ContentLocationKind::Prism, |this| {
                        this.child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded_full()
                                .bg(rgb(theme::CONTROL))
                                .text_xs()
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .text_color(rgb(theme::TEXT_SOFT))
                                .child(t("prismLauncher")),
                        )
                    }),
            )
            .child(
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(location.path.clone()),
            );
        let header = if collapsible {
            let id = location.id.clone();
            h_flex()
                .id(ElementId::Name(format!("location:{id}").into()))
                .w_full()
                .items_center()
                .gap_3()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.toggled_locations.remove(&id) {
                        this.toggled_locations.insert(id.clone());
                    }
                    cx.notify();
                }))
                .child(header)
                .child(expand_icon(open).size_6())
                .into_any_element()
        } else {
            header.into_any_element()
        };
        v_flex()
            .mx_5()
            .my_5()
            .p_4()
            .rounded(theme::radius_card())
            .bg(rgb(theme::PAGE))
            .child(header)
            .when(open, |this| {
                this.children(
                    location
                        .sections
                        .iter()
                        .map(|section| {
                            let section_ref = SectionRef {
                                location: location.id.clone(),
                                mod_type: section.mod_type,
                            };
                            self.render_section(
                                locations,
                                location,
                                &section.files,
                                section_ref,
                                cx,
                            )
                            .into_any_element()
                        })
                        .collect::<Vec<_>>(),
                )
            })
    }
}

/// The desktop app's `MdExpandMore`, turned over while its panel is open.
fn expand_icon(open: bool) -> Icon {
    Icon::new(MdIcon::ExpandMore).when(open, |icon| {
        icon.rotate(gpui::radians(std::f32::consts::PI))
    })
}

impl Render for InstalledContentPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = h_flex()
            .items_center()
            .gap_2()
            .mx_8()
            .my_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_2xl()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(t("installedContent")),
            )
            .child(
                theme::toolbar_button("refresh", Tone::Control, MdIcon::Refresh, cx)
                    .tooltip(t("installedContentRefresh"))
                    // A reload mid-action would race the reload the action does.
                    .disabled(self.running.is_some())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.locations = Loadable::Loading;
                        this.selection.clear();
                        this.reload(cx)
                    })),
            );
        let body = match &self.locations {
            Loadable::Idle | Loadable::Loading => h_flex()
                .flex_1()
                .p_8()
                .justify_center()
                .child(Spinner::new().large())
                .into_any_element(),
            // A failed load leaves the page usable; refresh is the retry.
            Loadable::Failed(error) => h_flex()
                .p_8()
                .justify_center()
                .text_color(rgb(theme::TEXT_MUTED))
                .child(error.clone())
                .into_any_element(),
            Loadable::Ready(locations) if locations.is_empty() => div().into_any_element(),
            Loadable::Ready(locations) => {
                let locations = locations.clone();
                v_flex()
                    .when(!self.selection.is_empty(), |this| {
                        this.child(self.render_selection_bar(&locations, cx))
                    })
                    .child(
                        v_flex()
                            .mx_6()
                            .my_4()
                            .rounded(theme::radius_card())
                            .bg(rgb(theme::SURFACE))
                            .children(
                                (0..locations.len())
                                    .map(|index| {
                                        self.render_location(&locations, index, cx)
                                            .into_any_element()
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                    )
                    .into_any_element()
            }
        };
        v_flex()
            .id("installed-content")
            .size_full()
            .overflow_y_scrollbar()
            .child(header)
            .child(body)
            .child(div().h_1())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_core::content::ContentSection;
    use quadrant_host::QuadrantHostOptions;

    fn file(name: &str) -> ContentFile {
        ContentFile {
            file_name: name.into(),
            size: 10,
            modified: 0,
            is_directory: false,
        }
    }

    fn location(id: &str, packs: &[&str], shaders: &[&str]) -> ContentLocation {
        ContentLocation {
            id: id.into(),
            kind: if id == "minecraft" {
                ContentLocationKind::Minecraft
            } else {
                ContentLocationKind::Prism
            },
            name: id.into(),
            path: format!("/{id}"),
            sections: vec![
                ContentSection {
                    mod_type: ModType::ResourcePack,
                    files: packs.iter().map(|name| file(name)).collect(),
                },
                ContentSection {
                    mod_type: ModType::ShaderPack,
                    files: shaders.iter().map(|name| file(name)).collect(),
                },
            ],
        }
    }

    fn pick(location: &str, mod_type: ModType, name: &str) -> SelectedFile {
        SelectedFile {
            section: SectionRef {
                location: location.into(),
                mod_type,
            },
            file_name: name.into(),
        }
    }

    #[test]
    fn copy_targets_skip_the_source_and_what_is_already_there() {
        let locations = [
            location("minecraft", &["a.zip", "b.zip"], &["s.zip"]),
            location("prism:one", &["a.zip"], &[]),
        ];
        let files = [
            pick("minecraft", ModType::ResourcePack, "a.zip"),
            pick("minecraft", ModType::ResourcePack, "b.zip"),
            pick("minecraft", ModType::ShaderPack, "s.zip"),
        ];
        let targets = copy_targets(&locations, &files);
        assert_eq!(
            targets.len(),
            1,
            "the Minecraft folder is where they all came from"
        );
        let prism = &targets[0];
        assert_eq!(prism.location, "prism:one");
        assert_eq!(prism.count(), 2);
        assert_eq!(prism.groups[0].file_names, ["b.zip"]);
        assert_eq!(prism.groups[1].section.mod_type, ModType::ShaderPack);

        let nothing_new = copy_targets(&locations, &files[..1]);
        assert_eq!(nothing_new[0].count(), 0);
    }

    #[test]
    fn selection_ticks_groups_and_prunes() {
        let mut selection = Vec::new();
        toggle_selection(
            &mut selection,
            pick("minecraft", ModType::ShaderPack, "s.zip"),
        );
        let packs = SectionRef {
            location: "minecraft".into(),
            mod_type: ModType::ResourcePack,
        };
        select_all_in(&mut selection, &packs, &["a.zip".into(), "b.zip".into()]);
        select_all_in(&mut selection, &packs, &["a.zip".into()]);
        assert_eq!(selection.len(), 3);
        let groups = group_by_section(&selection);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].section.mod_type, ModType::ShaderPack);
        assert_eq!(count_files(&groups), 3);

        toggle_selection(
            &mut selection,
            pick("minecraft", ModType::ShaderPack, "s.zip"),
        );
        assert_eq!(selection.len(), 2);
        prune_selection(&mut selection, &[location("minecraft", &["b.zip"], &[])]);
        assert_eq!(
            selection,
            [pick("minecraft", ModType::ResourcePack, "b.zip")]
        );
    }

    #[test]
    fn sizes_read_like_the_desktop_app() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(1000), "1,000B");
        assert_eq!(format_size(380_621), "371.7kB");
        assert_eq!(format_size(2 << 20), "2MB");
        assert_eq!(format_size(1_782_579), "1.7MB");
        assert_eq!(format_size(3 << 30), "3GB");
        let folder = ContentFile {
            is_directory: true,
            ..file("pack")
        };
        assert_eq!(describe_file(&folder), "Folder");
    }

    #[test]
    fn done_texts_count_and_name() {
        let one = Op::Delete {
            control: Control::Selection,
            groups: group_by_section(&[pick("minecraft", ModType::ShaderPack, "s.zip")]),
        };
        assert_eq!(one.done_text(1), "Deleted s.zip");
        let copy = Op::Copy {
            control: Control::Selection,
            to: "prism:one".into(),
            to_title: "One".into(),
            groups: Vec::new(),
        };
        assert_eq!(copy.done_text(2), "Copied 2 packs to One");
    }

    /// Copies and deletes through the host between a throwaway Minecraft
    /// folder and a throwaway Prism Launcher instance.
    #[test]
    fn ops_copy_and_delete_packs() {
        let temp = tempfile::tempdir().unwrap();
        let mc = temp.path().join("mc");
        let packs = mc.join("resourcepacks");
        std::fs::create_dir_all(&packs).unwrap();
        std::fs::write(packs.join("a.zip"), "a").unwrap();
        std::fs::write(packs.join("b.zip"), "b").unwrap();
        let prism = temp.path().join("prism");
        let instance = prism.join("instances").join("survival");
        std::fs::create_dir_all(instance.join("minecraft")).unwrap();
        std::fs::write(
            instance.join("instance.cfg"),
            "[General]
name=Survival
",
        )
        .unwrap();
        std::fs::write(
            instance.join("mmc-pack.json"),
            r#"{"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.21.1"}]}"#,
        )
        .unwrap();
        let mut options = QuadrantHostOptions::new(temp.path().join("data"), "", "", "");
        options.mc_folder = Some(mc.clone());
        options.keyring_service_name = "quadrant-gpui-tests".into();
        let host = QuadrantHost::new(options).unwrap();
        host.init_config().unwrap();
        host.set_config_value("experimentalFeatures", true.into())
            .unwrap();
        // Never the real Prism Launcher folder.
        host.set_config_value("prismLauncherFolder", prism.to_string_lossy().into())
            .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let run = |op: Op| runtime.block_on(op.perform(host.clone()));

        let listed = host.get_installed_content(true).unwrap();
        let ids: Vec<_> = listed.iter().map(|location| location.id.as_str()).collect();
        assert_eq!(ids, ["minecraft", "prism:survival"]);
        assert_eq!(location_title(&listed[1]), "Survival");

        let selection = [
            pick("minecraft", ModType::ResourcePack, "a.zip"),
            pick("minecraft", ModType::ResourcePack, "b.zip"),
        ];
        let target = copy_targets(&listed, &selection).remove(0);
        assert_eq!(target.count(), 2);
        let copied = run(Op::Copy {
            control: Control::Selection,
            to: target.location,
            to_title: target.title,
            groups: target.groups,
        })
        .unwrap();
        assert_eq!(copied, 2);
        assert!(instance.join("minecraft/resourcepacks/b.zip").is_file());
        let listed = host.get_installed_content(true).unwrap();
        assert_eq!(copy_targets(&listed, &selection)[0].count(), 0);

        let deleted = run(Op::Delete {
            control: Control::Selection,
            groups: group_by_section(&selection),
        })
        .unwrap();
        assert_eq!(deleted, 2);
        assert!(!packs.join("a.zip").exists());
        assert!(instance.join("minecraft/resourcepacks/a.zip").is_file());
        let again = run(Op::Delete {
            control: Control::Selection,
            groups: group_by_section(&selection[..1]),
        });
        assert_eq!(again.unwrap(), 0, "deleting what is gone is a no-op");
    }
}
