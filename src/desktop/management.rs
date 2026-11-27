use super::*;
use crate::ManagementEdit;
use fero::{Bandwidth, LibraryLimits, NetworkSettings, Tracker, TransferOptions};

pub(super) enum EditTarget {
    Library,
    Feed(String),
    Schedule,
    Web,
    Network(NetworkRoute),
    Transfer(i64),
    Tracker(i64, String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NetworkRoute {
    Direct,
    Socks5,
    Vpn,
}

impl NetworkRoute {
    fn from_settings(network: &NetworkSettings) -> Self {
        if network.socks5.is_some() {
            Self::Socks5
        } else if !network.vpn_interface.is_empty() {
            Self::Vpn
        } else {
            Self::Direct
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direct",
            Self::Socks5 => "SOCKS5",
            Self::Vpn => "VPN",
        }
    }

    fn shows_field(self, index: usize) -> bool {
        match index {
            0 | 1 => true,
            2 => self == Self::Socks5,
            3 | 4 => self == Self::Vpn,
            _ => false,
        }
    }
}

pub(super) struct Editor {
    target: EditTarget,
    fields: Vec<(&'static str, Entity<TextField>)>,
    pub(super) saving: bool,
    previewing: bool,
    preview: Option<Vec<fero::FeedItem>>,
    pub(super) error: Option<String>,
}

impl Desktop {
    pub(super) fn edit_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let limits = self.management.limits;
        self.open_editor(
            EditTarget::Library,
            [
                (
                    "Active downloads (0 for unlimited)",
                    "0",
                    limits.downloads.to_string(),
                ),
                (
                    "Active seeds (0 for unlimited)",
                    "0",
                    limits.seeds.to_string(),
                ),
                (
                    "Download cap (KiB/s, 0 for unlimited)",
                    "0",
                    speed_text(limits.bandwidth.download),
                ),
                (
                    "Upload cap (KiB/s, 0 for unlimited)",
                    "0",
                    speed_text(limits.bandwidth.upload),
                ),
                (
                    "Seeding ratio goal (0 to disable)",
                    "0",
                    ratio_text(limits.seed_ratio),
                ),
            ],
            window,
            cx,
        );
    }

    pub(super) fn edit_feed(
        &mut self,
        url: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription = self
            .management
            .subscriptions
            .iter()
            .find(|feed| Some(&feed.url) == url.as_ref());
        self.open_editor(
            EditTarget::Feed(url.clone().unwrap_or_default()),
            [
                (
                    "Feed URL",
                    "https://example.com/feed.xml",
                    url.unwrap_or_default(),
                ),
                (
                    "Include any of these terms",
                    "Comma-separated, optional",
                    subscription
                        .map(|feed| feed.include.clone())
                        .unwrap_or_default(),
                ),
                (
                    "Exclude any of these terms",
                    "Comma-separated, optional",
                    subscription
                        .map(|feed| feed.exclude.clone())
                        .unwrap_or_default(),
                ),
            ],
            window,
            cx,
        );
    }

    pub(super) fn edit_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let schedule = self.management.bandwidth_schedule;
        self.open_editor(
            EditTarget::Schedule,
            [
                (
                    "Start (local time, HH:MM)",
                    "22:00",
                    schedule
                        .map(|s| s.start.format("%H:%M").to_string())
                        .unwrap_or_default(),
                ),
                (
                    "End (local time, HH:MM)",
                    "07:00",
                    schedule
                        .map(|s| s.end.format("%H:%M").to_string())
                        .unwrap_or_default(),
                ),
                (
                    "Download cap (KiB/s, 0 for unlimited)",
                    "0",
                    schedule
                        .map(|s| speed_text(s.bandwidth.download))
                        .unwrap_or_default(),
                ),
                (
                    "Upload cap (KiB/s, 0 for unlimited)",
                    "0",
                    schedule
                        .map(|s| speed_text(s.bandwidth.upload))
                        .unwrap_or_default(),
                ),
            ],
            window,
            cx,
        );
    }

    pub(super) fn feed_preview(&mut self, result: Result<Vec<fero::FeedItem>, String>) {
        if let Some(editor) = self.editor.as_mut().filter(|editor| editor.previewing) {
            editor.previewing = false;
            match result {
                Ok(items) => editor.preview = Some(items),
                Err(error) => editor.error = Some(error),
            }
        }
    }

    fn preview_feed(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match editor.subscription(cx) {
            Ok(feed) => {
                editor.previewing = self.commands.send(Command::PreviewFeed(feed)).is_ok();
                editor.error =
                    (!editor.previewing).then(|| "Transfer engine is unavailable".into());
            }
            Err(error) => editor.error = Some(error.to_string()),
        }
        editor.preview = None;
        cx.notify();
    }

    pub(super) fn edit_web(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.web_settings.as_ref();
        self.open_editor(
            EditTarget::Web,
            [
                (
                    "LAN IP and port",
                    "192.168.1.10:8443",
                    settings.map(|s| s.address.to_string()).unwrap_or_default(),
                ),
                (
                    "Certificate PEM path",
                    "/path/to/certificate.pem",
                    settings
                        .map(|s| s.certificate.display().to_string())
                        .unwrap_or_default(),
                ),
                (
                    "Private key PEM path",
                    "/path/to/key.pem",
                    settings
                        .map(|s| s.private_key.display().to_string())
                        .unwrap_or_default(),
                ),
            ],
            window,
            cx,
        );
    }

    pub(super) fn edit_transfer(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let options = self
            .management
            .options
            .get(&id)
            .cloned()
            .unwrap_or_default();
        self.open_editor(
            EditTarget::Transfer(id),
            [
                ("Category", "Category name", options.category),
                ("Tags (comma-separated)", "Tag names", options.tags),
                (
                    "Download cap (KiB/s, 0 for no per-torrent cap)",
                    "0",
                    speed_text(options.bandwidth.download),
                ),
                (
                    "Upload cap (KiB/s, 0 for no per-torrent cap)",
                    "0",
                    speed_text(options.bandwidth.upload),
                ),
                (
                    "Seeding ratio goal (blank for default, 0 to disable)",
                    "Library default",
                    options.seed_ratio.map(ratio_text).unwrap_or_default(),
                ),
            ],
            window,
            cx,
        );
    }

    pub(super) fn edit_network(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let network = &self.management.network;
        self.open_editor(
            EditTarget::Network(NetworkRoute::from_settings(network)),
            [
                (
                    "Preferred listening port (0 for automatic)",
                    "6881",
                    network.listen_port.to_string(),
                ),
                (
                    "Peer connection limit",
                    "300",
                    network.connections.to_string(),
                ),
                (
                    "SOCKS5 IP and port",
                    "127.0.0.1:1080",
                    network
                        .socks5
                        .map(|proxy| proxy.to_string())
                        .unwrap_or_default(),
                ),
                ("VPN interface", "utun3", network.vpn_interface.clone()),
                (
                    "VPN DNS server IP (optional)",
                    "10.0.0.1",
                    network
                        .vpn_dns
                        .map(|dns| dns.to_string())
                        .unwrap_or_default(),
                ),
            ],
            window,
            cx,
        );
    }

    pub(super) fn edit_tracker(
        &mut self,
        id: i64,
        tracker: Option<(&str, u8)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (url, tier) = tracker.unwrap_or(("", 0));
        self.open_editor(
            EditTarget::Tracker(id, url.into()),
            [
                (
                    "Announce URL",
                    "https://tracker.example/announce",
                    url.into(),
                ),
                ("Tier (0–255)", "0", tier.to_string()),
            ],
            window,
            cx,
        );
    }

    fn open_editor<const N: usize>(
        &mut self,
        target: EditTarget,
        fields: [(&'static str, &'static str, String); N],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fields: Vec<_> = fields
            .into_iter()
            .map(|(label, placeholder, value)| {
                (
                    label,
                    cx.new(|cx| {
                        let mut field = TextField::new(placeholder, false, cx);
                        field.set(value, cx);
                        field
                    }),
                )
            })
            .collect();
        if let Some((_, field)) = fields.first() {
            window.focus(&field.read(cx).focus);
        }
        self.editor = Some(Editor {
            target,
            fields,
            saving: false,
            previewing: false,
            preview: None,
            error: None,
        });
        cx.notify();
    }

    pub(super) fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        window.focus(if self.settings {
            &self.focus
        } else {
            &self.rows_focus
        });
        cx.notify();
    }

