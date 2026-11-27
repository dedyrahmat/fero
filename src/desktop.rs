use crate::{
    BackTab, Command, Dismiss, Event, Find, LiveDetails, LiveInspector, OpenAdd, OpenSettings,
    PasteMagnet, Submit, Tab, ToggleSidebar,
    icons::{BRAND, icon},
    is_torrent,
    preferences::{Accent, Preferences, Theme},
    text_field::TextField,
};
mod management;
mod motion;
mod settings;

use fero::{Management, PieceMap, Source, TorrentFile, Transfer};
use gpui::{
    Bounds, Context, Div, ElementId, Entity, ExternalPaths, FocusHandle, Font, FontFeatures,
    FontWeight, ListAlignment, ListOffset, ListState, MouseButton, PathBuilder, Pixels, Point,
    PromptButton, PromptLevel, Rgba, ScrollHandle, Stateful, Window, WindowAppearance, canvas,
    deferred, div, fill, font, img, point, prelude::*, px, relative, rgb, rgba, size,
};
use management::Editor;
use motion::{ProgressTrack, reveal};
use settings::SettingsSection;
use std::{
    borrow::Cow,
    cmp::Reverse,
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct Palette {
    paper: Rgba,
    rail: Rgba,
    chrome: Rgba,
    field: Rgba,
    ink: Rgba,
    quiet: Rgba,
    line: Rgba,
    outline: Rgba,
    focus_ring: Rgba,
    hover: Rgba,
    select: Rgba,
    accent: Rgba,
    on_accent: Rgba,
    download: Rgba,
    danger: Rgba,
    seed: Rgba,
    amber: Rgba,
    blue: Rgba,
    tile_colors: [(Rgba, Rgba); 4],
    track: Rgba,
    interactive: bool,
    motion: bool,
    keyboard_focus: bool,
}

impl Palette {
    fn new(appearance: WindowAppearance, theme: Theme, accent: Accent) -> Self {
        let dark = match theme {
            Theme::System => matches!(
                appearance,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            Theme::Light => false,
            Theme::Dark => true,
        };
        let color = |light, night| rgb(if dark { night } else { light });
        let harbor = accent == Accent::Harbor;
        let surface = |warm_light, warm_dark, harbor_light, harbor_dark| {
            if harbor {
                color(harbor_light, harbor_dark)
            } else {
                color(warm_light, warm_dark)
            }
        };
        let (accent, on_accent, select) = match accent {
            Accent::Harbor => (
                color(0x173d70, 0x7cede1),
                color(0xf1f8ff, 0x0b2a36),
                color(0xe1edf9, 0x20384c),
            ),
            Accent::Ember => (
                color(0xa33f27, 0xf99875),
                color(0xfff9f2, 0x301811),
                color(0xf6e9df, 0x352923),
            ),
            Accent::Sage => (
                color(0x39654d, 0xa4c8a9),
                color(0xf9fff7, 0x172a1e),
                color(0xe8efe5, 0x2c3930),
            ),
            Accent::Indigo => (
                color(0x514e89, 0xbab5ec),
                color(0xfaf9ff, 0x252143),
                color(0xedeaf5, 0x373147),
            ),
        };
        Self {
            paper: surface(0xfdfbf7, 0x201d1a, 0xf6faff, 0x101b2a),
            rail: surface(0xf3eee6, 0x25211e, 0xeaf2fb, 0x152337),
            chrome: surface(0xf9f4ec, 0x292421, 0xf0f6fd, 0x192a3f),
            field: surface(0xf7f2ea, 0x292522, 0xeef5fd, 0x192b40),
            ink: surface(0x29231f, 0xf5eee5, 0x122741, 0xf1f8ff),
            quiet: surface(0x6e6257, 0xb8a99c, 0x495f77, 0xa7bdd3),
            line: surface(0xe6ded3, 0x403731, 0xd6e3f0, 0x2b415a),
            outline: surface(0xd4c8bb, 0x51453c, 0xb3c8dd, 0x405873),
            focus_ring: surface(0x7f7368, 0xb1a396, 0x246483, 0x7cede1),
            hover: surface(0xf5eee5, 0x2d2723, 0xe3effa, 0x20344b),
            select,
            accent,
            on_accent,
            download: surface(0xa33f27, 0xf99875, 0x075963, 0x7cede1),
            danger: color(0x9c382f, 0xffa49a),
            seed: color(0x356951, 0xa4c8a9),
            amber: surface(0x805a29, 0xdfb587, 0x785414, 0xffe09f),
            blue: color(0x4e6179, 0xb0bfd6),
            tile_colors: [
                (color(0xf6e5d9, 0x442d24), color(0xa33f27, 0xf99875)),
                (color(0xe7efe4, 0x303b31), color(0x356951, 0xa4c8a9)),
                (color(0xf0e8d7, 0x3e3226), color(0x805a29, 0xdfb587)),
                (color(0xe8ecf0, 0x30323b), color(0x4e6179, 0xb0bfd6)),
            ],
            track: surface(0xe9e1d6, 0x413630, 0xd9e6f4, 0x2a3f58),
            interactive: true,
            motion: true,
            keyboard_focus: false,
        }
    }
}

const FILTERS: [&str; 9] = [
    "All transfers",
    "Downloading",
    "Seeding",
    "Paused",
    "Waiting for peers",
    "Fetching metadata",
    "Error",
    "Queued",
    "Ratio reached",
];
const FILTER_ICONS: [&str; FILTERS.len()] = [
    "list",
    "arrow-down",
    "arrow-up",
    "pause",
    "clock-circle",
    "magnet",
    "danger-circle",
    "clock-circle",
    "stop",
];
const FILE_PAGE_SIZE: usize = 80;
const VIRTUAL_ROW_THRESHOLD: usize = 40;
const RATE_SAMPLES: usize = 16;
const ROW_ACTIONS_WIDTH: f32 = 148.;
const CARD_RADIUS: f32 = 24.;

#[derive(Clone, Copy)]
struct TransferColumns {
    tile: f32,
    name: f32,
    gap: f32,
    inset: f32,
    progress: f32,
    rate: f32,
    stacked: bool,
    inline_actions: bool,
}

impl TransferColumns {
    fn new(list_width: gpui::Pixels, wide: bool) -> Self {
        let width = f32::from(list_width);
        let inline_actions = width >= 600.;
        let stacked = width < 480.;
        let (tile, gap, inset, rate, minimum, ratio) = if stacked {
            (30., 8., 5., 74., 0., 0.)
        } else if wide {
            (40., 12., 10., 93., 86., 0.43)
        } else {
            (35., 8., 10., 82., 68., 0.35)
        };
        let actions = if inline_actions {
            ROW_ACTIONS_WIDTH + gap
        } else {
            0.
        };
        let remaining =
            width - tile - rate - actions - gap * if stacked { 2. } else { 3. } - inset * 2. - 2.;
        let progress = (remaining * ratio / (1. + ratio)).max(minimum);
        Self {
            tile,
            name: (remaining - progress).max(0.),
            gap,
            inset,
            progress,
            rate,
            stacked,
            inline_actions,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Recent,
    Name,
    Progress,
    Rate,
    Queue,
}

impl SortBy {
    const OPTIONS: [(Self, &'static str); 5] = [
        (Self::Recent, "Recently added"),
        (Self::Name, "Name"),
        (Self::Progress, "Progress"),
        (Self::Rate, "Transfer rate"),
        (Self::Queue, "Queue position"),
    ];
}

#[derive(Clone, Copy, PartialEq)]
enum AddMode {
    Magnet,
    File,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InspectorTab {
    Overview,
    Files,
    Peers,
    Trackers,
}

#[derive(Clone, Copy)]
enum ResizablePane {
    Sidebar,
    InspectorWidth,
    InspectorHeight,
}

#[derive(Clone, Copy, PartialEq)]
enum Picker {
    Torrent,
    Destination,
    DefaultFolder,
    WatchFolders,
    QbitProfile,
    MigrationDestination,
}

struct Draft {
    revision: u64,
    mode: AddMode,
    file: Option<PathBuf>,
    start_paused: bool,
    start_immediately: bool,
    requested_magnet: Option<String>,
    previewing: bool,
    submitting: bool,
    files: Vec<TorrentFile>,
    skipped: BTreeSet<u32>,
    reviewed: Option<u32>,
    file_limit: usize,
    error: Option<String>,
}

pub struct Desktop {
    commands: Sender<Command>,
    transfers: Vec<Transfer>,
    management: Arc<Management>,
    management_loaded: bool,
    editor: Option<Editor>,
    selecting: bool,
    bulk_selection: HashSet<i64>,
    download_history: HashMap<i64, VecDeque<i32>>,
    traffic_history: [VecDeque<i64>; 2],
    traffic_open: bool,
    last_rate_sample: Option<Instant>,
    selected: Option<i64>,
    filter: usize,
    sort: SortBy,
    sort_open: bool,
    search: Entity<TextField>,
    query: String,
    magnet: Entity<TextField>,
    destination: Entity<TextField>,
    file_search: Entity<TextField>,
    file_scroll: ScrollHandle,
    focus: FocusHandle,
    keyboard_focus: bool,
    rows_focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    list_scroll: ScrollHandle,
    virtual_list: ListState,
    narrow_scroll: ScrollHandle,
    draft: Option<Draft>,
    open_queue: VecDeque<Source>,
    preferences: Preferences,
    settings: bool,
    settings_section: Option<SettingsSection>,
    settings_scroll: ScrollHandle,
    about_notices_open: bool,
    default_folder_field: Entity<TextField>,
    folder_error: Option<String>,
    folder_saving: bool,
    starts_at_login: Option<bool>,
    web_settings: Option<crate::web::WebSettings>,
    web_running: bool,
    web_password: Option<String>,
    migration: Option<fero::MigrationPreview>,
    migration_destinations: HashMap<usize, Entity<TextField>>,
    migration_selected: HashSet<usize>,
    migration_invalid: HashSet<usize>,
    migration_importing: bool,
    migration_picker: Option<(usize, Entity<TextField>)>,
    inspector_tab: InspectorTab,
    inspector_open: bool,
    inspector_scroll: ScrollHandle,
    sidebar_width: Option<Pixels>,
    inspector_width: Option<Pixels>,
    inspector_height: Option<Pixels>,
    resize_drag: Option<(ResizablePane, Pixels, Point<Pixels>)>,
    inspected_id: Option<i64>,
    shown_inspector_id: Option<i64>,
    inspected_files: Option<Result<Vec<TorrentFile>, String>>,
    live_inspector: Option<LiveInspector>,
    live_details: Option<LiveDetails>,
    inspector_item_limit: usize,
    revision: u64,
    error: Option<String>,
    body_font: Font,
    heading_font: Font,
}

impl Desktop {
    pub fn new(
        commands: Sender<Command>,
        events: Rc<Receiver<Event>>,
        opened: Rc<Receiver<Source>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = cx.text_system();
        // GPUI matches named faces, so use fixed weights from the mockup's variable fonts.
        let body = text
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../design/fonts/DM-Sans-Regular.ttf")),
                Cow::Borrowed(include_bytes!("../design/fonts/DM-Sans-Bold.ttf")),
            ])
            .is_ok();
        let heading = text
            .add_fonts(vec![Cow::Borrowed(include_bytes!(
                "../design/fonts/Instrument-Sans-Semibold.ttf"
            ))])
            .is_ok();
        let mut body_font = font(if body {
            "Fero DM Sans"
        } else {
            ".SystemUIFont"
        });
        body_font.features = FontFeatures(Arc::new(vec![("tnum".into(), 1)]));
        let heading_font = if heading {
            font("Fero Instrument Sans")
        } else {
            body_font.clone()
        };

        let search = cx.new(|cx| TextField::new("Search name, category, or tag", false, cx));
        let magnet = cx.new(|cx| TextField::new("Paste a magnet link", true, cx));
        let destination = cx.new(|cx| TextField::new("Enter a download folder", false, cx));
        let default_folder_field =
            cx.new(|cx| TextField::new("Enter a download folder", false, cx));
        let file_search = cx.new(|cx| TextField::new("Find a file", false, cx));
        cx.observe(&search, |view, field, cx| {
            let query = field.read(cx).text.trim().to_lowercase();
            if view.query != query {
                view.query = query;
                view.reset_list_scroll();
                view.sync_inspector();
                cx.notify();
            }
        })
        .detach();
        cx.observe(&destination, |_, _, cx| cx.notify()).detach();
        cx.observe(&default_folder_field, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&file_search, |view, _, cx| {
            if let Some(draft) = &mut view.draft {
                draft.file_limit = FILE_PAGE_SIZE;
            }
            view.file_scroll.set_offset(gpui::Point::default());
            cx.notify();
        })
        .detach();
        cx.observe(&magnet, |view, field, cx| {
            if view
                .draft
                .as_ref()
                .and_then(|draft| draft.requested_magnet.as_deref())
                .is_some_and(|start| start != field.read(cx).text.trim())
                && view.draft.as_ref().is_some_and(|draft| !draft.submitting)
            {
                view.reset_preview();
            }
            cx.notify();
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(125))
                    .await;
                if view
                    .update_in(cx, |view, window, cx| {
                        let mut changed = false;
                        for event in events.try_iter() {
                            view.on_event(event, window, cx);
                            changed = true;
                        }
                        view.open_queue.extend(opened.try_iter());
                        if view.draft.is_none()
                            && view.editor.is_none()
                            && let Some(source) = view.open_queue.pop_front()
                        {
                            view.begin_add(Some(source), window, cx);
                            window.activate_window();
                            changed = true;
                        }
                        if changed {
                            view.sync_inspector();
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus);
        Self {
            commands,
            search,
            query: String::new(),
            magnet,
            destination,
            file_search,
            file_scroll: ScrollHandle::new(),
            focus,
            keyboard_focus: false,
            rows_focus: cx.focus_handle().tab_stop(true),
            transfers: Vec::new(),
            management: Arc::default(),
            management_loaded: false,
            editor: None,
            selecting: false,
            bulk_selection: HashSet::new(),
            download_history: HashMap::new(),
            traffic_history: Default::default(),
            traffic_open: false,
            last_rate_sample: None,
            selected: None,
            filter: 0,
            sort: SortBy::Recent,
            sort_open: false,
            draft: None,
            previous_focus: None,
            list_scroll: ScrollHandle::new(),
            virtual_list: ListState::new(0, ListAlignment::Top, px(250.)),
            narrow_scroll: ScrollHandle::new(),
            preferences: Preferences::default(),
            settings: false,
            settings_section: None,
            settings_scroll: ScrollHandle::new(),
            about_notices_open: false,
            default_folder_field,
            folder_error: None,
            folder_saving: false,
            starts_at_login: None,
            web_settings: None,
            web_running: false,
            web_password: None,
            migration: None,
            migration_destinations: HashMap::new(),
            migration_selected: HashSet::new(),
            migration_invalid: HashSet::new(),
            migration_importing: false,
            migration_picker: None,
            inspector_tab: InspectorTab::Overview,
            inspector_open: true,
            inspector_scroll: ScrollHandle::new(),
            sidebar_width: None,
            inspector_width: None,
            inspector_height: None,
            resize_drag: None,
            inspected_id: None,
            shown_inspector_id: None,
            inspected_files: None,
            live_inspector: None,
            live_details: None,
            inspector_item_limit: FILE_PAGE_SIZE,
            revision: 0,
            error: None,
            body_font,
            heading_font,
            open_queue: VecDeque::new(),
        }
    }

    fn reset_preview(&mut self) {
        if let Some(draft) = self.draft.as_mut() {
            self.revision += 1;
            draft.revision = self.revision;
            draft.requested_magnet = None;
            draft.previewing = false;
            draft.files.clear();
            draft.skipped.clear();
            draft.reviewed = None;
            draft.error = None;
            let _ = self.commands.send(Command::CancelPreview);
        }
    }

    fn begin_add(&mut self, source: Option<Source>, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            if let Some(source) = source {
                self.open_queue.push_back(source);
            }
            return;
        }
        if self.draft.is_some() {
            if let Some(source) = source {
                self.open_queue.push_back(source);
            }
            return;
        }
        self.previous_focus = window.focused(cx);
        self.sort_open = false;
        self.revision += 1;
        self.draft = Some(Draft {
            revision: self.revision,
            mode: AddMode::Magnet,
            file: None,
            start_paused: self.preferences.start_paused,
            start_immediately: self.preferences.start_immediately,
            requested_magnet: None,
            previewing: false,
            submitting: false,
            files: Vec::new(),
            skipped: BTreeSet::new(),
            reviewed: None,
            file_limit: FILE_PAGE_SIZE,
            error: None,
        });
        let text = match &source {
            Some(Source::Magnet(link)) => link.clone(),
            _ => String::new(),
        };
        self.magnet.update(cx, |field, cx| field.set(text, cx));
        let path = self.preferences.download_folder();
        self.set_destination(path.as_deref(), cx);
        self.clear_file_search(cx);
        if let Some(Source::File(path)) = source {
            self.set_file(path);
            window.focus(&self.focus);
        } else {
            window.focus(&self.magnet.read(cx).focus);
        }
        cx.notify();
    }

    fn close_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft = None;
        let _ = self.commands.send(Command::CancelPreview);
        window.focus(self.previous_focus.take().as_ref().unwrap_or(&self.focus));
        cx.notify();
    }

    fn on_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Preferences(preferences) => {
                self.preferences = preferences;
                if self.settings {
                    self.fill_default_folder(cx);
                }
            }
            Event::DefaultFolder(result) => {
                self.folder_saving = false;
                match result {
                    Ok(folder) => {
                        self.preferences.default_folder = folder;
                        self.folder_error = None;
                        self.fill_default_folder(cx);
                    }
                    Err(error) => self.folder_error = Some(error),
                }
            }
            Event::Files(id, result) if self.inspected_id == Some(id) => {
                self.inspected_files = Some(result);
            }
            Event::Files(..) => {}
            Event::Transfers(transfers, details, management) => {
                if self
                    .selected
                    .is_some_and(|id| !transfers.iter().any(|row| row.id == id))
                {
                    self.selected = None;
                }
                self.sample_rates(&transfers);
                if !self.bulk_selection.is_empty() {
                    let ids: HashSet<_> = transfers.iter().map(|row| row.id).collect();
                    self.bulk_selection.retain(|id| ids.contains(id));
                }
                self.transfers = transfers;
                self.management = management;
                self.management_loaded = true;
                self.live_details = details;
            }
            Event::ManagementSaved(result) => match result {
                Ok(()) if self.editor.as_ref().is_some_and(|editor| editor.saving) => {
                    self.close_editor(window, cx)
                }
                Err(error) => {
                    if let Some(editor) = self.editor.as_mut().filter(|editor| editor.saving) {
                        editor.saving = false;
                        editor.error = Some(error);
                    } else {
                        self.error = Some(error);
                    }
                }
                Ok(()) => {}
            },
            Event::StartAtLogin(enabled) => self.starts_at_login = Some(enabled),
            Event::WebState(settings, running) => {
                self.web_settings = settings;
                self.web_running = running;
            }
            Event::WebPassword(password) => self.web_password = password,
            Event::FeedPreview(result) => self.feed_preview(result),
            Event::MigrationPreview(result) => match result {
                Ok(preview) => {
                    self.migration_invalid.clear();
                    self.migration_destinations = preview
                        .items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            let field = cx.new(|cx| TextField::new("Save folder", false, cx));
                            field.update(cx, |field, cx| {
                                field.set(item.destination.to_string_lossy().into_owned(), cx)
                            });
                            (index, field)
                        })
                        .collect();
                    self.migration_selected = preview
                        .items
                        .iter()
                        .enumerate()
                        .filter_map(|(index, item)| {
                            (!item.duplicate && !item.bytes.is_empty()).then_some(index)
                        })
                        .collect();
                    self.migration = Some(preview);
                }
                Err(error) => self.error = Some(error),
            },
            Event::MigrationInvalidDestinations(rows) => {
                self.migration_importing = false;
                self.migration_invalid = rows.into_iter().collect();
            }
            Event::MigrationImported(result) => {
                self.migration_importing = false;
                match result {
                    Ok(count) => {
                        self.migration = None;
                        self.migration_destinations.clear();
                        self.error = Some(format!(
                            "Imported {count} transfers paused; files are being checked."
                        ));
                    }
                    Err(error) => {
                        self.migration = None;
                        self.migration_destinations.clear();
                        self.error = Some(error);
                    }
                }
            }
            Event::Error(error) => self.error = Some(error),
            Event::Prepared(revision, result) => {
                if let Some(draft) = &mut self.draft
                    && draft.revision == revision
                {
                    draft.previewing = false;
                    match result {
                        Ok(files) => {
                            draft.reviewed = files.first().map(|file| file.index);
                            if files.is_empty() {
                                draft.error =
                                    Some("This torrent contains no downloadable files".into());
                            }
                            draft.files = files;
                        }
                        Err(error) => draft.error = Some(error),
                    }
                }
            }
            Event::Added(revision, result) => {
                if let Some(draft) = &mut self.draft
                    && draft.revision == revision
                {
                    draft.submitting = false;
                    match result {
                        Ok(id) => {
                            self.settings = false;
                            self.inspector_tab = InspectorTab::Overview;
                            self.inspector_open = true;
                            self.inspector_scroll.set_offset(gpui::Point::default());
                            self.selected = Some(id);
                            self.filter = 0;
                            self.search
                                .update(cx, |field, cx| field.set(String::new(), cx));
                            self.close_add(window, cx);
                            // The worker publishes the new row immediately after this event.
                            window.focus(&self.rows_focus);
                        }
                        Err(error) => draft.error = Some(error),
                    }
                }
            }
        }
    }

    fn set_mode(&mut self, mode: AddMode, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = &self.draft
            && draft.mode != mode
            && !draft.submitting
        {
            self.reset_preview();
            self.clear_file_search(cx);
            self.draft.as_mut().unwrap().mode = mode;
            if mode == AddMode::Magnet {
                window.focus(&self.magnet.read(cx).focus);
            } else if let Some(path) = self.draft.as_ref().and_then(|draft| draft.file.clone()) {
                self.prepare(Source::File(path));
            }
            cx.notify();
        }
    }

    fn set_file(&mut self, path: PathBuf) {
        self.reset_preview();
        let draft = self.draft.as_mut().unwrap();
        draft.mode = AddMode::File;
        draft.file = Some(path.clone());
        self.prepare(Source::File(path));
    }

    fn pick_path(&mut self, target: Picker, window: &mut Window, cx: &mut Context<Self>) {
        let revision = self.draft.as_ref().map(|draft| draft.revision);
        if self.draft.as_ref().is_some_and(|draft| draft.submitting) {
            return;
        }
        let directory = target != Picker::Torrent;
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: target == Picker::WatchFolders,
            prompt: Some(
                if target == Picker::WatchFolders {
                    "Choose Folders to Watch"
                } else if target == Picker::QbitProfile {
                    "Choose qBittorrent Profile"
                } else if target == Picker::MigrationDestination {
                    "Choose Existing Save Folder"
                } else if directory {
                    "Choose Folder"
                } else {
                    "Choose Torrent"
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = picker
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = view.update(cx, |view, cx| {
                if target == Picker::QbitProfile {
                    if let Ok(Some(path)) =
                        result.map(|paths| paths.and_then(|paths| paths.into_iter().next()))
                    {
                        view.send(Command::PreviewMigration(path), cx);
                    }
                    return;
                }
                if target == Picker::MigrationDestination {
                    if let Some((index, field)) = view.migration_picker.take()
                        && let Ok(Some(path)) =
                            result.map(|paths| paths.and_then(|paths| paths.into_iter().next()))
                    {
                        field.update(cx, |field, cx| {
                            field.set(path.to_string_lossy().into_owned(), cx)
                        });
                        view.migration_invalid.remove(&index);
                        cx.notify();
                    }
                    return;
                }
                if target == Picker::WatchFolders {
                    match result {
                        Ok(Some(paths)) if !paths.is_empty() => {
                            view.send(Command::SaveWatchFolders(paths), cx)
                        }
                        Err(error) => {
                            view.error = Some(error);
                            cx.notify();
                        }
                        _ => {}
                    }
                    return;
                }
                let result = result.map(|paths| paths.and_then(|paths| paths.into_iter().next()));
                if target == Picker::DefaultFolder {
                    match result {
                        Ok(Some(path)) => view.save_default_folder(Some(path), cx),
                        Ok(None) => return,
                        Err(error) => view.folder_error = Some(error),
                    }
                    cx.notify();
                    return;
                }
                let Some(draft) = &mut view.draft else {
                    return;
                };
                if Some(draft.revision) != revision || draft.submitting {
                    return;
                }
                match result {
                    Ok(Some(path)) if directory && path.to_str().is_some() => {
                        draft.error = None;
                        view.set_destination(Some(&path), cx);
                    }
                    Ok(Some(_)) if directory => {
                        draft.error = Some("Download folder must be valid UTF-8".into());
                    }
                    Ok(Some(path)) if is_torrent(&path) => {
                        view.clear_file_search(cx);
                        view.set_file(path);
                    }
                    Ok(Some(_)) => draft.error = Some("Choose a .torrent file".into()),
                    Ok(None) => {}
                    Err(error) => draft.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn prepare(&mut self, source: Source) {
        let draft = self.draft.as_mut().unwrap();
        draft.requested_magnet = match &source {
            Source::Magnet(link) => Some(link.clone()),
            Source::File(_) => None,
        };
        draft.error = None;
        draft.previewing = self
            .commands
            .send(Command::Prepare(draft.revision, source))
            .is_ok();
        if !draft.previewing {
            draft.error = Some("Transfer engine is unavailable".into());
        }
    }

    fn preview_magnet(&mut self, cx: &mut Context<Self>) {
        let link = self.magnet.read(cx).text.trim().to_owned();
        let Some(draft) = &mut self.draft else {
            return;
        };
        if draft.submitting {
            return;
        }
        if link.starts_with("magnet:?") {
            self.reset_preview();
            self.clear_file_search(cx);
            self.prepare(Source::Magnet(link));
        } else {
            draft.error = Some("Enter a magnet link beginning with magnet:?".into());
        }
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let destination = self.entered_destination(cx);
        let Some(draft) = &mut self.draft else {
            return;
        };
        if draft.submitting || draft.previewing {
            return;
        }
        if draft.mode == AddMode::Magnet && !draft.start_immediately && draft.files.is_empty() {
            self.preview_magnet(cx);
            return;
        }
        let link = self.magnet.read(cx).text.trim();
        let use_magnet = draft.mode == AddMode::Magnet && draft.start_immediately;
        if use_magnet && !link.starts_with("magnet:?") {
            draft.error = Some("Enter a magnet link beginning with magnet:?".into());
        } else if !use_magnet && draft.files.is_empty() {
            draft.error = Some(
                if draft.mode == AddMode::File {
                    "Choose a valid torrent file"
                } else {
                    "Get the file list again or choose Start immediately"
                }
                .into(),
            );
        } else if !draft.files.is_empty() && draft.files.len() == draft.skipped.len() {
            draft.error = Some("Select at least one file".into());
        } else if let Some(destination) = destination {
            draft.error = None;
            draft.submitting = self
                .commands
                .send(Command::Add {
                    revision: draft.revision,
                    magnet: use_magnet.then(|| link.to_owned()),
                    destination,
                    start_paused: draft.start_paused,
                    skipped: draft.skipped.iter().copied().collect(),
                })
                .is_ok();
            if !draft.submitting {
                draft.error = Some("Transfer engine is unavailable".into());
            }
        } else {
            draft.error = Some("Choose a download folder or enter an absolute path".into());
        }
        cx.notify();
    }

    fn visible_rows(&self) -> Vec<&Transfer> {
        let mut rows: Vec<_> = self
            .transfers
            .iter()
            .filter(|row| {
                (self.filter == 0 || row.state == FILTERS[self.filter]) && self.matches_query(row)
            })
            .collect();
        match self.sort {
            SortBy::Recent => rows.sort_unstable_by_key(|row| Reverse(row.id)),
            SortBy::Name => {
                rows.sort_by_cached_key(|row| (row.name.to_lowercase(), Reverse(row.id)))
            }
            SortBy::Progress => rows.sort_unstable_by(|a, b| {
                b.progress
                    .total_cmp(&a.progress)
                    .then_with(|| b.id.cmp(&a.id))
            }),
            SortBy::Rate => rows.sort_unstable_by_key(|row| {
                (
                    Reverse(i64::from(row.download_rate) + i64::from(row.upload_rate)),
                    Reverse(row.id),
                )
            }),
            SortBy::Queue => {}
        }
        rows
    }

    fn matches_query(&self, row: &Transfer) -> bool {
        let options = self.management.options.get(&row.id);
        if let Some(category) = self.query.strip_prefix("category:") {
            options.is_some_and(|o| o.category.to_lowercase() == category.trim())
        } else if let Some(tag) = self.query.strip_prefix("tag:") {
            options.is_some_and(|o| {
                o.tags
                    .split(", ")
                    .any(|name| name.to_lowercase() == tag.trim())
            })
        } else {
            self.query.is_empty()
                || row.name.to_lowercase().contains(&self.query)
                || options.is_some_and(|o| {
                    o.category.to_lowercase().contains(&self.query)
                        || o.tags.to_lowercase().contains(&self.query)
                })
        }
    }

    fn sample_rates(&mut self, transfers: &[Transfer]) {
        let now = Instant::now();
        if self
            .last_rate_sample
            .is_some_and(|last| now - last < Duration::from_secs(4))
        {
            return;
        }
        self.last_rate_sample = Some(now);
        for (history, rate) in self
            .traffic_history
            .iter_mut()
            .zip(traffic_rates(transfers))
        {
            if history.len() == RATE_SAMPLES {
                history.pop_front();
            }
            history.push_back(rate);
        }
        let mut previous = std::mem::take(&mut self.download_history);
        for row in transfers {
            let mut history = previous.remove(&row.id).unwrap_or_default();
            if history.len() == RATE_SAMPLES {
                history.pop_front();
            }
            history.push_back(row.download_rate);
            self.download_history.insert(row.id, history);
        }
    }

    fn reset_list_scroll(&self) {
        self.list_scroll.set_offset(gpui::Point::default());
        self.narrow_scroll.set_offset(gpui::Point::default());
        self.virtual_list.scroll_to(ListOffset::default());
    }

    fn select_relative(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.visible_rows();
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|row| Some(row.id) == self.selected)
            .unwrap_or(0);
        let next = if down {
            (index + 1).min(rows.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        let virtualized = rows.len() > VIRTUAL_ROW_THRESHOLD;
        self.selected = Some(rows[next].id);
        self.sync_inspector();
        if virtualized {
            self.virtual_list.scroll_to_reveal_item(next);
        } else if window.viewport_size().width < px(732.) && !self.preferences.collapsed {
            if let Some(row) = self.list_scroll.bounds_for_item(next) {
                let viewport = self.narrow_scroll.bounds();
                let mut offset = self.narrow_scroll.offset();
                if row.top() < viewport.top() {
                    offset.y += viewport.top() - row.top();
                } else if row.bottom() > viewport.bottom() {
                    offset.y -= row.bottom() - viewport.bottom();
                }
                self.narrow_scroll.set_offset(offset);
            }
        } else {
            self.list_scroll.scroll_to_item(next);
        }
        window.focus(&self.rows_focus);
        cx.notify();
    }

    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        self.error = self
            .commands
            .send(command)
            .err()
            .map(|_| "Transfer engine is unavailable".into());
        cx.notify();
    }

    fn open_transfer(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected != Some(id) || !self.inspector_open {
            self.inspector_scroll.set_offset(Point::default());
        }
        self.selected = Some(id);
        self.inspector_open = true;
        if window.viewport_size().width < px(732.) {
            self.traffic_open = false;
        }
        self.sync_inspector();
        window.focus(&self.rows_focus);
        cx.notify();
    }

    fn close_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inspector_open = false;
        self.sync_inspector();
        if self.visible_rows().is_empty() {
            window.focus(&self.focus);
        } else {
            window.focus(&self.rows_focus);
        }
        cx.notify();
    }

    fn set_keyboard_focus(&mut self, keyboard: bool, cx: &mut Context<Self>) {
        if self.keyboard_focus != keyboard {
            self.keyboard_focus = keyboard;
            cx.notify();
        }
    }

    fn set_pane_size(&mut self, pane: ResizablePane, value: Pixels, cx: &mut Context<Self>) {
        let (current, minimum, maximum) = match pane {
            ResizablePane::Sidebar => (&mut self.sidebar_width, 157., 320.),
            ResizablePane::InspectorWidth => (&mut self.inspector_width, 245., 500.),
            ResizablePane::InspectorHeight => (&mut self.inspector_height, 148., 650.),
        };
        let value = Some(value.clamp(px(minimum), px(maximum)));
        if *current != value {
            *current = value;
            cx.notify();
        }
    }

    fn resize_grip(
        &self,
        pane: ResizablePane,
        value: Pixels,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let label = match pane {
            ResizablePane::Sidebar => "Resize sidebar",
            _ => "Resize transfer details",
        };
        let grip = div()
            .id(("resize-pane", pane as usize))
            .absolute()
            .flex()
            .items_center()
            .justify_center()
            .hover(move |el| el.bg(palette.select))
            .when(palette.keyboard_focus, |el| {
                el.focus(move |el| el.bg(palette.hover))
            })
            .tab_index(0)
            .tooltip(make_tooltip(label, palette))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                    view.resize_drag = Some((pane, value, event.position));
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(move |view, event: &gpui::KeyDownEvent, _, cx| {
                let delta = match (pane, event.keystroke.key.as_str()) {
                    (ResizablePane::Sidebar, "right")
                    | (ResizablePane::InspectorWidth, "left")
                    | (ResizablePane::InspectorHeight, "up") => px(16.),
                    (ResizablePane::Sidebar, "left")
                    | (ResizablePane::InspectorWidth, "right")
                    | (ResizablePane::InspectorHeight, "down") => px(-16.),
                    _ => return,
                };
                view.set_pane_size(pane, value + delta, cx);
                cx.stop_propagation();
            }));
        if matches!(pane, ResizablePane::InspectorHeight) {
            grip.top_0()
                .left_0()
                .right_0()
                .h(px(8.))
                .cursor_ns_resize()
                .child(
                    div()
                        .w(px(28.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(palette.outline),
                )
        } else {
            grip.top_0()
                .bottom_0()
                .w(px(8.))
                .cursor_ew_resize()
                .when(matches!(pane, ResizablePane::Sidebar), |el| el.right_0())
                .when(matches!(pane, ResizablePane::InspectorWidth), |el| {
                    el.left_0()
                })
                .child(
                    div()
                        .w(px(2.))
                        .h(px(28.))
                        .rounded_full()
                        .bg(palette.outline),
                )
        }
    }

    fn delete_files(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.transfers.iter().find(|row| row.id == id) else {
            return;
        };
        let detail = format!(
            "Delete all files and partial downloads belonging to “{}” in {}? This cannot be undone.",
            row.name, row.destination
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete downloaded files?",
            Some(&detail),
            &[
                PromptButton::cancel("Cancel"),
                PromptButton::new("Delete Files"),
            ],
            cx,
        );
        cx.spawn_in(window, async move |view, cx| {
            if answer.await == Ok(1) {
                let _ = view.update(cx, |view, cx| view.send(Command::Remove(id, true), cx));
            }
        })
        .detach();
    }

    fn dropped(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        let mut torrents = paths
            .paths()
            .iter()
            .filter(|path| is_torrent(path))
            .cloned();
        if let Some(path) = torrents.next() {
            if self.draft.as_ref().is_some_and(|draft| !draft.submitting) {
                self.clear_file_search(cx);
                self.set_file(path);
                window.focus(&self.focus);
            } else {
                self.begin_add(Some(Source::File(path)), window, cx);
            }
            self.open_queue.extend(torrents.map(Source::File));
        } else if let Some(draft) = &mut self.draft {
            draft.error = Some("Drop a .torrent file".into());
        } else {
            self.error = Some("Drop a .torrent file".into());
        }
        cx.notify();
    }

    fn set_destination(&self, path: Option<&Path>, cx: &mut Context<Self>) {
        let text = path.and_then(Path::to_str).unwrap_or_default().to_owned();
        self.destination.update(cx, |field, cx| field.set(text, cx));
    }

    fn entered_destination(&self, cx: &Context<Self>) -> Option<PathBuf> {
        absolute_folder(&self.destination.read(cx).text)
    }

    fn clear_file_search(&self, cx: &mut Context<Self>) {
        self.file_search
            .update(cx, |field, cx| field.set(String::new(), cx));
    }

    fn save_preferences(&mut self, cx: &mut Context<Self>) {
        self.send(Command::SavePreferences(self.preferences.clone()), cx);
    }

    fn fill_default_folder(&self, cx: &mut Context<Self>) {
        let path = self.preferences.download_folder();
        let text = path
            .as_deref()
            .and_then(Path::to_str)
            .unwrap_or_default()
            .to_owned();
        self.default_folder_field
            .update(cx, |field, cx| field.set(text, cx));
    }

    fn save_default_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        if !self.folder_saving {
            self.folder_error = None;
            self.folder_saving = self
                .commands
                .send(Command::SaveDefaultFolder(folder))
                .is_ok();
            if !self.folder_saving {
                self.folder_error = Some("Settings storage is unavailable".into());
            }
            cx.notify();
        }
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.draft.is_none() && self.editor.is_none() {
            self.preferences.collapsed = !self.preferences.collapsed;
            self.save_preferences(cx);
        }
    }

    fn show_settings(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.is_some() || self.editor.is_some() {
            return;
        }
        self.settings = open;
        self.sort_open = false;
        self.sync_inspector();
        if open {
            self.settings_section = None;
            self.settings_scroll.set_offset(Point::default());
            self.folder_error = None;
            self.fill_default_folder(cx);
            window.focus(&self.focus);
        } else {
            window.focus(&self.search.read(cx).focus);
        }
        cx.notify();
    }

    fn inspected_row<'a>(&self, filter_rows: &[&'a Transfer]) -> Option<&'a Transfer> {
        filter_rows
            .iter()
            .find(|row| Some(row.id) == self.selected)
            .or_else(|| filter_rows.first())
            .copied()
    }

    fn sync_inspector(&mut self) {
        let rows = self.visible_rows();
        let row = self
            .inspected_row(&rows)
            .filter(|_| self.inspector_open && !self.settings);
        let shown = row.map(|row| row.id);
        let id = row
            .filter(|_| self.inspector_tab == InspectorTab::Files)
            .map(|row| row.id);
        let live = row.and_then(|row| match self.inspector_tab {
            InspectorTab::Overview
                if matches!(row.state.as_str(), "Downloading" | "Waiting for peers") =>
            {
                Some(LiveInspector::Pieces(row.id))
            }
            InspectorTab::Peers => Some(LiveInspector::Peers(row.id)),
            InspectorTab::Trackers => Some(LiveInspector::Trackers(row.id)),
            _ => None,
        });
        if self.shown_inspector_id != shown {
            self.shown_inspector_id = shown;
            self.inspector_scroll.set_offset(Point::default());
        }
        if self.inspected_id != id {
            self.inspected_id = id;
            self.inspected_files = None;
            self.inspector_item_limit = FILE_PAGE_SIZE;
            let _ = self.commands.send(Command::InspectFiles(id));
        }
        if self.live_inspector != live {
            self.live_inspector = live;
            self.live_details = None;
            let _ = self.commands.send(Command::InspectLive(live));
        }
    }

    fn sidebar(
        &self,
        palette: Palette,
        width: gpui::Pixels,
        pane_width: Pixels,
        wrapped: bool,
        cx: &Context<Self>,
    ) -> Div {
        let collapsed = self.preferences.collapsed;
        let mut counts = [0; FILTERS.len()];
        counts[0] = self.transfers.len();
        for transfer in &self.transfers {
            if let Some(index) = FILTERS.iter().position(|state| *state == transfer.state) {
                counts[index] += 1;
            }
        }
        let toggle = nav_icon(
            "toggle-sidebar",
            if collapsed {
                "panel-left-open"
            } else {
                "panel-left-close"
            },
            if collapsed {
                "Expand sidebar"
            } else {
                "Collapse sidebar for Focus mode"
            },
            palette,
        )
        .bg(palette.field)
        .border_color(palette.line)
        .when(collapsed, |el| el.bg(palette.select))
        .on_click(cx.listener(|view, _, _, cx| view.toggle_sidebar(cx)));
        let brand = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .when(collapsed, |el| el.flex_col())
            .when(!wrapped, |el| el.mb(px(24.)))
            .child(img(BRAND).size(px(34.)).flex_none())
            .when(!collapsed, |el| {
                el.child(self.heading("Fero", 24.).ml(px(2.)).mr_auto())
            })
            .when(!wrapped, |el| el.justify_between())
            .child(toggle);
        let settings = if collapsed {
            nav_icon("settings", "settings", "Settings", palette).w_full()
        } else {
            control("settings", palette)
                .px_2()
                .py_1p5()
                .gap_2()
                .text_size(px(12.))
                .child(icon("settings", palette.quiet, 16.))
                .child("Settings")
        }
        .on_click(cx.listener(|view, _, window, cx| view.show_settings(true, window, cx)));
        let (heading, settings_link) = if wrapped {
            (
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .child(brand)
                    .child(settings),
                None,
            )
        } else {
            (brand, Some(settings))
        };

        let sidebar = div()
            .id("filters")
            .flex()
            .flex_none()
            .gap_2()
            .bg(palette.rail)
            .border_color(palette.line)
            .when(wrapped, |el| {
                el.flex_row()
                    .flex_wrap()
                    .items_center()
                    .w(width)
                    .border_b_1()
                    .px(px(11.))
                    .py(px(9.))
            })
            .when(!wrapped, |el| {
                el.w(pane_width)
                    .flex_col()
                    .min_h_0()
                    .overflow_y_scroll()
                    .border_r_1()
                    .px(px(if width < px(732.) {
                        5.
                    } else if collapsed || width < px(1116.) {
                        9.
                    } else {
                        13.
                    }))
                    .pt(px(25.))
                    .pb_4()
            })
            .child(heading)
            .when(!wrapped && !collapsed, |el| {
                el.child(
                    div()
                        .px_2()
                        .text_size(px(10.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(palette.quiet)
                        .child("STATUS"),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(3.))
                    .when(wrapped, |el| el.flex_row().flex_wrap().w_full())
                    .when(!wrapped, |el| el.flex_col().flex_1())
                    .children(FILTERS.into_iter().zip(counts).enumerate().map(
                        |(index, (label, count))| {
                            let id = ("filter", index);
                            let active = index == self.filter;
                            let icon_palette = if active {
                                Palette {
                                    quiet: palette.accent,
                                    ..palette
                                }
                            } else {
                                palette
                            };
                            let item = if collapsed {
                                nav_icon(id, FILTER_ICONS[index], label, icon_palette)
                            } else {
                                control(id, palette)
                                    .px_2()
                                    .gap_2()
                                    .h(px(if wrapped { 32. } else { 35. }))
                                    .text_size(px(12.))
                                    .child(icon(FILTER_ICONS[index], icon_palette.quiet, 16.))
                                    .child(div().flex_1().child(label))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(palette.quiet)
                                            .child(count.to_string()),
                                    )
                            };
                            item.when(active, |el| {
                                el.bg(palette.select)
                                    .text_color(palette.accent)
                                    .font_weight(FontWeight::BOLD)
                            })
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.filter = index;
                                    view.reset_list_scroll();
                                    view.sync_inspector();
                                    cx.notify();
                                },
                            ))
                        },
                    )),
            )
            .children(settings_link);
        div()
            .relative()
            .flex()
            .flex_none()
            .min_h_0()
            .child(sidebar)
            .when(!wrapped && !collapsed && palette.interactive, |el| {
                el.child(self.resize_grip(ResizablePane::Sidebar, pane_width, palette, cx))
            })
    }

    fn library(
        &self,
        rows: &[&Transfer],
        palette: Palette,
        columns: TransferColumns,
        scroll_workspace: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let width = window.viewport_size().width;
        let narrow = width < px(732.);
        let inset = if narrow {
            14.
        } else if width < px(1116.) {
            19.
        } else {
            26.
        };
        let row_tab = self
            .selected
            .filter(|id| rows.iter().any(|row| row.id == *id))
            .or_else(|| rows.first().map(|row| row.id));
        let virtualized = rows.len() > VIRTUAL_ROW_THRESHOLD;
        div()
            .id("library")
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .when(scroll_workspace, |el| el.flex_none())
            .when(!scroll_workspace, |el| el.flex_1())
            .px(px(inset))
            .pt(px(if narrow { 20. } else { 29. }))
            .pb(px(if narrow { 16. } else { 22. }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .mb(px(if narrow { 13. } else { 21. }))
                    .child(self.heading("Transfers", heading_size(width, 31.)))
                    .child(
                        button(
                            "open-add",
                            icon_label("add", "Add Torrent", palette.on_accent),
                            palette,
                            true,
                        )
                        .text_size(px(13.))
                        .min_h(px(39.))
                        .flex_none()
                        .on_click(
                            cx.listener(|view, _, window, cx| view.begin_add(None, window, cx)),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .mb(px(if narrow { 12. } else { 20. }))
                    .child(
                        field_box(palette)
                            .id("search-frame")
                            .rounded_full()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(10.))
                            .px(px(14.))
                            .min_h(px(40.))
                            .child(icon("magnifier", palette.quiet, 16.))
                            .when(
                                palette.keyboard_focus
                                    && self.search.read(cx).focus.is_focused(window),
                                |el| el.border_color(palette.focus_ring),
                            )
                            .when(palette.interactive, |el| {
                                el.child(div().flex_1().min_w_0().child(self.search.clone()))
                            })
                            .when(!palette.interactive, |el| {
                                let search = &self.search.read(cx).text;
                                let text = if search.is_empty() {
                                    "Search transfers"
                                } else {
                                    search
                                };
                                el.child(
                                    div()
                                        .text_color(palette.quiet)
                                        .truncate()
                                        .child(text.to_owned()),
                                )
                            })
                            .when(!columns.stacked, |el| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .px(px(5.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(palette.line)
                                        .text_size(px(10.))
                                        .text_color(palette.quiet)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(if cfg!(target_os = "macos") {
                                            "⌘ F"
                                        } else {
                                            "Ctrl F"
                                        }),
                                )
                            }),
                    )
                    .child(self.sort_control(palette, columns.stacked, cx)),
            )
            .when(!self.transfers.is_empty(), |el| {
                el.child(self.selection_toolbar(rows, palette, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .min_h(px(30.))
                    .px(px(columns.inset + 1.))
                    .gap(px(columns.gap))
                    .text_size(px(10.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(palette.quiet)
                    .child(div().flex_1().child("NAME"))
                    .when(!columns.stacked, |el| {
                        el.child(div().w(px(columns.progress)).flex_none().child("PROGRESS"))
                    })
                    .child(
                        div()
                            .w(px(columns.rate))
                            .flex_none()
                            .text_right()
                            .child("RATE / ETA"),
                    )
                    .when(columns.inline_actions, |el| {
                        el.child(div().w(px(ROW_ACTIONS_WIDTH)).text_right().child("ACTIONS"))
                    }),
            )
            .child(
                div()
                    .id("rows")
                    .min_h_0()
                    .when(palette.interactive && !rows.is_empty(), |el| {
                        el.track_focus(&self.rows_focus)
                    })
                    .key_context("FeroRows")
                    .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "up" => view.select_relative(false, window, cx),
                            "down" => view.select_relative(true, window, cx),
                            "space" if view.selecting && view.rows_focus.is_focused(window) => {
                                if let Some(id) =
                                    view.inspected_row(&view.visible_rows()).map(|row| row.id)
                                    && !view.bulk_selection.insert(id)
                                {
                                    view.bulk_selection.remove(&id);
                                }
                                cx.notify();
                            }
                            _ => return,
                        }
                        cx.stop_propagation();
                    }))
                    .when(scroll_workspace, |el| el.flex_none())
                    .when(!scroll_workspace, |el| el.flex_1())
                    .when(!virtualized && !scroll_workspace, |el| {
                        el.overflow_y_scroll()
                    })
                    .track_scroll(&self.list_scroll)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(rows.is_empty(), |el| {
                        let fresh = self.transfers.is_empty();
                        let (title, help) = if fresh {
                            (
                                "No transfers yet",
                                "Add a magnet link or torrent file to begin.",
                            )
                        } else {
                            (
                                "No matching transfers",
                                "Try a different search or status filter.",
                            )
                        };
                        el.child(reveal(
                            div()
                                .flex()
                                .flex_col()
                                .items_start()
                                .gap(px(9.))
                                .py(px(46.))
                                .px_3()
                                .child(self.heading(title, 21.))
                                .child(div().text_color(palette.quiet).child(help))
                                .child(
                                    button(
                                        "empty-action",
                                        if fresh {
                                            "Add Torrent"
                                        } else {
                                            "Clear filters"
                                        },
                                        palette,
                                        false,
                                    )
                                    .on_click(cx.listener(
                                        move |view, _, window, cx| {
                                            if fresh {
                                                view.begin_add(None, window, cx);
                                            } else {
                                                view.filter = 0;
                                                view.search.update(cx, |field, cx| {
                                                    field.set(String::new(), cx)
                                                });
                                                view.reset_list_scroll();
                                                view.sync_inspector();
                                                cx.notify();
                                            }
                                        },
                                    )),
                                ),
                            ("empty-library-in", usize::from(fresh)),
                            palette.motion,
                        ))
                    })
                    .when(!virtualized, |el| {
                        el.children(rows.iter().map(|row| {
                            self.row(row, row_tab == Some(row.id), columns, palette, cx)
                        }))
                    })
                    .when(virtualized, |el| {
                        let ids: Vec<_> = rows.iter().map(|row| row.id).collect();
                        let old_count = self.virtual_list.item_count();
                        if old_count != ids.len() {
                            self.virtual_list.splice(
                                old_count.min(ids.len())..old_count,
                                ids.len().saturating_sub(old_count),
                            );
                        }
                        let view = cx.entity().downgrade();
                        el.child(
                            gpui::list(self.virtual_list.clone(), move |index, _, app| {
                                let id = ids[index];
                                view.update(app, |view, cx| {
                                    div()
                                        .pb_1()
                                        .children(
                                            view.transfers.iter().find(|row| row.id == id).map(
                                                |row| {
                                                    view.row(
                                                        row,
                                                        row_tab == Some(id),
                                                        columns,
                                                        palette,
                                                        cx,
                                                    )
                                                },
                                            ),
                                        )
                                        .into_any_element()
                                })
                                .unwrap_or_else(|_| div().into_any_element())
                            })
                            .w_full()
                            .when(scroll_workspace, |list| {
                                list.h(window.viewport_size().height * 0.60)
                            })
                            .when(!scroll_workspace, |list| list.flex_1()),
                        )
                    }),
            )
    }

    fn sort_control(&self, palette: Palette, narrow: bool, cx: &Context<Self>) -> Div {
        let label = SortBy::OPTIONS
            .iter()
            .find(|(sort, _)| *sort == self.sort)
            .map_or("Sort", |(_, label)| *label);
        div()
            .relative()
            .flex_none()
            .child(
                button(
                    "sort-transfers",
                    if narrow { "Sort" } else { label },
                    palette,
                    false,
                )
                .h(px(40.))
                .gap_2()
                .child(icon("alt-arrow-down", palette.quiet, 13.))
                .when(!self.sort_open, |el| {
                    el.tooltip(make_tooltip(format!("Sort transfers · {label}"), palette))
                })
                .on_click(cx.listener(|view, _, _, cx| {
                    if view.draft.is_none() {
                        view.sort_open = !view.sort_open;
                        cx.notify();
                    }
                })),
            )
            .when(self.sort_open, |el| {
                el.child(
                    deferred(
                        div()
                            .id("sort-options")
                            .absolute()
                            .right_0()
                            .top(relative(1.))
                            .occlude()
                            .mt_1()
                            .w(px(160.))
                            .rounded(px(18.))
                            .p_1()
                            .border_1()
                            .border_color(palette.line)
                            .bg(palette.rail)
                            .shadow_md()
                            .children(SortBy::OPTIONS.into_iter().enumerate().map(
                                |(i, (sort, label))| {
                                    control(("sort-option", i), palette)
                                        .px_2()
                                        .py_1p5()
                                        .text_sm()
                                        .child(label)
                                        .when(sort == self.sort, |el| el.bg(palette.select))
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.sort = sort;
                                            view.sort_open = false;
                                            view.reset_list_scroll();
                                            view.sync_inspector();
                                            window.focus(&view.rows_focus);
                                            cx.notify();
                                        }))
                                },
                            ))
                            .map(|menu| reveal(menu, "sort-in", palette.motion)),
                    )
                    .with_priority(1),
                )
            })
    }

    fn row(
        &self,
        row: &Transfer,
        selected: bool,
        columns: TransferColumns,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let id = row.id;
        let bulk_checked = self.bulk_selection.contains(&id);
        let (transfer_rate, eta) = row_rate(row);
        let actions = || self.transfer_actions(row, palette, cx);
        let progress_cell = || {
            div()
                .flex()
                .gap(px(6.))
                .text_size(px(if columns.stacked { 10. } else { 12. }))
                .line_height(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .when(columns.stacked, |el| {
                    el.flex_row_reverse()
                        .items_center()
                        .h(px(16.))
                        .child(percent(row))
                        .child(div().flex_1().child(progress(row, 5., palette)))
                })
                .when(!columns.stacked, |el| {
                    el.flex_col()
                        .w(px(columns.progress))
                        .flex_none()
                        .text_right()
                        .child(percent(row))
                        .child(progress(row, 5., palette))
                })
        };
        let name = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .w_full()
            .min_w_0()
            .child(
                div()
                    .w(px(columns.name))
                    .h(px(19.))
                    .truncate()
                    .text_size(px(if columns.stacked { 13. } else { 14. }))
                    .line_height(px(19.))
                    .font_weight(FontWeight::BOLD)
                    .child(row.name.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(15.))
                    .gap(px(6.))
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .child(status_label(row, palette))
                    .when(row.size > 0, |el| {
                        el.child(
                            div()
                                .text_color(palette.quiet)
                                .child(format!("·  {}", quantity(row.size))),
                        )
                    }),
            );
        div()
            .id(("row", id as usize))
            .flex()
            .flex_col()
            .flex_none()
            .justify_center()
            .gap(px(4.))
            .px(px(columns.inset))
            .min_h(px(if self.preferences.compact {
                58.
            } else if columns.stacked {
                67.
            } else {
                71.
            }))
            .py(px(if self.preferences.compact {
                7.
            } else if columns.stacked {
                8.
            } else {
                11.
            }))
            .rounded(px(CARD_RADIUS))
            .when(columns.inline_actions, |el| el.rounded_full())
            .border_1()
            .border_color(palette.line)
            .bg(palette.paper.blend(Rgba {
                a: 0.17,
                ..palette.rail
            }))
            .hover(move |el| el.bg(palette.hover))
            .when(row.name.len() > 48, |el| {
                el.tooltip(make_tooltip(row.name.clone(), palette))
            })
            .when(selected || bulk_checked, |el| {
                el.bg(palette.select)
                    .border_color(palette.outline)
                    .shadow_sm()
            })
            .when(selected && palette.keyboard_focus, |el| {
                el.border_color(palette.focus_ring)
            })
            .cursor_pointer()
            .on_click(cx.listener(move |view, _, window, cx| {
                if view.selecting && !view.bulk_selection.insert(id) {
                    view.bulk_selection.remove(&id);
                }
                view.open_transfer(id, window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(columns.gap))
                    .child(
                        div()
                            .w(px(columns.tile))
                            .flex_none()
                            .flex()
                            .justify_center()
                            .child(if self.selecting {
                                checkmark(bulk_checked, palette)
                            } else {
                                self.file_tile(&row.name, columns.tile - 2., palette)
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(columns.name))
                            .flex_none()
                            .min_w_0()
                            .gap(px(3.))
                            .child(name)
                            .when(columns.stacked, |el| el.child(progress_cell())),
                    )
                    .when(!columns.stacked, |el| el.child(progress_cell()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .w(px(columns.rate))
                            .items_end()
                            .gap(px(4.))
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .child(div().font_weight(FontWeight::BOLD).child(
                                if let Some((direction, bytes)) = transfer_rate {
                                    icon_label(direction, rate(bytes), palette.ink)
                                } else {
                                    div().child("—")
                                },
                            ))
                            .when(row.state == "Downloading", |el| {
                                el.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(palette.quiet)
                                        .child(eta.map_or("—".into(), |eta| format!("{eta} left"))),
                                )
                            }),
                    )
                    .when(columns.inline_actions, |el| el.child(actions())),
            )
            .when(!columns.inline_actions, |el| {
                el.child(div().flex().justify_end().child(actions()))
            })
    }

    fn transfer_actions(&self, row: &Transfer, palette: Palette, cx: &Context<Self>) -> Div {
        let id = row.id;
        let paused = row.paused || !row.error.is_empty();
        let destination = PathBuf::from(&row.destination);
        let action = |id, symbol, tooltip| {
            let color = if id == "row-delete" {
                palette.danger
            } else {
                palette.quiet
            };
            control(id, palette)
                .min_w(px(28.))
                .w(px(28.))
                .h(px(28.))
                .flex_none()
                .justify_center()
                .child(icon(symbol, color, 17.))
                .tooltip(make_tooltip(tooltip, palette))
        };
        div()
            .flex()
            .flex_none()
            .justify_end()
            .w(px(ROW_ACTIONS_WIDTH))
            .gap(px(2.))
            .child(
                action(
                    if paused { "row-resume" } else { "row-pause" },
                    if paused { "play" } else { "pause" },
                    if paused {
                        "Resume torrent"
                    } else {
                        "Pause torrent"
                    },
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    cx.stop_propagation();
                    view.send(Command::Pause(id, !paused), cx);
                })),
            )
            .child(
                action("row-stop", "stop", "Stop and save progress").on_click(cx.listener(
                    move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.send(Command::Stop(id), cx);
                    },
                )),
            )
            .child(
                action("row-folder", "folder-open", "Reveal download folder").on_click(
                    move |_, _, cx| {
                        cx.stop_propagation();
                        cx.reveal_path(&destination);
                    },
                ),
            )
            .child(
                action("row-remove", "list-minus", "Remove torrent and keep files").on_click(
                    cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        window.focus(&view.focus);
                        view.send(Command::Remove(id, false), cx);
                    }),
                ),
            )
            .when(row.has_metadata, |el| {
                el.child(
                    action(
                        "row-delete",
                        "trash-bin-2",
                        "Delete torrent and downloaded files…",
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.delete_files(id, window, cx);
                    })),
                )
            })
    }

    fn inspector(
        &self,
        row: Option<&Transfer>,
        palette: Palette,
        below: bool,
        width: Pixels,
        extent: Pixels,
        cx: &Context<Self>,
    ) -> Div {
        let wide = width >= px(1116.);
        let split = below && width >= px(534.);
        let inset = if wide { 20. } else { 16. };
        let pane_width = if below {
            width
                - px(if width < px(732.) && self.preferences.collapsed {
                    53.
                } else {
                    0.
                })
        } else {
            extent
        };
        let pane_inner = pane_width - px(2. * inset);
        let panel_width = if split {
            pane_inner - (pane_inner * 0.35).max(px(175.)) - px(25.)
        } else {
            pane_inner - px(if below { 0. } else { 1. })
        };
        let frame = |pane: Stateful<Div>| {
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_none()
                .min_w_0()
                .min_h_0()
                .when(below, |el| el.h(extent))
                .when(!below, |el| el.w(extent))
                .child(reveal(pane, "inspector-in", palette.motion))
                .child(
                    nav_icon(
                        "close-inspector",
                        "close",
                        "Close transfer details",
                        palette,
                    )
                    .absolute()
                    .top(px(17.))
                    .right(px(8.))
                    .bg(palette.rail)
                    .on_click(cx.listener(|view, _, window, cx| view.close_inspector(window, cx))),
                )
                .when(palette.interactive, |el| {
                    let axis = if below {
                        ResizablePane::InspectorHeight
                    } else {
                        ResizablePane::InspectorWidth
                    };
                    el.child(self.resize_grip(axis, extent, palette, cx))
                })
        };
        let pane = div()
            .id("inspector")
            .flex()
            .flex_col()
            .px(px(inset))
            .pt(px(24.))
            .pb(px(19.))
            .bg(palette.rail)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.inspector_scroll)
            .border_color(palette.line)
            .when(below, |el| el.border_t_1())
            .when(!below, |el| el.border_l_1());
        let Some(row) = row else {
            return frame(
                pane.gap(px(6.))
                    .pr(px(40.))
                    .child(self.heading("Overview", 20.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.quiet)
                            .child("Select a transfer to see its progress and controls."),
                    ),
            );
        };
        let id = row.id;
        let paused = row.paused || !row.error.is_empty();
        let (_, eta) = row_rate(row);
        let title = div()
            .flex()
            .items_center()
            .gap(px(11.))
            .min_w_0()
            .when(!split, |el| el.pr(px(24.)))
            .child(self.file_tile(&row.name, 46., palette))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        self.heading(
                            row.name.clone(),
                            if !split && pane_width < px(300.) {
                                20.
                            } else {
                                22.
                            },
                        )
                        .id("inspected-name")
                        .line_clamp(2)
                        .tooltip(make_tooltip(row.name.clone(), palette)),
                    )
                    .child(
                        status_label(row, palette)
                            .mt_1()
                            .text_size(px(11.))
                            .line_height(px(15.)),
                    ),
            );
        let pieces = match &self.live_details {
            Some(LiveDetails::Pieces(map_id, map)) if *map_id == id && !map.cells.is_empty() => {
                Some(map)
            }
            _ => None,
        };
        let show_side_by_side = split && self.inspector_tab == InspectorTab::Overview;
        let downloaded = || {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::BOLD)
                        .child("Downloaded"),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .justify_between()
                        .gap_2()
                        .child(self.heading(percent(row), 39.))
                        .child(
                            quiet(
                                if row.wanted > 0 {
                                    format!(
                                        "{} of {}",
                                        quantity(row.wanted - row.remaining),
                                        quantity(row.wanted),
                                    )
                                } else {
                                    "Size pending".into()
                                },
                                palette,
                            )
                            .pb_1()
                            .text_right()
                            .text_size(px(11.)),
                        ),
                )
                .child(progress(row, 6., palette))
        };
        let details = || {
            div()
                .flex()
                .flex_col()
                .gap(px(19.))
                .min_w_0()
                .when(!show_side_by_side, |el| el.child(downloaded()))
                .when_some(pieces.filter(|_| show_side_by_side), |el, map| {
                    el.child(download_pieces(map, panel_width, palette))
                })
                .child(
                    div()
                        .flex()
                        .border_y_1()
                        .border_color(palette.line)
                        .children(
                            [("Download", row.download_rate), ("Upload", row.upload_rate)]
                                .into_iter()
                                .enumerate()
                                .map(|(i, (label, bytes))| {
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .py(px(10.))
                                        .when(i == 1, |el| {
                                            el.pl(px(14.)).border_l_1().border_color(palette.line)
                                        })
                                        .child(quiet(label, palette).text_size(px(11.)))
                                        .child(
                                            div()
                                                .font_weight(FontWeight::BOLD)
                                                .child(rate(bytes.into())),
                                        )
                                }),
                        ),
                )
                .when_some(pieces.filter(|_| !show_side_by_side), |el, map| {
                    el.child(download_pieces(map, panel_width, palette))
                })
                .when_some(self.download_history.get(&id), |el, history| {
                    el.when(row.state == "Downloading" && history.len() > 1, |el| {
                        el.child(download_chart(history, palette))
                    })
                })
                .when(!row.error.is_empty(), |el| {
                    el.child(
                        div()
                            .border_l_2()
                            .border_color(palette.danger)
                            .p_2()
                            .bg(palette.hover)
                            .text_xs()
                            .text_color(palette.danger)
                            .child(row.error.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(detail(
                            "Time left",
                            eta.unwrap_or_else(|| "—".into()),
                            palette,
                        ))
                        .child(detail("Connected peers", row.peers.to_string(), palette))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .text_xs()
                                .min_w_0()
                                .child(div().text_color(palette.quiet).child("Save to"))
                                .child(
                                    field_box(palette)
                                        .px_2()
                                        .border_color(palette.line)
                                        .font_family(if cfg!(target_os = "macos") {
                                            "Menlo"
                                        } else {
                                            "monospace"
                                        })
                                        .text_size(px(11.))
                                        .child(row.destination.clone()),
                                ),
                        ),
                )
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .mt_4()
            .child(
                button(
                    ("pause", id as usize),
                    if paused { "Resume" } else { "Pause" },
                    palette,
                    false,
                )
                .min_w(px(76.))
                .on_click(
                    cx.listener(move |view, _, _, cx| view.send(Command::Pause(id, !paused), cx)),
                ),
            )
            .child(
                quiet_button(("remove", id as usize), "Remove from Fero", palette).on_click(
                    cx.listener(move |view, _, window, cx| {
                        window.focus(&view.focus);
                        view.send(Command::Remove(id, false), cx);
                    }),
                ),
            )
            .when(row.has_metadata, |el| {
                el.child(
                    quiet_button(("delete", id as usize), "Delete Files…", palette)
                        .text_color(palette.danger)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.delete_files(id, window, cx)
                        })),
                )
                .child(
                    quiet_button(("recheck", id as usize), "Force Recheck", palette).on_click(
                        cx.listener(move |view, _, _, cx| view.send(Command::Recheck(id), cx)),
                    ),
                )
            });
        let tab_palette = Palette {
            hover: palette.rail.blend(Rgba {
                a: 0.065,
                ..palette.ink
            }),
            ..palette
        };
        let tabs = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .pb(px(6.))
            .border_b_1()
            .border_color(palette.line)
            .children(
                [
                    (InspectorTab::Overview, "Overview"),
                    (InspectorTab::Files, "Files"),
                    (InspectorTab::Peers, "Peers"),
                    (InspectorTab::Trackers, "Trackers"),
                ]
                .map(|(tab, label)| {
                    let active = self.inspector_tab == tab;
                    control(("inspector-tab", tab as usize), tab_palette)
                        .relative()
                        .px(px(10.))
                        .h(px(28.))
                        .when(palette.interactive, |el| {
                            el.active(move |el| el.bg(palette.select))
                        })
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if active { palette.ink } else { palette.quiet })
                        .child(label)
                        .when(active, |el| {
                            el.child(
                                div()
                                    .absolute()
                                    .bottom(px(-7.))
                                    .left(px(10.))
                                    .right(px(10.))
                                    .h(px(2.))
                                    .rounded_full()
                                    .bg(palette.accent),
                            )
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.inspector_tab = tab;
                            view.inspector_scroll.set_offset(gpui::Point::default());
                            view.inspector_item_limit = FILE_PAGE_SIZE;
                            view.sync_inspector();
                            cx.notify();
                        }))
                }),
            );
        let panel = div()
            .min_w_0()
            .child(tabs)
            .child(reveal(
                div().mt_4().child(match self.inspector_tab {
                    InspectorTab::Overview => details(),
                    InspectorTab::Files => self.transfer_files(id, palette, cx),
                    InspectorTab::Peers => self.transfer_peers(id, palette, cx),
                    InspectorTab::Trackers => self.transfer_trackers(id, paused, palette, cx),
                }),
                ("inspector-tab-in", self.inspector_tab as usize),
                palette.motion,
            ))
            .when(self.inspector_tab == InspectorTab::Overview, |el| {
                el.child(self.transfer_options(id, palette, cx))
            });
        frame(if split {
            pane.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(25.))
                    .child(
                        div()
                            .w(gpui::relative(0.35))
                            .min_w(px(175.))
                            .child(title)
                            .when(show_side_by_side, |el| {
                                el.child(div().mt_3().child(downloaded()))
                            })
                            .child(actions),
                    )
                    .child(div().flex_1().min_w_0().child(panel)),
            )
        } else {
            pane.child(div().mb_5().child(title))
                .child(panel)
                .child(actions)
        })
    }

    fn transfer_peers(&self, id: i64, palette: Palette, cx: &Context<Self>) -> Div {
        let peers = match &self.live_details {
            Some(LiveDetails::Peers(peer_id, result)) if *peer_id == id => match result {
                Ok(peers) if peers.is_empty() => return quiet("No peers connected.", palette),
                Ok(peers) => peers,
                Err(_) => return quiet("Couldn’t read connected peers. Retrying…", palette),
            },
            _ => return quiet("Loading peers…", palette),
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                quiet(format!("{} connected", peers.len()), palette)
                    .text_xs()
                    .mb_1(),
            )
            .children(peers.iter().take(self.inspector_item_limit).map(|peer| {
                field_box(palette)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .child(peer.endpoint.clone()),
                    )
                    .child(
                        quiet(
                            [
                                &peer.transport[..],
                                if peer.seed { "Seed" } else { "Peer" },
                                &peer.client,
                            ]
                            .into_iter()
                            .filter(|value| !value.is_empty())
                            .collect::<Vec<_>>()
                            .join(" · "),
                            palette,
                        )
                        .text_xs()
                        .w_full()
                        .min_w_0()
                        .line_clamp(2),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .gap_2()
                            .text_xs()
                            .child(icon_label(
                                "arrow-down",
                                rate(peer.download_rate.into()),
                                palette.ink,
                            ))
                            .child(icon_label(
                                "arrow-up",
                                rate(peer.upload_rate.into()),
                                palette.ink,
                            )),
                    )
            }))
            .when(peers.len() > self.inspector_item_limit, |el| {
                el.child(
                    button("more-transfer-peers", "Show more peers", palette, false).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.inspector_item_limit += FILE_PAGE_SIZE;
                            cx.notify();
                        }),
                    ),
                )
            })
    }

    fn transfer_trackers(
        &self,
        id: i64,
        paused: bool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Div {
        let trackers = match &self.live_details {
            Some(LiveDetails::Trackers(transfer_id, result)) if *transfer_id == id => {
                match result {
                    Ok(trackers) => trackers,
                    Err(error) => return quiet(error.clone(), palette),
                }
            }
            _ => return quiet("Loading trackers…", palette),
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .mb_1()
                    .child(
                        quiet(
                            if trackers.is_empty() {
                                "No trackers configured".into()
                            } else {
                                format!("{} configured", trackers.len())
                            },
                            palette,
                        )
                        .text_xs(),
                    )
                    .child(
                        button("add-tracker", "Add Tracker…", palette, false).on_click(
                            cx.listener(move |view, _, window, cx| {
                                view.edit_tracker(id, None, window, cx)
                            }),
                        ),
                    )
                    .when(!paused && !trackers.is_empty(), |el| {
                        el.child(
                            button(("reannounce", id as usize), "Reannounce", palette, false)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.send(Command::Reannounce(id), cx);
                                })),
                        )
                    }),
            )
            .children(
                trackers
                    .iter()
                    .take(self.inspector_item_limit)
                    .enumerate()
                    .map(|(i, tracker)| {
                        field_box(palette)
                            .min_w_0()
                            .child(
                                div()
                                    .id(("tracker-url", i))
                                    .w_full()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .line_clamp(2)
                                    .tooltip(make_tooltip(tracker.url.clone(), palette))
                                    .child(tracker.url.clone()),
                            )
                            .child(
                                quiet(
                                    format!("Tier {} · {}", tracker.tier, tracker.status),
                                    palette,
                                )
                                .text_xs(),
                            )
                            .when(!tracker.error.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(palette.danger)
                                        .line_clamp(2)
                                        .child(tracker.error.clone()),
                                )
                            })
                            .child(self.tracker_actions(id, i, tracker, palette, cx))
                    }),
            )
            .when(trackers.len() > self.inspector_item_limit, |el| {
                el.child(
                    button("more-trackers", "Show more trackers", palette, false).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.inspector_item_limit += FILE_PAGE_SIZE;
                            cx.notify();
                        }),
                    ),
                )
            })
    }

    fn transfer_files(&self, id: i64, palette: Palette, cx: &Context<Self>) -> Div {
        let files = match &self.inspected_files {
            Some(Ok(files)) => files,
            Some(Err(error)) => return quiet(error.clone(), palette),
            None => return quiet("Waiting for torrent metadata…", palette),
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(files.iter().take(self.inspector_item_limit).map(|file| {
                let index = file.index;
                field_box(palette)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child(file.path.clone()))
                    .child(quiet(
                        format!(
                            "{} · {}",
                            quantity(file.size),
                            if file.selected { "Selected" } else { "Skipped" }
                        ),
                        palette,
                    ))
                    .child(div().flex().flex_wrap().gap_1().children(
                        [(0, "Skip"), (1, "Low"), (4, "Normal"), (7, "High")].map(
                            |(priority, label)| {
                                choice(
                                    (label, index as usize),
                                    label,
                                    file.priority == priority,
                                    palette,
                                )
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.send(Command::FilePriority(id, index, priority), cx)
                                    },
                                ))
                            },
                        ),
                    ))
            }))
            .when(files.len() > self.inspector_item_limit, |el| {
                el.child(
                    button("more-transfer-files", "Show more files", palette, false).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.inspector_item_limit += FILE_PAGE_SIZE;
                            cx.notify();
                        }),
                    ),
                )
            })
    }

    fn footer(&self, visible: usize, palette: Palette, cx: &Context<Self>) -> Div {
        let rates = [
            ("arrow-down", "Download", palette.accent),
            ("arrow-up", "Upload", palette.seed),
        ]
        .into_iter()
        .zip(traffic_rates(&self.transfers));
        let count = if visible == self.transfers.len() {
            format!(
                "{visible} {}",
                if visible == 1 {
                    "transfer"
                } else {
                    "transfers"
                }
            )
        } else {
            format!("{visible} of {} transfers", self.transfers.len())
        };
        let toggle = control("traffic-toggle", palette)
            .rounded_none()
            .flex_wrap()
            .justify_between()
            .gap_x_4()
            .gap_y_1()
            .min_h(px(38.))
            .px_4()
            .py(px(7.))
            .bg(palette.chrome)
            .text_color(palette.quiet)
            .text_size(px(11.))
            .child(count)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .children(rates.clone().map(|((arrow, label, color), bytes)| {
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon(arrow, color, 13.))
                            .child(label)
                            .child(
                                div()
                                    .text_color(palette.ink)
                                    .font_weight(FontWeight::BOLD)
                                    .child(rate(bytes)),
                            )
                    }))
                    .child(
                        icon("alt-arrow-down", palette.quiet, 13.).with_transformation(
                            gpui::Transformation::scale(size(
                                1.,
                                if self.traffic_open { 1. } else { -1. },
                            )),
                        ),
                    ),
            )
            .when(palette.interactive, |el| {
                el.when(!self.traffic_open, |el| {
                    el.tooltip(make_tooltip("Show transfer activity", palette))
                })
                .on_click(cx.listener(|view, _, _, cx| {
                    view.traffic_open = !view.traffic_open;
                    cx.notify();
                }))
            });
        div()
            .flex_none()
            .border_t_1()
            .border_color(palette.line)
            .when(self.traffic_open, |el| {
                el.child(
                    div()
                        .bg(palette.chrome)
                        .shadow(vec![gpui::BoxShadow {
                            color: rgba(0x1a110e40).into(),
                            offset: point(px(0.), px(-4.)),
                            blur_radius: px(13.),
                            spread_radius: px(0.),
                        }])
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(palette.line)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Transfer activity"),
                                )
                                .child(quiet("All transfers · Last minute", palette).text_xs()),
                        )
                        .child(div().flex().gap_3().mt_2().children(
                            rates.zip(&self.traffic_history).map(
                                |(((arrow, label, color), current), history)| {
                                    let peak = history.iter().copied().fold(current, i64::max);
                                    field_box(palette)
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_wrap()
                                                .items_center()
                                                .justify_between()
                                                .gap_x_2()
                                                .child(icon_label(arrow, label, color).text_xs())
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_weight(FontWeight::BOLD)
                                                        .child(rate(current)),
                                                ),
                                        )
                                        .child(
                                            quiet(format!("Peak {}", rate(peak)), palette)
                                                .text_xs(),
                                        )
                                        .child(rate_plot(
                                            history.iter().copied().collect(),
                                            color,
                                            palette,
                                        ))
                                },
                            ),
                        ))
                        .child(rate_axis(palette))
                        .map(|chart| reveal(chart, "traffic-in", palette.motion)),
                )
            })
            .child(toggle)
    }

    fn add_dialog(
        &self,
        draft: &Draft,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let busy = draft.submitting;
        let controls = Palette {
            interactive: !busy,
            ..palette
        };
        let modes = div()
            .flex()
            .gap(px(3.))
            .p(px(3.))
            .rounded_full()
            .bg(palette.field)
            .border_1()
            .border_color(palette.outline)
            .children(
                [
                    (AddMode::Magnet, "Magnet link"),
                    (AddMode::File, "Torrent file"),
                ]
                .into_iter()
                .enumerate()
                .map(|(i, (mode, label))| {
                    control(("add-mode", i), controls)
                        .px(px(10.))
                        .py_1()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .when(mode == draft.mode, |el| {
                            el.bg(palette.paper).border_color(palette.line)
                        })
                        .when(mode != draft.mode, |el| el.text_color(palette.quiet))
                        .child(label)
                        .on_click(
                            cx.listener(move |view, _, window, cx| view.set_mode(mode, window, cx)),
                        )
                }),
            );
        let source = if draft.mode == AddMode::Magnet {
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .child("Magnet link"),
                        )
                        .child(self.input("magnet-frame", &self.magnet, palette, busy, window, cx))
                        .child(
                            quiet(
                                "You can also drop a .torrent file into this window.",
                                palette,
                            )
                            .text_size(px(11.)),
                        ),
                )
                .when(draft.files.is_empty(), |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .child("When to start"),
                            )
                            .child(
                                div().flex().flex_wrap().gap_x_3().gap_y_1().children(
                                    [(false, "Review files first"), (true, "Start immediately")]
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, (immediate, label))| {
                                            let active = immediate == draft.start_immediately;
                                            control(("magnet-timing", i), controls)
                                                .gap(px(7.))
                                                .text_xs()
                                                .py_0p5()
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .size(px(15.))
                                                        .rounded_full()
                                                        .border_1()
                                                        .border_color(if active {
                                                            palette.accent
                                                        } else {
                                                            palette.outline
                                                        })
                                                        .when(active, |el| {
                                                            el.child(
                                                                div()
                                                                    .size(px(7.))
                                                                    .rounded_full()
                                                                    .bg(palette.accent),
                                                            )
                                                        }),
                                                )
                                                .child(label)
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    if let Some(draft) = &mut view.draft
                                                        && !draft.submitting
                                                        && draft.start_immediately != immediate
                                                    {
                                                        draft.start_immediately = immediate;
                                                        view.reset_preview();
                                                        cx.notify();
                                                    }
                                                }))
                                        }),
                                ),
                            ),
                    )
                })
        } else {
            let has_file = draft.file.is_some();
            let filename = draft
                .file
                .as_ref()
                .map(|path| {
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                })
                .unwrap_or_else(|| "Drop a .torrent file here".into());
            let detail = if draft.previewing {
                "Reading torrent…".into()
            } else if draft.files.len() == 1 {
                "1 file ready to review".into()
            } else if !draft.files.is_empty() {
                format!("{} files ready to review", draft.files.len())
            } else if has_file {
                "Drop another torrent to replace this file".into()
            } else {
                "The file list opens for review.".into()
            };
            let drop_zone = div()
                .id("drop-zone")
                .flex()
                .items_center()
                .gap(px(9.))
                .px_4()
                .py_2()
                .rounded(px(CARD_RADIUS))
                .when(has_file, |el| el.rounded_full())
                .bg(palette.field)
                .border_1()
                .border_color(palette.outline)
                .when(!has_file, |el| el.flex_col().py(px(15.)).text_center())
                .when(!busy, |el| el.on_drop(cx.listener(Self::dropped)))
                .drag_over::<ExternalPaths>(move |style, paths, _, _| {
                    if !busy && paths.paths().iter().any(|path| is_torrent(path)) {
                        style.border_color(palette.accent).bg(palette.select)
                    } else {
                        style
                    }
                })
                .when(!has_file, |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(32.))
                            .rounded_full()
                            .bg(palette.select)
                            .text_color(palette.accent)
                            .child(icon("arrow-down", palette.accent, 17.)),
                    )
                })
                .child(
                    div()
                        .when(has_file, |el| el.flex_1())
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(filename),
                        )
                        .child(quiet(detail, palette)),
                )
                .child(
                    button(
                        "pick-torrent",
                        if has_file {
                            "Replace…"
                        } else {
                            "Browse Files…"
                        },
                        controls,
                        false,
                    )
                    .flex_none()
                    .py_1p5()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.pick_path(Picker::Torrent, window, cx)
                    })),
                );
            div().child(drop_zone)
        };
        let picked_bytes = draft
            .files
            .iter()
            .filter(|file| !draft.skipped.contains(&file.index))
            .fold(0_i64, |total, file| total.saturating_add(file.size));
        let submit_label = if busy {
            "Adding…"
        } else if draft.mode == AddMode::Magnet
            && !draft.start_immediately
            && draft.files.is_empty()
        {
            if draft.previewing {
                "Fetching metadata…"
            } else {
                "Get File List"
            }
        } else if draft.mode == AddMode::Magnet && draft.start_immediately && !draft.start_paused {
            "Start Download"
        } else {
            "Add Torrent"
        };
        let can_submit = !busy
            && !draft.previewing
            && (draft.files.is_empty() || draft.skipped.len() < draft.files.len());
        let inset = px(if window.viewport_size().width < px(534.) {
            16.
        } else {
            23.
        });
        let body = div()
            .id("add-form")
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(14.))
            .px(inset)
            .pb_3()
            .child(div().flex().child(modes))
            .child(source)
            .when(draft.previewing && draft.mode == AddMode::Magnet, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(motion::pending(
                            palette.accent,
                            palette.motion && window.is_window_active(),
                        ))
                        .child(quiet("Contacting peers for the file list…", palette)),
                )
            })
            .when(!draft.files.is_empty(), |el| {
                el.child(reveal(
                    self.review_files(draft, controls, window, cx),
                    "file-review-in",
                    palette.motion,
                ))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Save to"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                self.input(
                                    "destination-frame",
                                    &self.destination,
                                    palette,
                                    busy,
                                    window,
                                    cx,
                                )
                                .flex_1()
                                .min_w_0(),
                            )
                            .child(
                                button("pick-folder", "Choose Folder…", controls, false)
                                    .flex_none()
                                    .py_1p5()
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.pick_path(Picker::Destination, window, cx)
                                    })),
                            ),
                    ),
            )
            .when_some(draft.error.as_ref(), |el, error| {
                el.child(
                    field_box(palette)
                        .border_color(palette.danger)
                        .text_sm()
                        .text_color(palette.danger)
                        .child(error.clone()),
                )
            })
            .child(
                checkbox("start-paused", "Start paused", draft.start_paused, controls)
                    .mr_auto()
                    .px_0()
                    .text_xs()
                    .on_click(cx.listener(|view, _, _, cx| {
                        if let Some(draft) = &mut view.draft
                            && !draft.submitting
                        {
                            draft.start_paused = !draft.start_paused;
                            cx.notify();
                        }
                    })),
            );

        div()
            .id("add-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_drop(cx.listener(Self::dropped))
            .p_3()
            .child(motion::backdrop(palette.motion))
            .child(
                div()
                    .id("add-dialog")
                    .flex()
                    .flex_col()
                    .w(px(568.))
                    .max_w_full()
                    .max_h_full()
                    .rounded(px(28.))
                    .border_1()
                    .border_color(palette.outline)
                    .bg(palette.paper)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_between()
                            .px(inset)
                            .pt(inset)
                            .pb(px(17.))
                            .child(self.heading("Add Torrent", 28.))
                            .child(
                                nav_icon("close-add", "close", "Close Add Torrent", controls)
                                    .size(px(30.))
                                    .p_0()
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        if view
                                            .draft
                                            .as_ref()
                                            .is_some_and(|draft| !draft.submitting)
                                        {
                                            view.close_add(window, cx);
                                        }
                                    })),
                            ),
                    )
                    .child(body)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .px(inset)
                            .py_3()
                            .border_t_1()
                            .border_color(palette.line)
                            .child(div().flex_1().when(!draft.files.is_empty(), |el| {
                                el.child(quiet(
                                    format!("{} to download", quantity(picked_bytes)),
                                    palette,
                                ))
                            }))
                            .child(quiet_button("cancel-add", "Cancel", controls).on_click(
                                cx.listener(|view, _, window, cx| {
                                    if view.draft.as_ref().is_some_and(|draft| !draft.submitting) {
                                        view.close_add(window, cx);
                                    }
                                }),
                            ))
                            .child(
                                button(
                                    "submit-add",
                                    submit_label,
                                    Palette {
                                        interactive: can_submit,
                                        ..palette
                                    },
                                    true,
                                )
                                .when(!can_submit, |el| el.opacity(0.55))
                                .on_click(cx.listener(|view, _, _, cx| view.submit(cx))),
                            ),
                    )
                    .map(|dialog| motion::reveal_dialog(dialog, "add-in", palette.motion)),
            )
    }

    fn review_files(
        &self,
        draft: &Draft,
        palette: Palette,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let reviewed = draft
            .files
            .iter()
            .find(|file| Some(file.index) == draft.reviewed);
        let destination = self.entered_destination(cx);
        let search = self.file_search.read(cx).text.trim().to_lowercase();
        let mut matches = draft
            .files
            .iter()
            .filter(|file| search.is_empty() || file.path.to_lowercase().contains(&search));
        let entries: Vec<_> = matches.by_ref().take(draft.file_limit).collect();
        let has_more = matches.next().is_some();

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2p5()
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(palette.line)
            .bg(palette.rail)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Files to download"),
                    )
                    .child(quiet(
                        format!(
                            "{} of {} selected",
                            draft.files.len() - draft.skipped.len(),
                            draft.files.len()
                        ),
                        palette,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        button("select-all-files", "Select all", palette, false)
                            .py_1()
                            .on_click(cx.listener(|view, _, _, cx| {
                                if let Some(draft) = &mut view.draft
                                    && !draft.submitting
                                {
                                    draft.skipped.clear();
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        button("clear-files", "Clear selection", palette, false)
                            .py_1()
                            .on_click(cx.listener(|view, _, _, cx| {
                                if let Some(draft) = &mut view.draft
                                    && !draft.submitting
                                {
                                    draft.skipped =
                                        draft.files.iter().map(|file| file.index).collect();
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .when(draft.files.len() > 12, |el| {
                el.child(self.input(
                    "file-search-frame",
                    &self.file_search,
                    palette,
                    draft.submitting,
                    window,
                    cx,
                ))
            })
            .child(
                div()
                    .id("review-list")
                    .max_h(px(144.))
                    .overflow_y_scroll()
                    .track_scroll(&self.file_scroll)
                    .rounded(px(18.))
                    .bg(palette.paper)
                    .border_1()
                    .border_color(palette.line)
                    .when(entries.is_empty(), |el| {
                        el.child(quiet("No matching files.", palette).p_2())
                    })
                    .children(
                        entries
                            .iter()
                            .map(|file| self.review_row(file, draft, palette, cx)),
                    )
                    .when(has_more, |el| {
                        el.child(
                            button("more-review-files", "Show more files", palette, false)
                                .m_1()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    if let Some(draft) = &mut view.draft
                                        && !draft.submitting
                                    {
                                        draft.file_limit += FILE_PAGE_SIZE;
                                        cx.notify();
                                    }
                                })),
                        )
                    }),
            )
            .when_some(reviewed, |el, file| {
                let (name, _) = file_parts(&file.path);
                let full_path = destination
                    .as_ref()
                    .map(|folder| folder.join(&file.path).display().to_string());
                el.child(
                    field_box(palette)
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .justify_between()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(name.to_owned()),
                                )
                                .child(
                                    quiet(
                                        format!(
                                            "{} · {}",
                                            quantity(file.size),
                                            if draft.skipped.contains(&file.index) {
                                                "Skipped"
                                            } else {
                                                "Selected"
                                            }
                                        ),
                                        palette,
                                    )
                                    .flex_none(),
                                ),
                        )
                        .when_some(full_path, |el, path| {
                            el.child(
                                div()
                                    .id("review-path")
                                    .text_xs()
                                    .line_clamp(2)
                                    .tooltip(make_tooltip(path.clone(), palette))
                                    .child(path),
                            )
                        })
                        .when(destination.is_none(), |el| {
                            el.child(quiet("Choose a folder to see the full save path.", palette))
                        }),
                )
            })
    }

    fn review_row(
        &self,
        file: &TorrentFile,
        draft: &Draft,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Div {
        let index = file.index;
        let selected = !draft.skipped.contains(&index);
        let (name, folder) = file_parts(&file.path);
        div()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .py_0p5()
            .border_b_1()
            .border_color(palette.line)
            .child(
                control(("file-selection", index as usize), palette)
                    .flex_none()
                    .p_1()
                    .tooltip(make_tooltip(
                        format!("{} {}", if selected { "Skip" } else { "Download" }, name),
                        palette,
                    ))
                    .child(checkmark(selected, palette))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(draft) = &mut view.draft
                            && !draft.submitting
                        {
                            if !draft.skipped.remove(&index) {
                                draft.skipped.insert(index);
                            }
                            draft.reviewed = Some(index);
                            cx.notify();
                        }
                    })),
            )
            .child(
                control(("file-preview", index as usize), palette)
                    .flex_col()
                    .items_start()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .py_0p5()
                    .when(Some(index) == draft.reviewed, |el| el.bg(palette.select))
                    .tooltip(make_tooltip(file.path.clone(), palette))
                    .child(
                        div()
                            .max_w_full()
                            .truncate()
                            .text_sm()
                            .line_height(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name.to_owned()),
                    )
                    .when_some(folder, |el, folder| {
                        el.child(
                            quiet(folder.to_owned(), palette)
                                .line_height(px(15.))
                                .max_w_full()
                                .truncate(),
                        )
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(draft) = &mut view.draft
                            && !draft.submitting
                        {
                            draft.reviewed = Some(index);
                            cx.notify();
                        }
                    })),
            )
            .child(quiet(quantity(file.size), palette).flex_none().pr_2())
    }

    fn input(
        &self,
        id: impl Into<ElementId>,
        field: &Entity<TextField>,
        palette: Palette,
        read_only: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        field_box(palette)
            .id(id)
            .rounded_full()
            .when(
                palette.keyboard_focus && field.read(cx).focus.is_focused(window),
                |el| el.border_color(palette.focus_ring),
            )
            .when(!read_only, |el| el.child(field.clone()))
            .when(read_only, |el| {
                el.child(div().truncate().child(field.read(cx).text.clone()))
            })
    }

    fn heading(&self, text: impl Into<gpui::SharedString>, em: f32) -> Div {
        div()
            .font(self.heading_font.clone())
            .font_weight(FontWeight::BOLD)
            .text_size(px(em))
            .line_height(px(em * 1.14))
            .child(text.into())
    }

    fn file_tile(&self, name: &str, size: f32, palette: Palette) -> Div {
        let extension = Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_uppercase();
        let kind = match extension.as_str() {
            "ISO" | "IMG" | "DMG" => Some(0),
            "MP4" | "MKV" | "MOV" | "WEBM" | "AVI" | "M4V" => Some(1),
            "ZIP" | "RAR" | "7Z" | "TAR" | "GZ" | "XZ" | "BZ2" | "ZST" => Some(2),
            "FLAC" | "MP3" | "WAV" | "AAC" | "OGG" | "M4A" => Some(3),
            _ => None,
        };
        let label = if !extension.is_empty()
            && extension.len() <= 4
            && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            extension
        } else {
            "TOR".into()
        };
        let (fill, ink) = kind.map_or((palette.field, palette.quiet), |i| palette.tile_colors[i]);
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(size))
            .rounded_full()
            .border_1()
            .border_color(fill.blend(Rgba { a: 0.16, ..ink }))
            .bg(fill)
            .text_color(ink)
            .font(self.heading_font.clone())
            .font_weight(FontWeight::BOLD)
            .text_size(px(if size < 30. { 9. } else { 10. }))
            .child(label)
    }
}

impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette {
            keyboard_focus: self.keyboard_focus,
            motion: !self.preferences.reduce_motion,
            ..Palette::new(
                window.appearance(),
                self.preferences.theme,
                self.preferences.accent,
            )
        };
        let library_palette = Palette {
            interactive: self.draft.is_none() && self.editor.is_none(),
            ..palette
        };
        let viewport = window.viewport_size();
        let width = viewport.width;
        let wide = width >= px(1116.);
        let narrow = width < px(732.);
        let below = width <= px(936.);
        let collapsed = self.preferences.collapsed;
        let wrapped = narrow && !collapsed;
        let scroll_workspace = wrapped && !self.settings;
        let rows = self.visible_rows();
        let row = self.inspected_row(&rows);
        let show_inspector = self.inspector_open
            && !self.settings
            && (!narrow || (row.is_some() && !self.traffic_open));
        let inspector_in_library = show_inspector && below && narrow;
        let sidebar_width = if collapsed {
            px(if narrow { 53. } else { 65. })
        } else if wrapped {
            px(0.)
        } else {
            self.sidebar_width
                .unwrap_or(px(if wide { 200. } else { 178. }))
        };
        let inspector_width = if below || !show_inspector {
            px(0.)
        } else {
            let maximum = (width - sidebar_width - px(350.)).clamp(px(245.), px(500.));
            self.inspector_width
                .unwrap_or(px(if wide { 320. } else { 283. }))
                .min(maximum)
        };
        let inspector_extent = if below {
            self.inspector_height
                .unwrap_or_else(|| {
                    let preferred = px(if width >= px(534.) { 268. } else { 244. });
                    if narrow {
                        preferred.min(viewport.height * 0.38)
                    } else {
                        preferred
                    }
                })
                .min(viewport.height * 0.65)
        } else {
            inspector_width
        };
        let library_inset = if narrow {
            14.
        } else if wide {
            26.
        } else {
            19.
        };
        let columns = TransferColumns::new(
            width - sidebar_width - inspector_width - px(2. * library_inset),
            wide,
        );
        let main = div()
            .id("main-content")
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .when(wrapped, |el| el.w(width).flex_col())
            .when(scroll_workspace, |el| {
                el.overflow_y_scroll().track_scroll(&self.narrow_scroll)
            })
            .when(inspector_in_library, |el| el.flex_col())
            .child(self.library(
                &rows,
                library_palette,
                columns,
                scroll_workspace,
                window,
                cx,
            ))
            .when(show_inspector && (!below || inspector_in_library), |el| {
                el.child(self.inspector(row, library_palette, below, width, inspector_extent, cx))
            });
        let work_area = div()
            .flex()
            .min_h_0()
            .when(wrapped, |el| el.w(width).flex_col())
            .flex_1()
            .when(!self.settings, |el| {
                el.child(reveal(
                    self.sidebar(library_palette, width, sidebar_width, wrapped, cx),
                    ("sidebar-in", usize::from(collapsed)),
                    palette.motion,
                ))
            })
            .when(self.settings && !narrow, |el| {
                el.child(self.settings_sidebar(library_palette, cx))
            })
            .child(if self.settings {
                reveal(
                    self.settings(library_palette, window, cx),
                    "settings-in",
                    palette.motion,
                )
            } else {
                reveal(main, "library-in", palette.motion)
            });
        let content = div()
            .id("workspace")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(work_area)
            .when(show_inspector && below && !inspector_in_library, |el| {
                el.child(self.inspector(row, library_palette, below, width, inspector_extent, cx))
            });
        div()
            .id("fero")
            .track_focus(&self.focus)
            .relative()
            .key_context(if self.draft.is_some() {
                "FeroAdd"
            } else if self.editor.is_some() {
                "FeroManagement"
            } else if self.settings {
                "FeroSettings"
            } else {
                "FeroLibrary"
            })
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font(self.body_font.clone())
            .text_size(px(14.))
            .bg(palette.paper)
            .text_color(palette.ink)
            .capture_key_down(cx.listener(|view, _, _, cx| {
                view.set_keyboard_focus(true, cx);
            }))
            .capture_any_mouse_down(cx.listener(|view, _, _, cx| {
                view.set_keyboard_focus(false, cx);
            }))
            .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    view.resize_drag = None;
                } else if let Some((pane, size, start)) = view.resize_drag {
                    let delta = event.position - start;
                    let offset = match pane {
                        ResizablePane::Sidebar => delta.x,
                        ResizablePane::InspectorWidth => -delta.x,
                        ResizablePane::InspectorHeight => -delta.y,
                    };
                    view.set_pane_size(pane, size + offset, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.resize_drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.resize_drag = None),
            )
            .on_action(
                cx.listener(|view, _: &OpenAdd, window, cx| view.begin_add(None, window, cx)),
            )
            .on_action(cx.listener(|view, _: &OpenSettings, window, cx| {
                view.show_settings(!view.settings, window, cx);
            }))
            .on_action(cx.listener(|view, _: &ToggleSidebar, _, cx| view.toggle_sidebar(cx)))
            .on_action(cx.listener(|view, _: &PasteMagnet, window, cx| {
                if let Some(link) = cx.read_from_clipboard().and_then(|item| item.text())
                    && link.trim().starts_with("magnet:?")
                {
                    view.begin_add(Some(Source::Magnet(link.trim().to_owned())), window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &Find, window, cx| {
                view.set_keyboard_focus(true, cx);
                view.narrow_scroll.set_offset(gpui::Point::default());
                window.focus(&view.search.read(cx).focus);
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &Submit, _, cx| {
                if view.editor.is_some() {
                    view.save_editor(cx);
                } else {
                    view.submit(cx);
                }
            }))
            .on_action(cx.listener(|view, _: &Dismiss, window, cx| {
                if let Some(editor) = &view.editor {
                    if !editor.busy() {
                        view.close_editor(window, cx);
                    }
                } else if view.draft.as_ref().is_some_and(|draft| !draft.submitting) {
                    view.close_add(window, cx);
                } else if view.sort_open {
                    view.sort_open = false;
                    cx.notify();
                } else if view.draft.is_none() && view.settings {
                    view.back_from_settings(window, cx);
                } else if view.draft.is_none() && view.traffic_open {
                    view.traffic_open = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|view, _: &Tab, window, cx| {
                view.set_keyboard_focus(true, cx);
                window.focus_next();
            }))
            .on_action(cx.listener(|view, _: &BackTab, window, cx| {
                view.set_keyboard_focus(true, cx);
                window.focus_prev();
            }))
            .on_drop(cx.listener(Self::dropped))
            .child(content)
            .when_some(self.management.network_problem.as_ref(), |el, problem| {
                el.child(reveal(
                    div()
                        .flex_none()
                        .px_4()
                        .py_2()
                        .bg(palette.select)
                        .text_color(palette.danger)
                        .text_sm()
                        .child(format!("{problem}.")),
                    "network-notice-in",
                    palette.motion,
                ))
            })
            .when_some(self.error.as_ref(), |el, error| {
                el.child(reveal(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_4()
                        .py_2()
                        .bg(palette.select)
                        .text_color(palette.danger)
                        .text_sm()
                        .child(error.clone())
                        .child(
                            button("dismiss-error", "Dismiss", library_palette, false).on_click(
                                cx.listener(|view, _, _, cx| {
                                    view.error = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                    "error-notice-in",
                    palette.motion,
                ))
            })
            .child(self.footer(rows.len(), library_palette, cx))
            .when_some(self.draft.as_ref(), |el, draft| {
                el.child(self.add_dialog(draft, palette, window, cx))
            })
            .when_some(self.editor.as_ref(), |el, editor| {
                el.child(self.editor_dialog(editor, palette, window, cx))
            })
    }
}

fn control(id: impl Into<ElementId>, palette: Palette) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(gpui::transparent_black())
        .when(palette.interactive, |el| {
            el.tab_index(0)
                .cursor_pointer()
                .when(palette.keyboard_focus, |el| {
                    el.focus(move |el| el.border_color(palette.focus_ring))
                })
                .hover(move |el| el.bg(palette.hover))
                .active(move |el| {
                    el.bg(palette.hover.blend(Rgba {
                        a: 0.09,
                        ..palette.ink
                    }))
                })
        })
}

fn button(
    id: impl Into<ElementId>,
    label: impl IntoElement,
    palette: Palette,
    primary: bool,
) -> Stateful<Div> {
    let interaction = if primary {
        Palette {
            hover: palette.accent.blend(Rgba {
                a: 0.1,
                ..palette.on_accent
            }),
            ..palette
        }
    } else {
        palette
    };
    control(id, interaction)
        .px(px(13.))
        .py_2()
        .justify_center()
        .text_xs()
        .line_height(px(17.))
        .font_weight(FontWeight::BOLD)
        .bg(if primary {
            palette.accent
        } else {
            palette.field
        })
        .when(primary, |el| el.text_color(palette.on_accent).shadow_sm())
        .when(!primary, |el| el.border_color(palette.outline))
        .child(label)
}

fn quiet_button(id: impl Into<ElementId>, label: &'static str, palette: Palette) -> Stateful<Div> {
    control(id, palette)
        .px(px(7.))
        .py_2()
        .text_xs()
        .line_height(px(17.))
        .font_weight(FontWeight::BOLD)
        .text_color(palette.quiet)
        .child(label)
}

fn field_box(palette: Palette) -> Div {
    div()
        .px_3()
        .py_1p5()
        .rounded(px(18.))
        .bg(palette.field)
        .border_1()
        .border_color(palette.outline)
}

fn heading_size(width: gpui::Pixels, minimum: f32) -> f32 {
    (f32::from(width) * 0.033 + 2.).clamp(minimum, 44.)
}

fn status_label(row: &Transfer, palette: Palette) -> Div {
    let color = state_color(row, palette);
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_color(color)
        .font_weight(FontWeight::SEMIBOLD)
        .child(div().size(px(5.)).flex_none().rounded_full().bg(color))
        .child(row.state.clone())
}

fn checkbox(
    id: impl Into<ElementId>,
    label: impl Into<gpui::SharedString>,
    checked: bool,
    palette: Palette,
) -> Stateful<Div> {
    control(id, palette)
        .gap_2()
        .px_2()
        .py_1()
        .text_sm()
        .child(checkmark(checked, palette))
        .child(div().flex_1().min_w_0().child(label.into()))
}

fn checkmark(checked: bool, palette: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(17.))
        .rounded_full()
        .border_1()
        .border_color(if checked {
            palette.accent
        } else {
            palette.quiet
        })
        .when(checked, |el| {
            el.bg(palette.accent)
                .text_color(palette.on_accent)
                .child(icon("check", palette.on_accent, 14.))
        })
}

fn choice(
    id: impl Into<ElementId>,
    label: &'static str,
    selected: bool,
    palette: Palette,
) -> Stateful<Div> {
    button(id, label, palette, false)
        .py_1p5()
        .when(selected, |el| el.bg(palette.select))
}

fn quiet(text: impl Into<gpui::SharedString>, palette: Palette) -> Div {
    div().text_xs().text_color(palette.quiet).child(text.into())
}

fn icon_label(name: &'static str, label: impl Into<gpui::SharedString>, color: Rgba) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(icon(name, color, 13.))
        .child(label.into())
}

struct Tooltip(gpui::SharedString, Palette);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        field_box(self.1)
            .max_w(px(310.))
            .text_sm()
            .text_color(self.1.ink)
            .shadow_md()
            .child(self.0.clone())
    }
}

fn make_tooltip(
    label: impl Into<gpui::SharedString>,
    palette: Palette,
) -> impl Fn(&mut Window, &mut gpui::App) -> gpui::AnyView + 'static {
    let label = label.into();
    move |_, cx| cx.new(|_| Tooltip(label.clone(), palette)).into()
}

fn nav_icon(
    id: impl Into<ElementId>,
    glyph: &'static str,
    label: &'static str,
    palette: Palette,
) -> Stateful<Div> {
    control(id, palette)
        .min_w(px(32.))
        .h(px(32.))
        .justify_center()
        .child(icon(glyph, palette.quiet, 16.))
        .tooltip(make_tooltip(label, palette))
}

fn settings_row(label: &'static str, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_3()
        .border_t_1()
        .border_color(palette.line)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
}

fn file_parts(path: &str) -> (&str, Option<&str>) {
    let file = Path::new(path);
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);
    let parent = file
        .parent()
        .and_then(Path::to_str)
        .filter(|path| !path.is_empty());
    (name, parent)
}

fn absolute_folder(text: &str) -> Option<PathBuf> {
    let path = match text.strip_prefix("~/") {
        Some(relative) => dirs::home_dir()?.join(relative),
        None => PathBuf::from(text),
    };
    path.is_absolute().then_some(path)
}

fn detail(label: &'static str, value: String, palette: Palette) -> Div {
    div()
        .flex()
        .justify_between()
        .gap_2()
        .text_sm()
        .child(div().text_color(palette.quiet).child(label))
        .child(div().text_right().child(value))
}

fn progress(row: &Transfer, height: f32, palette: Palette) -> ProgressTrack {
    ProgressTrack {
        id: row.id,
        height: px(height),
        value: row.progress,
        track: palette.track,
        animate: palette.motion,
        fill: match row.state.as_str() {
            "Seeding" => palette.seed,
            "Downloading" => palette.download,
            "Error" => palette.danger,
            _ => palette.quiet,
        },
    }
}

fn download_pieces(map: &PieceMap, panel_width: gpui::Pixels, palette: Palette) -> Div {
    let colors = [palette.track, palette.download, palette.seed];
    let cells = map.cells.clone();
    let pitch = 7.;
    // The card has 22 px of horizontal padding and borders.
    let columns = ((f32::from(panel_width) - 20.) / pitch).floor().max(1.) as usize;
    let grid_height = cells.len().div_ceil(columns) as f32 * pitch - 2.;
    div()
        .flex()
        .flex_col()
        .gap(px(9.))
        .p(px(10.))
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(palette.line)
        .bg(palette.paper)
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_baseline()
                .justify_between()
                .gap_x_2()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::BOLD)
                        .child("Torrent pieces"),
                )
                .child(
                    quiet(format!("{} each", quantity(map.piece_size.into())), palette).text_xs(),
                ),
        )
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{} of {} pieces", map.have, map.total)),
        )
        .child(
            div().rounded_sm().overflow_hidden().child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let clip = window.content_mask().bounds;
                        if !clip.intersects(&bounds) {
                            return;
                        }
                        let step = bounds.size.width / cells.len() as f32;
                        let mut start = 0;
                        for run in cells.chunk_by(|a, b| a == b) {
                            let origin = point(bounds.left() + step * start as f32, bounds.top());
                            window.paint_quad(fill(
                                Bounds::new(origin, size(step * run.len() as f32, px(6.))),
                                colors[run[0] as usize],
                            ));
                            start += run.len();
                        }
                        for (index, state) in cells.into_iter().enumerate() {
                            let x = bounds.left() + px((index % columns) as f32 * pitch);
                            let y = bounds.top() + px(15. + (index / columns) as f32 * pitch);
                            let cell_bounds = Bounds::new(point(x, y), size(px(5.), px(5.)));
                            if clip.intersects(&cell_bounds) {
                                let mut cell = fill(cell_bounds, colors[state as usize]);
                                cell.corner_radii = px(1.).into();
                                window.paint_quad(cell);
                            }
                        }
                    },
                )
                .w_full()
                .h(px(15. + grid_height)),
            ),
        )
        .child(
            div().flex().flex_wrap().gap_x(px(10.)).children(
                [
                    ("Have", colors[2]),
                    ("Some", colors[1]),
                    ("Missing", colors[0]),
                ]
                .into_iter()
                .map(|(label, color)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(div().size(px(7.)).rounded(px(2.)).bg(color))
                        .child(quiet(label, palette).text_size(px(11.)))
                }),
            ),
        )
        .when(map.total as usize > map.cells.len(), |el| {
            el.child(quiet("Cells show groups of pieces", palette).text_xs())
        })
}

