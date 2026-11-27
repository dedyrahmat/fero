use super::management::{ratio_text, speed_text};
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsSection {
    Downloads,
    Appearance,
    Network,
    Automation,
    Import,
    About,
}

impl SettingsSection {
    const ALL: [Self; 6] = [
        Self::Downloads,
        Self::Appearance,
        Self::Network,
        Self::Automation,
        Self::Import,
        Self::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Downloads => "Downloads",
            Self::Appearance => "Appearance",
            Self::Network => "Network & remote access",
            Self::Automation => "Automation & startup",
            Self::Import => "Import library",
            Self::About => "About Fero",
        }
    }
}

fn narrow_settings(window: &Window) -> bool {
    window.viewport_size().width < px(732.)
}

impl Desktop {
    pub(crate) fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.is_some() || self.editor.is_some() {
            return;
        }
        if !self.settings {
            self.show_settings(true, window, cx);
        }
        self.settings_section = Some(SettingsSection::About);
        self.settings_scroll.set_offset(Point::default());
        window.focus(&self.focus);
        cx.notify();
    }

    pub(super) fn back_from_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if narrow_settings(window) && self.settings_section.take().is_some() {
            self.settings_scroll.set_offset(Point::default());
            window.focus(&self.focus);
            cx.notify();
        } else {
            self.show_settings(false, window, cx);
        }
    }

    fn settings_categories(&self, narrow: bool, palette: Palette, cx: &Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(
                SettingsSection::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, section)| {
                        let active = !narrow
                            && self.settings_section.unwrap_or(SettingsSection::Downloads)
                                == section;
                        control(("settings-category", index), palette)
                            .rounded(px(10.))
                            .px_3()
                            .py_2()
                            .text_sm()
                            .child(section.label())
                            .when(active, |el| {
                                el.bg(palette.select)
                                    .text_color(palette.accent)
                                    .font_weight(FontWeight::BOLD)
                            })
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.settings_section = Some(section);
                                view.settings_scroll.set_offset(Point::default());
                                if narrow {
                                    window.focus(&view.focus);
                                }
                                cx.notify();
                            }))
                    }),
            )
    }

    pub(super) fn settings_sidebar(&self, palette: Palette, cx: &Context<Self>) -> Stateful<Div> {
        div()
            .id("settings-sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .min_h_0()
            .w(px(216.))
            .gap_4()
            .px_3()
            .pt(px(25.))
            .pb_4()
            .overflow_y_scroll()
            .bg(palette.rail)
            .border_r_1()
            .border_color(palette.line)
            .child(self.back_settings_button("Back to Library", palette, cx))
            .child(self.settings_categories(false, palette, cx))
    }

    fn back_settings_button(
        &self,
        label: &'static str,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        button(
            "settings-back",
            icon_label("arrow-left", label, palette.ink),
            palette,
            false,
        )
        .on_click(cx.listener(|view, _, window, cx| view.back_from_settings(window, cx)))
    }

    pub(super) fn settings(
        &self,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let width = window.viewport_size().width;
        let narrow = narrow_settings(window);
        let section = if narrow {
            self.settings_section
        } else {
            Some(self.settings_section.unwrap_or(SettingsSection::Downloads))
        };
        let panel = match section {
            Some(SettingsSection::Downloads) => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.download_settings(palette, window, cx))
                .child(self.transfer_limits(palette, cx))
                .child(self.schedule_settings(palette, cx)),
            Some(SettingsSection::Appearance) => self.appearance_settings(palette, window, cx),
            Some(SettingsSection::Network) => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.network_settings(palette, cx))
                .child(self.lan_settings(palette, cx)),
            Some(SettingsSection::Automation) => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.watched_folders(palette, cx))
                .child(self.rss_settings(palette, cx))
                .child(self.startup_settings(palette, cx)),
            Some(SettingsSection::Import) => self.import_settings(palette, window, cx),
            Some(SettingsSection::About) => self.about_settings(palette, cx),
            None => settings_card(palette).child(self.settings_categories(true, palette, cx)),
        };
        div()
            .id("settings-page")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.settings_scroll)
            .px(px(if narrow { 14. } else { 28. }))
            .pt(px(if narrow { 22. } else { 30. }))
            .pb(px(44.))
            .child(
                div()
                    .w_full()
                    .max_w(px(820.))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .when(narrow, |el| {
                        let label = if section.is_some() {
                            "Back to Settings"
                        } else {
                            "Back to Library"
                        };
                        el.child(div().child(self.back_settings_button(label, palette, cx)))
                    })
                    .child(self.heading(
                        section.map_or("Settings", SettingsSection::label),
                        heading_size(width, 32.),
                    ))
                    .child(panel),
            )
    }

    fn about_settings(&self, palette: Palette, cx: &Context<Self>) -> Div {
        use crate::about;

        settings_card(palette)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(img(BRAND).size(px(56.)).flex_none())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(self.heading("Fero", 24.))
                            .child(quiet(format!("Version {}", about::VERSION), palette)),
                    ),
            )
            .child("A native BitTorrent client for everyday downloads and large libraries.")
            .child(quiet(about::NAME_ORIGIN, palette))
            .child(self.heading("Credits", 18.))
            .child(about::CORE_CREDIT)
            .child(
                "DM Sans and Instrument Sans by their project authors · SIL Open Font License 1.1.",
            )
            .child(about::SOLAR_CREDIT)
            .child(
                div().flex().flex_wrap().gap_2().children(
                    [
                        ("Solar Icons source", about::SOLAR_SOURCE),
                        ("CC BY 4.0 license", about::SOLAR_LICENSE),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, url))| {
                        button(("about-credit-link", index), label, palette, false)
                            .on_click(move |_, _, cx| cx.open_url(url))
                    }),
                ),
            )
            .child(
                div().child(
                    button(
                        "about-notices",
                        if self.about_notices_open {
                            "Hide license notices"
                        } else {
                            "Show license notices"
                        },
                        palette,
                        false,
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.about_notices_open = !view.about_notices_open;
                        cx.notify();
                    })),
                ),
            )
            .when(self.about_notices_open, |el| {
                el.children(about::NOTICES.iter().map(|&(name, text)| {
                    settings_row(name, palette)
                        .child(div().w_full().min_w_0().text_sm().child(text))
                }))
            })
    }

    fn appearance_settings(&self, palette: Palette, window: &Window, cx: &Context<Self>) -> Div {
        let prefs = &self.preferences;
        let themes = div().flex().gap(px(7.)).children(
            [
                (Theme::System, "System"),
                (Theme::Light, "Light"),
                (Theme::Dark, "Dark"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (theme, label))| {
                let thumbnail = |mode| {
                    let preview = Palette::new(window.appearance(), mode, prefs.accent);
                    let rail = preview.outline;
                    div()
                        .flex()
                        .flex_1()
                        .gap_1()
                        .p(px(7.))
                        .bg(preview.paper)
                        .map(|preview| match (theme, mode) {
                            (Theme::System, Theme::Dark) => preview.rounded_r(px(16.)),
                            (Theme::System, _) => preview.rounded_l(px(16.)),
                            _ => preview.rounded(px(16.)),
                        })
                        .child(div().w(relative(0.22)).rounded_full().bg(rail))
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .pt_1()
                                .child(
                                    div()
                                        .h(px(2.))
                                        .w(relative(0.75))
                                        .rounded_full()
                                        .bg(preview.accent),
                                )
                                .children((0..3).map(|_| div().h(px(3.)).rounded_full().bg(rail))),
                        )
                };
                control(("theme", i), palette)
                    .flex_1()
                    .min_w_0()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .h(px(58.))
                            .rounded(px(18.))
                            .overflow_hidden()
                            .border_2()
                            .border_color(if theme == prefs.theme {
                                palette.accent
                            } else {
                                palette.outline
                            })
                            .child(thumbnail(if theme == Theme::Dark {
                                Theme::Dark
                            } else {
                                Theme::Light
                            }))
                            .when(theme == Theme::System, |el| {
                                el.child(thumbnail(Theme::Dark))
                            }),
                    )
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.preferences.theme = theme;
                        view.save_preferences(cx);
                    }))
            }),
        );
        let palettes = div().flex().flex_wrap().gap_2().children(
            [
                (Accent::Harbor, "Harbor Current"),
                (Accent::Ember, "Ember"),
                (Accent::Sage, "Sage"),
                (Accent::Indigo, "Indigo"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (accent, label))| {
                let swatch = Palette::new(window.appearance(), prefs.theme, accent);
                control(("accent", i), palette)
                    .rounded_full()
                    .px(px(9.))
                    .py_1()
                    .min_h(px(32.))
                    .gap_1()
                    .text_xs()
                    .bg(palette.field)
                    .border_color(if accent == prefs.accent {
                        palette.accent
                    } else {
                        palette.line
                    })
                    .child(label)
                    .child(div().size(px(9.)).rounded_full().bg(swatch.accent))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.preferences.accent = accent;
                        view.save_preferences(cx);
                    }))
            }),
        );
        let spacing = div().flex().flex_wrap().gap_2().children(
            [(false, "Roomy"), (true, "Compact")]
                .into_iter()
                .enumerate()
                .map(|(i, (compact, label))| {
                    choice(("row-spacing", i), label, prefs.compact == compact, palette).on_click(
                        cx.listener(move |view, _, _, cx| {
                            view.preferences.compact = compact;
                            view.save_preferences(cx);
                        }),
                    )
                }),
        );
        settings_card(palette)
            .child(themes)
            .child(settings_row("Color palette", palette).child(palettes))
            .child(settings_row("Transfer row spacing", palette).child(spacing))
            .child(
                settings_row("Motion", palette).child(
                    checkbox(
                        "settings-motion",
                        "Reduce motion",
                        prefs.reduce_motion,
                        palette,
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.preferences.reduce_motion = !view.preferences.reduce_motion;
                        view.save_preferences(cx);
                    })),
                ),
            )
            .child(
                settings_row("Focus mode", palette).child(
                    checkbox(
                        "settings-focus",
                        "Collapse sidebar",
                        prefs.collapsed,
                        palette,
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_sidebar(cx))),
                ),
            )
    }

    fn download_settings(&self, palette: Palette, window: &Window, cx: &Context<Self>) -> Div {
        let prefs = &self.preferences;
        let folder = prefs.download_folder();
        let edited_folder = self.default_folder_field.read(cx).text.as_str()
            != folder.as_deref().and_then(Path::to_str).unwrap_or_default();
        let folder_controls = Palette {
            interactive: palette.interactive && !self.folder_saving,
            ..palette
        };
        settings_card(palette)
            .child(
                settings_row("Default save folder", palette)
                    .child(quiet(
                        folder.as_ref().map_or_else(
                            || "No folder is set".into(),
                            |folder| format!("Saved folder: {}", folder.display()),
                        ),
                        palette,
                    ))
                    .child(self.input(
                        "default-folder-frame",
                        &self.default_folder_field,
                        palette,
                        !folder_controls.interactive,
                        window,
                        cx,
                    ))
                    .child(quiet(
                        "You can choose a different folder for each torrent.",
                        palette,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                button("default-folder", "Choose Folder…", folder_controls, false)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        if !view.folder_saving {
                                            view.pick_path(Picker::DefaultFolder, window, cx);
                                        }
                                    })),
                            )
                            .when(edited_folder, |el| {
                                el.child(
                                    button(
                                        "apply-default-folder",
                                        "Save Folder",
                                        folder_controls,
                                        true,
                                    )
                                    .on_click(cx.listener(
                                        |view, _, _, cx| {
                                            let folder = absolute_folder(
                                                &view.default_folder_field.read(cx).text,
                                            );
                                            if let Some(folder) = folder {
                                                view.save_default_folder(Some(folder), cx);
                                            } else {
                                                view.folder_error = Some(
                                                    "Choose a folder or enter an absolute path"
                                                        .into(),
                                                );
                                                cx.notify();
                                            }
                                        },
                                    )),
                                )
                            })
                            .when(prefs.default_folder.is_some(), |el| {
                                el.child(
                                    button(
                                        "reset-default-folder",
                                        "Use Downloads",
                                        folder_controls,
                                        false,
                                    )
                                    .on_click(cx.listener(
                                        |view, _, _, cx| {
                                            view.save_default_folder(None, cx);
                                        },
                                    )),
                                )
                            }),
                    )
                    .when_some(self.folder_error.as_ref(), |el, error| {
                        el.child(quiet(error.clone(), palette).text_color(palette.danger))
                    }),
            )
            .child(
                settings_row("Before adding a magnet", palette).child(
                    div().flex().flex_wrap().gap_2().children(
                        [(false, "Review files"), (true, "Skip file review")]
                            .into_iter()
                            .enumerate()
                            .map(|(i, (immediate, label))| {
                                choice(
                                    ("magnet-default", i),
                                    label,
                                    prefs.start_immediately == immediate,
                                    palette,
                                )
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.preferences.start_immediately = immediate;
                                        view.save_preferences(cx);
                                    },
                                ))
                            }),
                    ),
                ),
            )
            .child(settings_row("After adding", palette).child(
                div().flex().flex_wrap().gap_2().children(
                    [(false, "Start downloading"), (true, "Keep paused")].map(|(paused, label)| {
                        choice(
                            ("default-paused", usize::from(paused)),
                            label,
                            prefs.start_paused == paused,
                            palette,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.preferences.start_paused = paused;
                            view.save_preferences(cx);
                        }))
                    }),
                ),
            ))
    }

    fn transfer_limits(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let limits = self.management.limits;
        let count = |value: u32| {
            if value == 0 {
                "Unlimited".into()
            } else {
                value.to_string()
            }
        };
        let cap = |value| {
            if value == 0 {
                "Unlimited".into()
            } else {
                format!("{} KiB/s", speed_text(value))
            }
        };
        settings_card(palette)
            .child(self.heading("Transfer limits", 21.))
            .child(detail("Active downloads", count(limits.downloads), palette))
            .child(detail("Active seeds", count(limits.seeds), palette))
            .child(detail(
                "Download cap",
                cap(limits.bandwidth.download),
                palette,
            ))
            .child(detail("Upload cap", cap(limits.bandwidth.upload), palette))
            .child(detail(
                "Seeding ratio goal",
                if limits.seed_ratio == 0 {
                    "Off".into()
                } else {
                    ratio_text(limits.seed_ratio)
                },
                palette,
            ))
            .child(
                div().child(
                    button("edit-limits", "Edit Limits…", palette, false)
                        .on_click(cx.listener(|view, _, window, cx| view.edit_library(window, cx))),
                ),
            )
    }

    fn network_settings(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let saved = &self.management.network;
        let active = &self.management.active_network;
        let suspended = self.management.network_restart_required;
        settings_card(palette)
            .child(self.heading("Network", 21.))
            .when(self.management_loaded, |el| {
                el.child(detail("Saved route", network_route_text(saved), palette))
                    .child(detail(
                        "Current route",
                        if suspended {
                            format!("Suspended · {} at startup", network_route_text(active))
                        } else if self.management.network_problem.is_some() {
                            format!("Unavailable · {}", network_route_text(active))
                        } else {
                            network_route_text(active)
                        },
                        palette,
                    ))
                    .child(detail(
                        "Saved listening port",
                        if saved.listen_port == 0 {
                            "Automatic".into()
                        } else {
                            saved.listen_port.to_string()
                        },
                        palette,
                    ))
                    .child(detail(
                        "Saved peer connection limit",
                        saved.connections.to_string(),
                        palette,
                    ))
                    .when(suspended || saved != active, |el| {
                        el.child(quiet(
                            "Restart Fero to apply saved network settings.",
                            palette,
                        ))
                    })
            })
            .child(
                div().child(
                    button(
                        "edit-network",
                        "Edit Network…",
                        Palette {
                            interactive: palette.interactive && self.management_loaded,
                            ..palette
                        },
                        false,
                    )
                    .when(self.management_loaded, |button| {
                        button.on_click(
                            cx.listener(|view, _, window, cx| view.edit_network(window, cx)),
                        )
                    }),
                ),
            )
    }

    fn watched_folders(&self, palette: Palette, cx: &Context<Self>) -> Div {
        settings_card(palette)
            .child(self.heading("Watched folders", 21.))
                            .child(quiet(
                                "Fero imports torrent files into your default save folder and follows your start-paused setting.",
                                palette,
                            ))
                            .children(self.management.watch_folders.iter().map(|path| {
                                quiet(path.display().to_string(), palette)
                            }))
                            .when_some(self.management.watch_problem.as_ref(), |el, error| {
                                el.child(quiet(error.clone(), palette).text_color(palette.danger))
                            })
                            .when(self.management_loaded, |el| {
                                el.child(
                                    div().flex().flex_wrap().gap_2().child(
                                        button(
                                            "choose-watch-folders",
                                            if self.management.watch_folders.is_empty() {
                                                "Choose Watch Folders…"
                                            } else {
                                                "Change Watch Folders…"
                                            },
                                            palette,
                                            false,
                                        )
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.pick_path(Picker::WatchFolders, window, cx);
                                            })),
                                    )
                                    .when(!self.management.watch_folders.is_empty(), |el| {
                                        el.child(
                                            button("clear-watch-folders", "Clear Watch Folders", palette, false)
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    view.send(Command::SaveWatchFolders(Vec::new()), cx);
                                                })),
                                        )
                                    }),
                                )
                            })
    }

    fn import_settings(&self, palette: Palette, window: &Window, cx: &Context<Self>) -> Div {
        let controls = Palette {
            interactive: palette.interactive && !self.migration_importing,
            ..palette
        };
        settings_card(palette)
            .child(self.heading("Import from qBittorrent", 21.))
            .child(quiet(
                "Choose a profile folder containing torrents.db or a BT_backup directory. Fero imports selected transfers paused and checks existing files.",
                palette,
            ))
            .child(div().child(
                button("choose-qbit-profile", "Choose qBittorrent Profile…", controls, false)
                    .when(controls.interactive, |el| el.on_click(cx.listener(|view, _, window, cx| {
                        view.pick_path(Picker::QbitProfile, window, cx);
                    }))),
            ))
            .when_some(self.migration.as_ref(), |el, preview| {
                let can_import = controls.interactive && !self.migration_selected.is_empty();
                el.child(self.heading("Review transfers", 18.))
                    .child(quiet(format!(
                        "{} torrents · {} selected · {} duplicates · {} missing paths",
                        preview.items.len(),
                        self.migration_selected.len(),
                        preview.items.iter().filter(|item| item.duplicate).count(),
                        preview.items.iter().filter(|item| item.missing).count(),
                    ), palette))
                    .when(!self.migration_invalid.is_empty(), |el| {
                        el.child(quiet("Correct or skip the marked destinations to continue.", palette).text_color(palette.danger))
                    })
                    .children(preview.items.iter().enumerate().map(|(index, item)| {
                        self.import_row(index, item, controls, window, cx)
                    }))
                    .child(div().child(button(
                        "import-qbit",
                        if self.migration_importing { "Importing…" } else { "Import selected paused" },
                        Palette { interactive: can_import, ..palette },
                        can_import,
                    ).when(can_import, |el| el.on_click(cx.listener(|view, _, _, cx| {
                        view.import_selected(cx);
                    })))))
            })
    }

    fn import_row(
        &self,
        index: usize,
        item: &fero::MigrationItem,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let destination = &self.migration_destinations[&index];
        let status = if item.duplicate {
            " · already present"
        } else if item.bytes.is_empty() {
            " · .torrent missing"
        } else if item.missing {
            " · files missing"
        } else {
            ""
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(quiet(
                format!(
                    "{}{status}",
                    item.torrent
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ),
                palette,
            ))
            .when(!item.duplicate && !item.bytes.is_empty(), |el| {
                el.child(
                    checkbox(
                        ("migration-select", index),
                        "Import this transfer",
                        self.migration_selected.contains(&index),
                        palette,
                    )
                    .when(palette.interactive, |el| {
                        el.on_click(cx.listener(move |view, _, _, cx| {
                            if !view.migration_selected.remove(&index) {
                                view.migration_selected.insert(index);
                            }
                            view.migration_invalid.remove(&index);
                            cx.notify();
                        }))
                    }),
                )
                .child(self.input(
                    ("migration-destination", index),
                    destination,
                    palette,
                    !palette.interactive,
                    window,
                    cx,
                ))
                .child(
                    div().child(
                        button(
                            ("migration-pick-folder", index),
                            "Choose folder…",
                            palette,
                            false,
                        )
                        .when(palette.interactive, |el| {
                            el.on_click(cx.listener(move |view, _, window, cx| {
                                view.migration_picker =
                                    Some((index, view.migration_destinations[&index].clone()));
                                view.pick_path(Picker::MigrationDestination, window, cx);
                            }))
                        }),
                    ),
                )
                .when(self.migration_invalid.contains(&index), |el| {
                    el.child(
                        quiet(
                            "Folder unavailable — choose another folder or skip this transfer.",
                            palette,
                        )
                        .text_color(palette.danger),
                    )
                })
            })
    }

    fn import_selected(&mut self, cx: &mut Context<Self>) {
        let mut selected: Vec<_> = self
            .migration_selected
            .iter()
            .map(|&index| {
                let field = self.migration_destinations[&index].read(cx);
                (index, PathBuf::from(field.text.trim()))
            })
            .collect();
        selected.sort_unstable_by_key(|(index, _)| *index);
        if !self.migration_importing && !selected.is_empty() {
            self.migration_importing = self
                .commands
                .send(Command::ImportMigration(selected))
                .is_ok();
            if !self.migration_importing {
                self.error = Some("Transfer engine is unavailable".into());
            }
            cx.notify();
        }
    }

    fn rss_settings(&self, palette: Palette, cx: &Context<Self>) -> Div {
        settings_card(palette)
            .child(self.heading("RSS subscriptions", 21.))
            .children(
                self.management
                    .subscriptions
                    .iter()
                    .enumerate()
                    .map(|(index, feed)| {
                        let edit = feed.url.clone();
                        let remove = edit.clone();
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(quiet(feed.url.clone(), palette))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        button(("edit-feed", index), "Edit…", palette, false)
                                            .on_click(cx.listener(move |view, _, window, cx| {
                                                view.edit_feed(Some(edit.clone()), window, cx)
                                            })),
                                    )
                                    .child(
                                        button(("remove-feed", index), "Remove", palette, false)
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.send(Command::RemoveFeed(remove.clone()), cx)
                                            })),
                                    ),
                            )
                    }),
            )
            .child(
                div().flex().child(
                    button("add-feed", "Add Subscription…", palette, false).on_click(
                        cx.listener(|view, _, window, cx| view.edit_feed(None, window, cx)),
                    ),
                ),
            )
    }

    fn schedule_settings(&self, palette: Palette, cx: &Context<Self>) -> Div {
        settings_card(palette)
            .child(self.heading("Bandwidth schedule", 21.))
            .child(quiet(
                self.management.bandwidth_schedule.map_or_else(
                    || "Use different speed caps during a daily time window.".into(),
                    |s| {
                        format!(
                            "Daily {}–{} · ↓ {} / ↑ {} KiB/s",
                            s.start.format("%H:%M"),
                            s.end.format("%H:%M"),
                            speed_text(s.bandwidth.download),
                            speed_text(s.bandwidth.upload)
                        )
                    },
                ),
                palette,
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button("edit-schedule", "Edit Schedule…", palette, false).on_click(
                            cx.listener(|view, _, window, cx| view.edit_schedule(window, cx)),
                        ),
                    )
                    .when(self.management.bandwidth_schedule.is_some(), |el| {
                        el.child(
                            button("clear-schedule", "Clear Schedule", palette, false).on_click(
                                cx.listener(|view, _, _, cx| view.send(Command::ClearSchedule, cx)),
                            ),
                        )
                    }),
            )
    }

    fn startup_settings(&self, palette: Palette, cx: &Context<Self>) -> Div {
        settings_card(palette)
            .child(self.heading("Background", 21.))
            .child(quiet(
                "Use Fero’s menu-bar or tray icon to reopen the window or quit.",
                palette,
            ))
            .when_some(self.starts_at_login, |el, enabled| {
                el.child(
                    div().child(
                        button(
                            "login",
                            if enabled {
                                "Disable Start at Login"
                            } else {
                                "Start at Login"
                            },
                            palette,
                            enabled,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.send(Command::SetStartAtLogin(!enabled), cx)
                        })),
                    ),
                )
            })
    }

    fn lan_settings(&self, palette: Palette, cx: &Context<Self>) -> Div {
        settings_card(palette)
            .child(self.heading("LAN access", 21.))
            .child(quiet(
                match (&self.web_settings, self.web_running) {
                    (Some(settings), true) => {
                        format!("https://{} · Username fero", settings.address)
                    }
                    (Some(_), false) => "Stopped. Check the address and TLS files.".into(),
                    (None, _) => {
                        "Access Fero over HTTPS from a browser on your local network.".into()
                    }
                },
                palette,
            ))
            .when_some(self.web_password.as_ref(), |el, password| {
                let password = password.clone();
                let copy = password.clone();
                el.child(detail("New password", password, palette))
                    .child(div().child(
                        button("copy-web-password", "Copy Password", palette, false).on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()))
                            }),
                        ),
                    ))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        button(
                            "edit-web",
                            if self.web_settings.is_some() {
                                "Edit / Reset Password…"
                            } else {
                                "Enable LAN Access…"
                            },
                            palette,
                            false,
                        )
                        .on_click(cx.listener(|view, _, window, cx| view.edit_web(window, cx))),
                    )
                    .when(self.web_settings.is_some(), |el| {
                        el.child(button("disable-web", "Disable", palette, false).on_click(
                            cx.listener(|view, _, _, cx| view.send(Command::DisableWeb, cx)),
                        ))
                    }),
            )
    }
}

fn settings_card(palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .min_w_0()
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(palette.line)
        .bg(palette.rail)
        .shadow_sm()
        .p(px(18.))
}

fn network_route_text(network: &fero::NetworkSettings) -> String {
    if let Some(proxy) = network.socks5 {
        format!("SOCKS5 · {proxy}")
    } else if !network.vpn_interface.is_empty() {
        let interface = &network.vpn_interface;
        if let Some(dns) = network.vpn_dns {
            format!("VPN · {interface} · DNS {dns}")
        } else {
            format!("VPN · {interface}")
        }
    } else {
        "Direct".into()
    }
}
