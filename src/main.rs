mod about;
mod background;
mod desktop;
mod icons;
mod open_requests;
mod preferences;
mod rss_worker;
mod text_field;
mod web;

use anyhow::{Context as _, Result};
use desktop::Desktop;
use fero::{
    BandwidthSchedule, FeedItem, Library, LibraryLimits, Management, NetworkSettings, Peer,
    PieceMap, PreparedSource, Source, Subscription, TorrentFile, Tracker, Transfer,
    TransferOptions, is_torrent,
};
use gpui::{
    App, AppContext, Application, Bounds, Global, KeyBinding, Menu, MenuItem, WindowBounds,
    WindowHandle, WindowOptions, actions, px, size,
};
use preferences::Preferences;
use std::{
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

actions!(
    fero,
    [
        Quit,
        ShowFero,
        CloseFero,
        About,
        OpenAdd,
        OpenSettings,
        ToggleSidebar,
        PasteMagnet,
        Find,
        Submit,
        Dismiss,
        Tab,
        BackTab
    ]
);

enum Command {
    PreviewMigration(PathBuf),
    ImportMigration(Vec<(usize, PathBuf)>),
    Prepare(u64, Source),
    CancelPreview,
    Add {
        revision: u64,
        magnet: Option<String>,
        destination: PathBuf,
        start_paused: bool,
        skipped: Vec<u32>,
    },
    Pause(i64, bool),
    PauseMany(Vec<i64>, bool),
    Stop(i64),
    Remove(i64, bool),
    RemoveMany(Vec<i64>),
    Reannounce(i64),
    Recheck(i64),
    MoveInQueue(i64, bool),
    FilePriority(i64, u32, u8),
    Manage(ManagementEdit),
    InspectFiles(Option<i64>),
    InspectLive(Option<LiveInspector>),
    SavePreferences(Preferences),
    SaveDefaultFolder(Option<PathBuf>),
    SaveWatchFolders(Vec<PathBuf>),
    PreviewFeed(Subscription),
    RemoveFeed(String),
    ClearSchedule,
    LoadUi,
    SetStartAtLogin(bool),
    DisableWeb,
    WebAction(web::WebAction, web::Reply),
    Quit(Sender<()>),
}

enum ManagementEdit {
    Library(LibraryLimits),
    Feed(String, Subscription),
    Schedule(BandwidthSchedule),
    Web(web::WebSettings),
    Network(NetworkSettings),
    Transfer(i64, TransferOptions),
    Tracker(i64, String, String, u8),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LiveInspector {
    Pieces(i64),
    Peers(i64),
    Trackers(i64),
}

enum LiveDetails {
    Pieces(i64, PieceMap),
    Peers(i64, Result<Vec<Peer>, String>),
    Trackers(i64, Result<Vec<Tracker>, String>),
}

enum Event {
    MigrationPreview(Result<fero::MigrationPreview, String>),
    MigrationInvalidDestinations(Vec<usize>),
    MigrationImported(Result<usize, String>),
    Transfers(Vec<Transfer>, Option<LiveDetails>, Arc<Management>),
    ManagementSaved(Result<(), String>),
    FeedPreview(Result<Vec<FeedItem>, String>),
    StartAtLogin(bool),
    WebState(Option<web::WebSettings>, bool),
    WebPassword(Option<String>),
    Prepared(u64, Result<Vec<TorrentFile>, String>),
    Added(u64, Result<i64, String>),
    Preferences(Preferences),
    DefaultFolder(Result<Option<PathBuf>, String>),
    Files(i64, Result<Vec<TorrentFile>, String>),
    Error(String),
}

fn data_dir() -> Result<PathBuf> {
    std::env::var_os("FERO_DATA_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|path| path.join("Fero")))
        .context("Cannot find an application data folder")
}

#[cfg(test)]
#[test]
fn worker_reports_each_unavailable_import_destination() -> Result<()> {
    let data = tempfile::tempdir()?;
    let (commands, incoming) = mpsc::channel();
    let (events, outgoing) = mpsc::channel();
    let (opened, _) = mpsc::channel();
    commands.send(Command::ImportMigration(vec![
        (0, data.path().to_path_buf()),
        (1, data.path().join("missing")),
        (2, PathBuf::new()),
    ]))?;
    commands.send(Command::Quit(mpsc::channel().0))?;
    worker(
        data.path().to_path_buf(),
        incoming,
        commands,
        &events,
        &opened,
        &AtomicBool::new(false),
    )?;
    let invalid = outgoing.try_iter().find_map(|event| match event {
        Event::MigrationInvalidDestinations(rows) => Some(rows),
        _ => None,
    });
    assert_eq!(invalid, Some(vec![1, 2]));
    Ok(())
}

fn worker(
    data_dir: PathBuf,
    commands: Receiver<Command>,
    sender: Sender<Command>,
    events: &Sender<Event>,
    opened: &Sender<Source>,
    pending_open: &AtomicBool,
) -> Result<()> {
    let db_path = data_dir.join("library.sqlite3");
    let mut library = Library::open(&db_path)?;
    let (settings_db, mut preferences) = match Preferences::open(&db_path) {
        Ok((db, preferences)) => {
            let _ = events.send(Event::Preferences(preferences.clone()));
            (Some(db), preferences)
        }
        Err(error) => {
            let _ = events.send(Event::Error(format!("Load settings: {error:#}")));
            (None, Preferences::default())
        }
    };
    let mut web = web::WebControl::default();
    if let Some(db) = &settings_db
        && let Err(error) = web.load(db, &sender)
    {
        let _ = events.send(Event::Error(format!("Start LAN access: {error:#}")));
    }
    let publish_web = |web: &web::WebControl| {
        let (settings, running) = web.view();
        let _ = events.send(Event::WebState(settings, running));
    };
    publish_web(&web);
    let mut last_checkpoint = Instant::now();
    let mut next_open_scan = Instant::now();
    let mut next_web_retry = Instant::now() + Duration::from_secs(15);
    let open_directory = data_dir.join("open-requests");
    let mut rss = rss_worker::RssWorker::new();
    let mut next_watch_scan = Instant::now();
    let mut prepared: Option<(u64, PreparedSource)> = None;
    let mut migration: Option<fero::MigrationPreview> = None;
    let mut fetching: Option<(u64, String, Instant)> = None;
    let mut inspect_files = None;
    let mut inspect_live = None;
    loop {
        let result = match commands.recv_timeout(Duration::from_millis(500)) {
            Ok(Command::PreviewMigration(profile)) => {
                let preview = library.preview_qbittorrent(&profile);
                migration = preview.as_ref().ok().cloned();
                let _ = events.send(Event::MigrationPreview(
                    preview.map_err(|error| format!("{error:#}")),
                ));
                Ok(())
            }
            Ok(Command::ImportMigration(selected)) => {
                let unavailable: Vec<_> = selected
                    .iter()
                    .filter_map(|(index, path)| {
                        (!path.is_absolute() || !path.is_dir()).then_some(*index)
                    })
                    .collect();
                if !unavailable.is_empty() {
                    let _ = events.send(Event::MigrationInvalidDestinations(unavailable));
                    continue;
                }
                let result = migration
                    .as_ref()
                    .context("Preview a qBittorrent profile first")
                    .and_then(|preview| library.import_qbittorrent(preview, &selected));
                migration = None;
                let _ = events.send(Event::MigrationImported(
                    result.map_err(|error| format!("{error:#}")),
                ));
                Ok(())
            }
            Ok(Command::Prepare(revision, source)) => {
                prepared = None;
                fetching = None;
                let result = library.cancel_metadata().and_then(|()| match &source {
                    Source::Magnet(link) => {
                        library.fetch_metadata(link, &data_dir)?;
                        fetching = Some((revision, link.clone(), Instant::now()));
                        Ok(None)
                    }
                    Source::File(_) => PreparedSource::new(&source).map(Some),
                });
                publish_preview(events, &mut prepared, revision, result);
                Ok(())
            }
            Ok(Command::CancelPreview) => {
                prepared = None;
                fetching = None;
                library.cancel_metadata()
            }
            Ok(Command::Add {
                revision,
                magnet,
                destination,
                start_paused,
                skipped,
            }) => {
                let result = (|| -> Result<i64> {
                    let immediate;
                    let source = if let Some(link) = magnet {
                        immediate = PreparedSource::new(&Source::Magnet(link))?;
                        &immediate
                    } else {
                        &prepared
                            .as_ref()
                            .filter(|(key, _)| *key == revision)
                            .context("The file list is unavailable. Choose the source again.")?
                            .1
                    };
                    library.add_prepared(source, &destination, start_paused, &skipped)
                })();
                if result.is_ok() {
                    prepared = None;
                }
                let _ = events.send(Event::Added(
                    revision,
                    result.map_err(|error| format!("{error:#}")),
                ));
                Ok(())
            }
            Ok(Command::Pause(id, paused)) => library.set_paused(id, paused),
            Ok(Command::PauseMany(ids, paused)) => library.set_paused_many(&ids, paused),
            Ok(Command::Stop(id)) => library.stop(id).context("Stop and save transfer progress"),
            Ok(Command::Remove(id, delete)) => library.remove(id, delete),
            Ok(Command::RemoveMany(ids)) => library.remove_many(&ids),
            Ok(Command::Reannounce(id)) => library.reannounce(id),
            Ok(Command::Recheck(id)) => library.force_recheck(id).context("Recheck transfer"),
            Ok(Command::MoveInQueue(id, up)) => library.move_in_queue(id, up),
            Ok(Command::FilePriority(id, index, priority)) => {
                inspect_files = Some(id);
                library
                    .set_file_priority(id, index, priority)
                    .context("Set file priority")
            }
            Ok(Command::Manage(edit)) => {
                let result = match edit {
                    ManagementEdit::Feed(previous, feed) => library
                        .save_subscription(&previous, feed)
                        .map(|()| rss.rescan()),
                    ManagementEdit::Web(settings) => settings_db
                        .as_ref()
                        .context("Settings storage is unavailable")
                        .and_then(|db| web.save(db, &sender, Some(settings)))
                        .map(|password| {
                            publish_web(&web);
                            let _ = events.send(Event::WebPassword(password));
                        }),
                    ManagementEdit::Schedule(schedule) => {
                        library.set_bandwidth_schedule(Some(schedule))
                    }
                    ManagementEdit::Library(limits) => library.set_library_limits(limits),
                    ManagementEdit::Network(network) => library.save_network_settings(network),
                    ManagementEdit::Transfer(id, options) => {
                        library.set_transfer_options(id, options)
                    }
                    ManagementEdit::Tracker(id, old, url, tier) => {
                        library.edit_tracker(id, &old, &url, tier)
                    }
                };
                let _ = events.send(Event::ManagementSaved(
                    result.map_err(|error| format!("{error:#}")),
                ));
                Ok(())
            }
            Ok(Command::InspectFiles(id)) => {
                inspect_files = id;
                Ok(())
            }
            Ok(Command::InspectLive(target)) => {
                inspect_live = target;
                Ok(())
            }
            Ok(Command::SavePreferences(updated)) => settings_db
                .as_ref()
                .context("Settings storage is unavailable")
                .and_then(|db| updated.save(db))
                .map(|()| {
                    let folder = preferences.default_folder.take();
                    preferences = updated;
                    preferences.default_folder = folder;
                })
                .context("Save settings"),
            Ok(Command::SaveDefaultFolder(folder)) => {
                let result = settings_db
                    .as_ref()
                    .context("Settings storage is unavailable")
                    .and_then(|db| Preferences::save_folder(db, folder.as_deref()));
                let _ = events.send(Event::DefaultFolder(
                    result
                        .map(|()| {
                            preferences.default_folder = folder.clone();
                            next_watch_scan = Instant::now();
                            folder
                        })
                        .map_err(|error| format!("{error:#}")),
                ));
                Ok(())
            }
            Ok(Command::SaveWatchFolders(folders)) => {
                next_watch_scan = Instant::now();
                library
                    .set_watch_folders(folders)
                    .context("Save watch folders")
            }
            Ok(Command::PreviewFeed(feed)) => {
                rss.preview(feed);
                Ok(())
            }
            Ok(Command::RemoveFeed(url)) => library.remove_subscription(&url),
            Ok(Command::ClearSchedule) => library.set_bandwidth_schedule(None),
            Ok(Command::LoadUi) => {
                let _ = events.send(Event::Preferences(preferences.clone()));
                publish_web(&web);
                match background::starts_at_login() {
                    Ok(enabled) => {
                        let _ = events.send(Event::StartAtLogin(enabled));
                        Ok(())
                    }
                    Err(error) => Err(error.context("Read start at login")),
                }
            }
            Ok(Command::SetStartAtLogin(enabled)) => background::set_start_at_login(enabled)
                .map(|()| {
                    let _ = events.send(Event::StartAtLogin(enabled));
                })
                .context("Change start at login"),
            Ok(Command::DisableWeb) => settings_db
                .as_ref()
                .context("Settings storage is unavailable")
                .and_then(|db| web.save(db, &sender, None))
                .map(|_| {
                    publish_web(&web);
                    let _ = events.send(Event::WebPassword(None));
                }),
            Ok(Command::WebAction(action, reply)) => {
                if !reply.is_closed() {
                    let result = web_action(&mut library, &preferences, action);
                    let _ = reply.send(
                        result
                            .map(|rows| (rows, preferences.accent, preferences.reduce_motion))
                            .map_err(|error| format!("{error:#}")),
                    );
                }
                Ok(())
            }
            Ok(Command::Quit(done)) => {
                drop(web);
                let result = library.checkpoint();
                drop(library);
                let _ = done.send(());
                return result;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return library.checkpoint(),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
        };
        if let Err(error) = result {
            let _ = events.send(Event::Error(format!("{error:#}")));
        }
        if let Some(id) = inspect_files {
            match library.files(id) {
                Ok(files) if files.is_empty() => {}
                result => {
                    let _ = events.send(Event::Files(id, result.map_err(|err| format!("{err:#}"))));
                    inspect_files = None;
                }
            }
        }
        if let Some((revision, link, start)) = &fetching {
            let result = if start.elapsed() >= Duration::from_secs(90) {
                Err(anyhow::anyhow!(
                    "No peer supplied metadata. Try again or start the download directly."
                ))
            } else {
                library.poll_metadata(link)
            };
            if !matches!(result, Ok(None)) {
                if result.is_err() {
                    library.cancel_metadata()?;
                }
                publish_preview(events, &mut prepared, *revision, result);
                fetching = None;
            }
        }
        if last_checkpoint.elapsed() >= Duration::from_secs(10) {
            if let Err(error) = library.checkpoint_changed() {
                let _ = events.send(Event::Error(format!("Save transfer progress: {error:#}")));
            }
            last_checkpoint = Instant::now();
        }
        if fetching.is_none() && Instant::now() >= next_watch_scan {
            let folder = settings_db
                .as_ref()
                .and_then(|_| preferences.download_folder());
            library.scan_watch_folders(folder.as_deref(), preferences.start_paused);
            next_watch_scan = Instant::now() + Duration::from_secs(5);
        }
        if Instant::now() >= next_open_scan {
            if let Err(error) = open_requests::receive(&open_directory, opened, pending_open) {
                let _ = events.send(Event::Error(format!("Read open requests: {error:#}")));
            }
            next_open_scan = Instant::now() + Duration::from_secs(1);
        }
        if Instant::now() >= next_web_retry {
            // At login the selected LAN address may arrive after the app starts.
            if let Some(db) = &settings_db
                && web.retry_pending(db, &sender)
            {
                publish_web(&web);
            }
            next_web_retry = Instant::now() + Duration::from_secs(15);
        }
        rss.tick(&mut library, &preferences, events);
        let transfers = library.snapshot()?;
        let details = match inspect_live {
            Some(LiveInspector::Pieces(id))
                if transfers
                    .iter()
                    .any(|row| row.id == id && row.remaining > 0) =>
            {
                library
                    .pieces(id)
                    .ok()
                    .map(|map| LiveDetails::Pieces(id, map))
            }
            Some(LiveInspector::Peers(id)) if transfers.iter().any(|row| row.id == id) => {
                let peers = library.peers(id).map(|mut peers| {
                    peers.sort_by(|a, b| a.endpoint.cmp(&b.endpoint));
                    peers
                });
                Some(LiveDetails::Peers(
                    id,
                    peers.map_err(|error| format!("{error:#}")),
                ))
            }
            Some(LiveInspector::Trackers(id)) if transfers.iter().any(|row| row.id == id) => {
                Some(LiveDetails::Trackers(
                    id,
                    library.trackers(id).map_err(|error| format!("{error:#}")),
                ))
            }
            _ => None,
        };
        if events
            .send(Event::Transfers(transfers, details, library.management()))
            .is_err()
        {
            return library.checkpoint();
        }
    }
}

fn web_action(
    library: &mut Library,
    preferences: &Preferences,
    action: web::WebAction,
) -> Result<Vec<Transfer>> {
    use web::WebAction;
    let source = match action {
        WebAction::List => return library.snapshot(),
        WebAction::Pause(id, paused) => {
            library.set_paused(id, paused)?;
            return Ok(Vec::new());
        }
        WebAction::Magnet(link) => PreparedSource::new(&Source::Magnet(link.trim().into()))?,
        WebAction::Torrent(bytes) => PreparedSource::from_torrent_bytes(bytes)?,
    };
    let folder = preferences
        .download_folder()
        .context("Choose a default save folder in Settings")?;
    library.add_prepared(&source, &folder, preferences.start_paused, &[])?;
    Ok(Vec::new())
}

fn publish_preview(
    events: &Sender<Event>,
    prepared: &mut Option<(u64, PreparedSource)>,
    revision: u64,
    result: Result<Option<PreparedSource>>,
) {
    match result {
        Ok(Some(source)) => {
            let _ = events.send(Event::Prepared(revision, Ok(source.files.clone())));
            *prepared = Some((revision, source));
        }
        Ok(None) => {}
        Err(error) => {
            let _ = events.send(Event::Prepared(revision, Err(format!("{error:#}"))));
        }
    }
}

fn opened_source(argument: std::ffi::OsString) -> Option<Source> {
    if let Some(text) = argument.to_str() {
        if text.starts_with("magnet:") {
            return Some(Source::Magnet(text.to_owned()));
        }
        if text.starts_with("file:") {
            let path = url::Url::parse(text).ok()?.to_file_path().ok()?;
            return is_torrent(&path).then_some(Source::File(path));
        }
    }
    let path = PathBuf::from(argument);
    is_torrent(&path).then_some(Source::File(path))
}

struct Ui {
    commands: Sender<Command>,
    events: Rc<Receiver<Event>>,
    opened: Rc<Receiver<Source>>,
}

impl Global for Ui {}

fn show_window(cx: &mut App) {
    let _ = show_desktop(cx);
}

fn show_desktop(cx: &mut App) -> Option<WindowHandle<Desktop>> {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<Desktop>())
    {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        cx.activate(true);
        return Some(window);
    }
    let ui = cx.global::<Ui>();
    let commands = ui.commands.clone();
    let events = ui.events.clone();
    let opened = ui.opened.clone();
    let _ = commands.send(Command::LoadUi);
    let bounds = Bounds::centered(None, size(px(1060.), px(700.)), cx);
    match cx.open_window(
        WindowOptions {
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Fero".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(420.), px(480.))),
            app_id: Some("dev.fero.desktop".into()),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| Desktop::new(commands, events, opened, window, cx)),
    ) {
        Ok(window) => {
            #[cfg(not(target_os = "macos"))]
            let _ = window.update(cx, |_, window, cx| {
                // These GPUI backends stop their event loop when the last window closes.
                window.on_window_should_close(cx, |window, _| {
                    window.minimize_window();
                    false
                });
            });
            cx.activate(true);
            Some(window)
        }
        Err(error) => {
            eprintln!("Open Fero window: {error:#}");
            None
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Fero: {error:#}");
    }
}

fn run() -> Result<()> {
    let data_dir = data_dir()?;
    let open_directory = data_dir.join("open-requests");
    fs::create_dir_all(&open_directory)?;
    let mut background_start = false;
    let mut sources = Vec::new();
    for argument in std::env::args_os().skip(1) {
        if argument == "--background" {
            background_start = true;
        } else if let Some(source) = opened_source(argument) {
            sources.push(source);
        }
    }
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(data_dir.join("instance.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) if background_start && sources.is_empty() => {
            return Ok(());
        }
        Err(fs::TryLockError::WouldBlock) => {
            return open_requests::forward(&open_directory, &sources);
        }
        Err(error) => return Err(error).context("Lock the Fero library"),
    }
    let (commands, input) = mpsc::channel();
    let (output, events) = mpsc::channel();
    let (open, opened) = mpsc::channel();
    let pending_open = Arc::new(AtomicBool::new(background_start && !sources.is_empty()));
    for source in sources {
        open.send(source)?;
    }
    let sender = commands.clone();
    let worker_opened = open.clone();
    let worker_pending = pending_open.clone();
    let worker = thread::spawn(move || {
        if let Err(error) = worker(
            data_dir,
            input,
            sender,
            &output,
            &worker_opened,
            &worker_pending,
        ) {
            let _ = output.send(Event::Error(format!("{error:#}")));
            eprintln!("Fero: {error:#}");
        }
    });
    let ui_commands = commands.clone();
    let application = Application::new().with_assets(icons::Icons);
    let url_pending = pending_open.clone();
    application.on_open_urls(move |urls| {
        for url in urls {
            if let Some(source) = opened_source(url.into()) {
                let _ = open.send(source);
                url_pending.store(true, Ordering::Release);
            }
        }
    });
    application.on_reopen(show_window);
    application.run(move |cx: &mut App| {
        let quit_commands = ui_commands.clone();
        cx.on_app_quit(move |_| {
            // AppKit can terminate the process without returning from Application::run.
            let (done, saved) = mpsc::channel();
            if quit_commands.send(Command::Quit(done)).is_ok() {
                let _ = saved.recv();
            }
            std::future::ready(())
        })
        .detach();
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &ShowFero, cx| show_window(cx));
        cx.on_action(|_: &About, cx| {
            if let Some(window) = show_desktop(cx) {
                let _ = window.update(cx, Desktop::show_about);
            }
        });
        cx.on_action(|_: &CloseFero, cx| {
            if let Some(window) = cx
                .windows()
                .into_iter()
                .find(|window| window.downcast::<Desktop>().is_some())
            {
                let _ = window.update(cx, |_, window, _| {
                    if !cfg!(target_os = "macos") {
                        window.minimize_window();
                    } else {
                        window.remove_window();
                    }
                });
            }
        });
        cx.set_menus(vec![
            Menu {
                name: "Fero".into(),
                items: vec![
                    MenuItem::action("Open Fero", ShowFero),
                    MenuItem::action("About Fero", About),
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::separator(),
                    MenuItem::action("Quit Fero", Quit),
                ],
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Add Torrent", OpenAdd),
                    MenuItem::action("Close Window", CloseFero),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![MenuItem::action("Toggle Sidebar", ToggleSidebar)],
            },
        ]);
        cx.set_dock_menu(vec![
            MenuItem::action("Open Fero", ShowFero),
            MenuItem::action("Quit Fero", Quit),
        ]);
        bind_keys(cx);
        cx.set_global(Ui {
            commands: ui_commands,
            events: Rc::new(events),
            opened: Rc::new(opened),
        });
        if let Err(error) = background::Tray::install(cx) {
            eprintln!("Fero tray: {error:#}");
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                let commands = &cx.global::<Ui>().commands;
                let _ = commands.send(Command::CancelPreview);
                let _ = commands.send(Command::InspectFiles(None));
                let _ = commands.send(Command::InspectLive(None));
            }
        })
        .detach();
        if !background_start || !cfg!(target_os = "macos") {
            show_window(cx);
            if background_start && let Some(window) = cx.windows().first().copied() {
                let _ = window.update(cx, |_, window, _| window.minimize_window());
            }
        }
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(125))
                    .await;
                if cx
                    .update(|cx| {
                        if pending_open.swap(false, Ordering::AcqRel) {
                            show_window(cx);
                        }
                        if cx.windows().is_empty() {
                            // The worker continues to publish snapshots while no desktop view exists.
                            for _ in cx.global::<Ui>().events.try_iter() {}
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    });
    let _ = commands.send(Command::Quit(mpsc::channel().0));
    let _ = worker.join();
    Ok(())
}

fn bind_keys(cx: &mut App) {
    text_field::bind(cx);
    let mac = cfg!(target_os = "macos");
    cx.bind_keys([
        KeyBinding::new(if mac { "cmd-q" } else { "ctrl-q" }, Quit, None),
        KeyBinding::new(if mac { "cmd-w" } else { "ctrl-w" }, CloseFero, None),
        KeyBinding::new(if mac { "cmd-n" } else { "ctrl-n" }, OpenAdd, None),
        KeyBinding::new(if mac { "cmd-," } else { "ctrl-," }, OpenSettings, None),
        KeyBinding::new(
            if mac { "cmd-shift-s" } else { "ctrl-shift-s" },
            ToggleSidebar,
            None,
        ),
        KeyBinding::new(
            if mac { "cmd-f" } else { "ctrl-f" },
            Find,
            Some("FeroLibrary"),
        ),
        KeyBinding::new(
            if mac { "cmd-v" } else { "ctrl-v" },
            PasteMagnet,
            Some("FeroLibrary"),
        ),
        KeyBinding::new("enter", Submit, Some("FeroMagnet")),
        KeyBinding::new("enter", Submit, Some("FeroManagement")),
        KeyBinding::new(
            if mac { "cmd-enter" } else { "ctrl-enter" },
            Submit,
            Some("FeroAdd"),
        ),
        KeyBinding::new("escape", Dismiss, None),
        KeyBinding::new("tab", Tab, None),
        KeyBinding::new("shift-tab", BackTab, None),
    ]);
}