fn download_chart(history: &VecDeque<i32>, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .py_2()
        .border_t_1()
        .border_color(palette.line)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Download activity"),
                )
                .child(quiet("Last minute", palette)),
        )
        .child(rate_plot(
            history.iter().map(|&rate| i64::from(rate)).collect(),
            palette.download,
            palette,
        ))
        .child(rate_axis(palette))
}

fn traffic_rates(transfers: &[Transfer]) -> [i64; 2] {
    transfers.iter().fold([0, 0], |[down, up], row| {
        [
            down + i64::from(row.download_rate),
            up + i64::from(row.upload_rate),
        ]
    })
}

fn rate_plot(rates: Vec<i64>, color: Rgba, palette: Palette) -> impl IntoElement {
    let peak = rates.iter().copied().max().unwrap_or(0).max(1) as f32;
    let first_sample = RATE_SAMPLES - rates.len();
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let mut line = PathBuilder::stroke(px(1.5));
            for (i, rate) in rates.into_iter().enumerate() {
                let x = bounds.left()
                    + px(2.)
                    + (bounds.size.width - px(4.))
                        * ((first_sample + i) as f32 / (RATE_SAMPLES - 1) as f32);
                let y =
                    bounds.bottom() - px(2.) - (bounds.size.height - px(4.)) * (rate as f32 / peak);
                if i == 0 {
                    // Keep the first sample visible before there is a line to draw.
                    line.move_to(point(x - px(1.), y));
                }
                line.line_to(point(x, y));
            }
            if let Ok(path) = line.build() {
                window.paint_path(path, color);
            }
        },
    )
    .w_full()
    .h(px(56.))
    .border_b_1()
    .border_color(palette.line)
}