    pub(super) fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut().filter(|editor| !editor.busy()) else {
            return;
        };
        let edit = (|| -> anyhow::Result<ManagementEdit> {
            let value = |i: usize| editor.fields[i].1.read(cx).text.trim();
            let bandwidth = |start| -> anyhow::Result<Bandwidth> {
                Ok(Bandwidth {
                    download: speed(value(start))?,
                    upload: speed(value(start + 1))?,
                })
            };
            Ok(match &editor.target {
                EditTarget::Web => {
                    let address = value(0)
                        .parse()
                        .map_err(|_| anyhow::anyhow!("Enter a numeric LAN IP and port"))?;
                    let certificate = PathBuf::from(value(1));
                    let private_key = PathBuf::from(value(2));
                    anyhow::ensure!(
                        certificate.is_absolute() && private_key.is_absolute(),
                        "Enter absolute paths to the TLS files"
                    );
                    ManagementEdit::Web(crate::web::WebSettings {
                        address,
                        certificate,
                        private_key,
                    })
                }
                EditTarget::Feed(previous) => {
                    ManagementEdit::Feed(previous.clone(), editor.subscription(cx)?)
                }
                EditTarget::Schedule => {
                    let time = |index| {
                        chrono::NaiveTime::parse_from_str(value(index), "%H:%M")
                            .map_err(|_| anyhow::anyhow!("Enter local times in HH:MM format"))
                    };
                    ManagementEdit::Schedule(fero::BandwidthSchedule {
                        start: time(0)?,
                        end: time(1)?,
                        bandwidth: bandwidth(2)?,
                    })
                }
                EditTarget::Library => ManagementEdit::Library(LibraryLimits {
                    downloads: whole_number(value(0), "Active downloads")?,
                    seeds: whole_number(value(1), "Active seeds")?,
                    bandwidth: bandwidth(2)?,
                    seed_ratio: ratio(value(4))?,
                }),
                EditTarget::Network(route) => {
                    let listen_port = value(0)
                        .parse()
                        .map_err(|_| anyhow::anyhow!("Listening port must be from 0 to 65535"))?;
                    let connections = value(1)
                        .parse::<i32>()
                        .ok()
                        .filter(|&limit| limit > 0)
                        .ok_or_else(|| anyhow::anyhow!("Enter a positive peer connection limit"))?;
                    if *route == NetworkRoute::Vpn {
                        anyhow::ensure!(
                            !value(3).is_empty(),
                            "Enter a VPN interface name or Windows adapter GUID"
                        );
                    }
                    let vpn_dns = (*route == NetworkRoute::Vpn && !value(4).is_empty())
                        .then(|| {
                            value(4)
                                .parse::<std::net::SocketAddr>()
                                .or_else(|_| {
                                    value(4)
                                        .parse::<std::net::IpAddr>()
                                        .map(|ip| (ip, 53).into())
                                })
                                .map_err(|_| anyhow::anyhow!("Enter a numeric VPN DNS address"))
                        })
                        .transpose()?;
                    ManagementEdit::Network(NetworkSettings {
                        listen_port,
                        connections,
                        socks5: (*route == NetworkRoute::Socks5)
                            .then(|| value(2).parse())
                            .transpose()
                            .map_err(|_| anyhow::anyhow!("Enter a SOCKS5 IP address and port"))?,
                        vpn_interface: if *route == NetworkRoute::Vpn {
                            value(3).to_owned()
                        } else {
                            String::new()
                        },
                        vpn_dns,
                    })
                }
                EditTarget::Transfer(id) => ManagementEdit::Transfer(
                    *id,
                    TransferOptions {
                        category: value(0).into(),
                        tags: value(1).into(),
                        bandwidth: bandwidth(2)?,
                        seed_ratio: (!value(4).is_empty())
                            .then(|| ratio(value(4)))
                            .transpose()?,
                    },
                ),
                EditTarget::Tracker(id, old) => {
                    anyhow::ensure!(!value(0).is_empty(), "Enter a tracker URL");
                    let tier = whole_number(value(1), "Tracker tier")?
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("Tracker tier must be from 0 to 255"))?;
                    ManagementEdit::Tracker(*id, old.clone(), value(0).into(), tier)
                }
            })
        })();
        editor.error = match edit {
            Ok(edit) => {
                editor.saving = self.commands.send(Command::Manage(edit)).is_ok();
                (!editor.saving).then(|| "Transfer engine is unavailable".into())
            }
            Err(error) => Some(error.to_string()),
        };
        cx.notify();
    }

    pub(super) fn editor_dialog(
        &self,
        editor: &Editor,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let title = match &editor.target {
            EditTarget::Library => "Library limits",
            EditTarget::Feed(_) => "RSS subscription",
            EditTarget::Schedule => "Bandwidth schedule",
            EditTarget::Web => "LAN access",
            EditTarget::Network(_) => "Network settings",
            EditTarget::Transfer(_) => "Transfer options",
            EditTarget::Tracker(_, old) if old.is_empty() => "Add tracker",
            EditTarget::Tracker(..) => "Edit tracker",
        };
        let controls = Palette {
            interactive: !editor.busy(),
            ..palette
        };
        div()
            .id("management-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .p_3()
            .child(super::motion::backdrop(palette.motion))
            .child(
                div()
                    .id("management-dialog")
                    .w(px(480.))
                    .max_w_full()
                    .max_h_full()
                    .overflow_y_scroll()
                    .rounded(px(24.))
                    .border_1()
                    .border_color(palette.outline)
                    .bg(palette.paper)
                    .shadow_lg()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.heading(title, 24.))
                    .when(matches!(editor.target, EditTarget::Web), |el| el.child(quiet(
                        "Use a certificate trusted by your browser and valid for the selected IP. Saving generates a new password.",
                        palette,
                    )))
                    .map(|el| {
                        let EditTarget::Network(selected) = editor.target else { return el; };
                        el.child(settings_row("Connection mode", palette).child(
                            div().flex().flex_wrap().gap_2().children(
                                [NetworkRoute::Direct, NetworkRoute::Socks5, NetworkRoute::Vpn]
                                    .into_iter().enumerate().map(|(index, route)| {
                                        choice(("network-route", index), route.label(), route == selected, controls)
                                            .when(controls.interactive, |el| el.on_click(cx.listener(move |view, _, _, cx| {
                                                if let Some(editor) = view.editor.as_mut().filter(|editor| !editor.busy()) {
                                                    editor.target = EditTarget::Network(route);
                                                    editor.error = None;
                                                    cx.notify();
                                                }
                                            })))
                                    }),
                            ),
                        ))
                        .child(quiet("Changing the route pauses transfers until restart.", palette))
                    })
                    .children(editor.fields.iter().enumerate().filter(|(i, _)| {
                        match editor.target {
                            EditTarget::Network(route) => route.shows_field(*i),
                            _ => true,
                        }
                    }).map(|(i, (label, field))| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_sm()
                            .child(*label)
                            .child(self.input(
                                ("management-input", i),
                                field,
                                palette,
                                editor.busy(),
                                window,
                                cx,
                            ))
                    }))
                    .when_some(editor.preview.as_ref(), |el, items| {
                        el.child(quiet(format!("{} matching {}", items.len(), if items.len() == 1 { "item" } else { "items" }), palette))
                          .children(items.iter().take(50).map(|item| quiet(item.title.clone(), palette)))
                          .when(items.len() > 50, |el| el.child(quiet("Showing the first 50 matches.", palette)))
                    })
                    .when_some(editor.error.clone(), |el, error| {
                        el.child(quiet(error, palette).text_color(palette.danger))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_end()
                            .gap_2()
                            .when(matches!(editor.target, EditTarget::Feed(_)), |el| el.child(
                                button("feed-preview", if editor.previewing { "Fetching…" } else { "Preview Matches" }, controls, false)
                                    .when(!editor.busy(), |el| el.on_click(cx.listener(|view, _, _, cx| view.preview_feed(cx))))
                            ))
                            .child(
                                button("management-cancel", "Cancel", controls, false)
                                    .when(!editor.busy(), |el| {
                                        el.on_click(cx.listener(Self::cancel_editor))
                                    }),
                            )
                            .child(
                                button(
                                    "management-save",
                                    if editor.saving { "Saving…" } else { "Save" },
                                    controls,
                                    true,
                                )
                                .when(!editor.busy(), |el| {
                                    el.on_click(cx.listener(|view, _, _, cx| view.save_editor(cx)))
                                }),
                            ),
                    )
                    .map(|dialog| super::motion::reveal_dialog(dialog, "editor-in", palette.motion)),
            )
    }

    fn cancel_editor(&mut self, _: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.close_editor(window, cx);
    }

    pub(super) fn transfer_options(&self, id: i64, palette: Palette, cx: &Context<Self>) -> Div {
        let position = self
            .management
            .queue
            .iter()
            .position(|&queued| queued == id);
        let mut summary = div().flex().flex_col().gap_1().mt_4();
        if let Some(options) = self.management.options.get(&id) {
            if !options.category.is_empty() {
                summary = summary.child(detail("Category", options.category.clone(), palette));
            }
            if !options.tags.is_empty() {
                summary = summary.child(detail("Tags", options.tags.clone(), palette));
            }
        }
        summary.child(
            div()
                .flex()
                .gap_1()
                .flex_wrap()
                .items_center()
                .child(
                    button(
                        ("edit-transfer", id as usize),
                        "Transfer Options…",
                        palette,
                        false,
                    )
                    .on_click(
                        cx.listener(move |view, _, window, cx| view.edit_transfer(id, window, cx)),
                    ),
                )
                .when_some(position, |el, position| {
                    el.child(quiet(format!("Queue #{}", position + 1), palette).text_xs())
                        .children(
                            [
                                (true, "Earlier", position > 0),
                                (false, "Later", position + 1 < self.management.queue.len()),
                            ]
                            .into_iter()
                            .filter(|&(_, _, enabled)| enabled)
                            .map(|(up, label, _)| {
                                button(("move-transfer", usize::from(up)), label, palette, false)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.send(Command::MoveInQueue(id, up), cx)
                                    }))
                            }),
                        )
                }),
        )
    }

    pub(super) fn tracker_actions(
        &self,
        id: i64,
        index: usize,
        tracker: &Tracker,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Div {
        let edit_url = tracker.url.clone();
        let remove_url = tracker.url.clone();
        let tier = tracker.tier;
        div()
            .flex()
            .gap_1()
            .flex_wrap()
            .child(
                quiet_button(("edit-tracker", index), "Edit…", palette).on_click(cx.listener(
                    move |view, _, window, cx| {
                        view.edit_tracker(id, Some((&edit_url, tier)), window, cx)
                    },
                )),
            )
            .child(
                quiet_button(("remove-tracker", index), "Remove", palette).on_click(cx.listener(
                    move |view, _, _, cx| {
                        view.send(
                            Command::Manage(ManagementEdit::Tracker(
                                id,
                                remove_url.clone(),
                                String::new(),
                                0,
                            )),
                            cx,
                        );
                    },
                )),
            )
    }

    pub(super) fn selection_toolbar(
        &self,
        rows: &[&Transfer],
        palette: Palette,
        cx: &Context<Self>,
    ) -> Div {
        let ids: Vec<_> = rows.iter().map(|row| row.id).collect();
        let selected: Vec<_> = ids
            .iter()
            .copied()
            .filter(|id| self.bulk_selection.contains(id))
            .collect();
        let all = !ids.is_empty() && selected.len() == ids.len();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .mb_2()
            .child(
                button(
                    "selection-mode",
                    if self.selecting { "Done" } else { "Select" },
                    palette,
                    false,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.selecting = !view.selecting;
                    view.bulk_selection.clear();
                    cx.notify();
                })),
            )
            .when(self.selecting, |el| {
                el.child(
                    button(
                        "select-visible",
                        if all {
                            "Clear visible"
                        } else {
                            "Select visible"
                        },
                        palette,
                        false,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        for &id in &ids {
                            if all {
                                view.bulk_selection.remove(&id);
                            } else {
                                view.bulk_selection.insert(id);
                            }
                        }
                        cx.notify();
                    })),
                )
                .child(quiet(format!("{} selected", selected.len()), palette).text_xs())
                .when(!selected.is_empty(), |el| {
                    el.children([(true, "Pause"), (false, "Resume")].map(|(paused, label)| {
                        let ids = selected.clone();
                        button(("bulk-pause", usize::from(paused)), label, palette, false).on_click(
                            cx.listener(move |view, _, _, cx| {
                                view.send(Command::PauseMany(ids.clone(), paused), cx)
                            }),
                        )
                    }))
                    .child(
                        button("bulk-remove", "Remove from Fero", palette, false)
                            .tooltip(make_tooltip("Keep downloaded files", palette))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.send(Command::RemoveMany(selected.clone()), cx)
                            })),
                    )
                })
            })
    }
}