fn rate_axis(palette: Palette) -> Div {
    div()
        .flex()
        .justify_between()
        .child(quiet("60s ago", palette).text_size(px(11.)))
        .child(quiet("Now", palette).text_size(px(11.)))
}

fn state_color(row: &Transfer, palette: Palette) -> Rgba {
    match row.state.as_str() {
        "Seeding" => palette.seed,
        "Downloading" => palette.download,
        "Paused" | "Waiting for peers" => palette.amber,
        "Fetching metadata" => palette.blue,
        "Error" => palette.danger,
        _ => palette.quiet,
    }
}

fn percent(row: &Transfer) -> String {
    format!("{:.0}%", row.progress.clamp(0., 1.) * 100.)
}

fn quantity(bytes: i64) -> String {
    if bytes < 1024 {
        return format!("{} B", bytes.max(0));
    }
    let (divisor, unit) = if bytes >= 1024 * 1024 * 1024 {
        (1024.0_f64.powi(3), "GiB")
    } else if bytes >= 1024 * 1024 {
        (1024.0_f64.powi(2), "MiB")
    } else {
        (1024., "KiB")
    };
    format!("{:.1} {unit}", bytes as f64 / divisor)
}

fn rate(bytes: i64) -> String {
    format!("{}/s", quantity(bytes))
}

fn row_rate(row: &Transfer) -> (Option<(&'static str, i64)>, Option<String>) {
    if row.download_rate > 0 && !row.paused && row.remaining > 0 {
        let seconds = (row.remaining - 1) / i64::from(row.download_rate) + 1;
        let eta = if seconds >= 3600 {
            format!("{}h {}m", seconds / 3600, seconds / 60 % 60)
        } else if seconds >= 60 {
            format!("{}m {}s", seconds / 60, seconds % 60)
        } else {
            format!("{seconds}s")
        };
        (Some(("arrow-down", row.download_rate.into())), Some(eta))
    } else if row.upload_rate > 0 && !row.paused {
        (Some(("arrow-up", row.upload_rate.into())), None)
    } else {
        (None, None)
    }
}