fn whole_number(text: &str, label: &str) -> anyhow::Result<u32> {
    if text.is_empty() {
        return Ok(0);
    }
    text.parse()
        .map_err(|_| anyhow::anyhow!("{label} must be a nonnegative whole number"))
}

fn speed(text: &str) -> anyhow::Result<i32> {
    let value = if text.is_empty() {
        0.
    } else {
        text.parse::<f64>().unwrap_or(f64::NAN) * 1024.
    };
    anyhow::ensure!(
        (0. ..=f64::from(i32::MAX)).contains(&value),
        "Enter a nonnegative speed cap in KiB/s"
    );
    Ok(value.round() as i32)
}

pub(super) fn speed_text(value: i32) -> String {
    (f64::from(value) / 1024.).to_string()
}

fn ratio(text: &str) -> anyhow::Result<u32> {
    let value = if text.is_empty() {
        0.
    } else {
        text.parse::<f64>().unwrap_or(f64::NAN) * 100.
    };
    anyhow::ensure!(
        (0. ..=f64::from(u32::MAX)).contains(&value),
        "Enter a nonnegative seeding ratio"
    );
    Ok(value.round() as u32)
}

pub(super) fn ratio_text(value: u32) -> String {
    (f64::from(value) / 100.).to_string()
}

impl Editor {
    pub(super) fn busy(&self) -> bool {
        self.saving || self.previewing
    }

    fn subscription(&self, cx: &Context<Desktop>) -> anyhow::Result<fero::Subscription> {
        let value = |index: usize| self.fields[index].1.read(cx).text.trim().to_owned();
        fero::Subscription {
            url: value(0),
            include: value(1),
            exclude: value(2),
        }
        .validate()
    }
}
