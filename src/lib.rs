mod automation;
#[cfg_attr(not(test), allow(dead_code))]
mod bridge;
mod management;

use anyhow::{Context, Result, ensure};
use bridge::ffi;
use rusqlite::OpenFlags;
use rusqlite::{Connection, params};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

pub use automation::{BandwidthSchedule, FeedItem, Subscription};
pub use bridge::ffi::{Peer, PieceMap, TorrentFile, Tracker, Transfer};
pub use management::{Bandwidth, LibraryLimits, Management, NetworkSettings, TransferOptions};

#[derive(Clone, Debug)]
pub enum Source {
    Magnet(String),
    File(PathBuf),
}

pub fn is_torrent(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
}

pub struct PreparedSource {
    bytes: Vec<u8>,
    magnet: bool,
    resume: Vec<u8>,
    pub files: Vec<TorrentFile>,
}

#[derive(Clone, Debug)]
pub struct MigrationItem {
    pub source: PathBuf,
    pub torrent: PathBuf,
    pub destination: PathBuf,
    pub category: String,
    pub tags: String,
    pub duplicate: bool,
    pub missing: bool,
    pub resume: Vec<u8>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct MigrationPreview {
    pub items: Vec<MigrationItem>,
}

fn hex_hash(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn extract_torrent_from_resume(resume: &[u8]) -> Result<Vec<u8>> {
    let mut offset = 0;
    let mut depth = 0;
    let mut info = None;
    while offset < resume.len() {
        match resume[offset] {
            b'd' | b'l' => {
                depth += 1;
                offset += 1;
            }
            b'e' => {
                depth -= 1;
                offset += 1;
                if depth == 0 {
                    break;
                }
            }
            b'i' => {
                offset += 1;
                while offset < resume.len() && resume[offset] != b'e' {
                    offset += 1;
                }
                offset += usize::from(offset < resume.len());
            }
            b'0'..=b'9' => {
                let start = offset;
                while offset < resume.len() && resume[offset].is_ascii_digit() {
                    offset += 1;
                }
                ensure!(
                    offset < resume.len() && resume[offset] == b':',
                    "Invalid resume data string"
                );
                let len: usize = std::str::from_utf8(&resume[start..offset])?.parse()?;
                offset += 1;
                ensure!(offset + len <= resume.len(), "Truncated resume data string");
                let value = &resume[offset..offset + len];
                offset += len;
                // Locate the info key in the top-level dictionary.
                if (depth == 1 || depth == 2 && resume.first() == Some(&b'd')) && value == b"info" {
                    let start = offset;
                    let first = *resume.get(start).context("Missing info dictionary")?;
                    ensure!(first == b'd', "Invalid info dictionary");
                    let mut nested = 0_i32;
                    while offset < resume.len() {
                        match resume[offset] {
                            b'd' | b'l' => {
                                nested += 1;
                                offset += 1;
                            }
                            b'e' => {
                                nested -= 1;
                                offset += 1;
                                if nested == 0 {
                                    break;
                                }
                            }
                            b'i' => {
                                offset += 1;
                                while offset < resume.len() && resume[offset] != b'e' {
                                    offset += 1;
                                }
                                offset += usize::from(offset < resume.len());
                            }
                            b'0'..=b'9' => {
                                let nstart = offset;
                                while offset < resume.len() && resume[offset].is_ascii_digit() {
                                    offset += 1;
                                }
                                let n: usize =
                                    std::str::from_utf8(&resume[nstart..offset])?.parse()?;
                                offset += 1;
                                ensure!(offset + n <= resume.len(), "Truncated resume data");
                                offset += n;
                            }
                            _ => anyhow::bail!("Invalid resume data token"),
                        }
                    }
                    info = Some(resume[start..offset].to_vec());
                    break;
                }
            }
            _ => anyhow::bail!("Invalid resume data token"),
        }
    }
    let info = info.context("Resume data has no embedded torrent metadata")?;
    let mut torrent = b"d4:info".to_vec();
    torrent.extend(info);
    torrent.push(b'e');
    Ok(torrent)
}

fn resume_matches(resume: &[u8], expected: &ffi::TorrentHashes) -> bool {
    if let Ok(saved) = ffi::torrent_hashes(resume, true) {
        return (!expected.v1.is_empty() && saved.v1 == expected.v1)
            || (!expected.v2.is_empty() && saved.v2 == expected.v2);
    }
    extract_torrent_from_resume(resume)
        .and_then(|bytes| ffi::torrent_hashes(&bytes, false).map_err(Into::into))
        .is_ok_and(|saved| {
            (!expected.v1.is_empty() && saved.v1 == expected.v1)
                || (!expected.v2.is_empty() && saved.v2 == expected.v2)
        })
}

impl PreparedSource {
    pub fn new(source: &Source) -> Result<Self> {
        let (bytes, magnet) = match source {
            Source::Magnet(link) => {
                ensure!(
                    link.starts_with("magnet:?"),
                    "Enter a magnet link beginning with magnet:?"
                );
                (link.as_bytes().to_vec(), true)
            }
            Source::File(path) => (
                fs::read(path).with_context(|| format!("Read {}", path.display()))?,
                false,
            ),
        };
        Self::from_bytes(bytes, magnet)
    }

    pub fn from_torrent_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::from_bytes(bytes, false)
    }

    fn from_bytes(bytes: Vec<u8>, magnet: bool) -> Result<Self> {
        let files = if magnet {
            Vec::new()
        } else {
            ffi::inspect_torrent(&bytes)?
        };
        Ok(Self {
            bytes,
            magnet,
            files,
            resume: Vec::new(),
        })
    }
}

pub struct FeedRequest(cxx::UniquePtr<ffi::HttpRequest>);

impl FeedRequest {
    pub fn poll(&mut self) -> Result<Option<Vec<u8>>> {
        let response = self.0.pin_mut().poll()?;
        Ok(response.ready.then_some(response.bytes))
    }
}

pub struct Library {
    db: Connection,
    engine: cxx::UniquePtr<ffi::Engine>,
    management: Arc<Management>,
    paused: HashSet<i64>,
    next_route_check: Instant,
    applied_bandwidth: Bandwidth,
    watched_files: HashMap<PathBuf, (SystemTime, u64)>,
}

fn proxy_problem(proxy: SocketAddr) -> Option<String> {
    let handshake = || -> std::io::Result<bool> {
        let timeout = Some(Duration::from_millis(250));
        let mut stream = TcpStream::connect_timeout(&proxy, Duration::from_millis(250))?;
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        stream.write_all(&[5, 1, 0])?;
        let mut reply = [0; 2];
        stream.read_exact(&mut reply)?;
        Ok(reply == [5, 0])
    };
    match handshake() {
        Ok(true) => None,
        Ok(false) => Some(format!(
            "SOCKS5 proxy {proxy} rejected unauthenticated connections"
        )),
        Err(_) => Some(format!("SOCKS5 proxy {proxy} is unavailable")),
    }
}

impl Library {
    pub fn preview_qbittorrent(&self, profile: &Path) -> Result<MigrationPreview> {
        let profile = profile.canonicalize()?;
        ensure!(profile.is_dir(), "Choose a qBittorrent profile folder");
        let backup_dir = if profile.join("BT_backup").is_dir() {
            profile.join("BT_backup")
        } else {
            profile.to_path_buf()
        };
        let database = [profile.join("torrents.db"), backup_dir.join("torrents.db")]
            .into_iter()
            .find(|path| path.is_file());
        let root = backup_dir;
        let Some(database) = database else {
            let mut items = Vec::new();
            for entry in fs::read_dir(&root)? {
                let path = entry?.path();
                let is_resume = path.extension().is_some_and(|ext| ext == "fastresume");
                if !is_torrent(&path) && !is_resume {
                    continue;
                }
                let resume = if is_resume {
                    fs::read(&path)?
                } else {
                    Vec::new()
                };
                let (bytes, resume) = if is_resume {
                    (extract_torrent_from_resume(&resume)?, resume)
                } else {
                    let stem = path.file_stem().unwrap_or_default();
                    let fastresume = root.join(format!("{}.fastresume", stem.to_string_lossy()));
                    let saved = fs::read(&fastresume).unwrap_or_default();
                    (fs::read(&path)?, saved)
                };
                let hashes = ffi::torrent_hashes(&bytes, false)?;
                ensure!(
                    !hashes.v1.is_empty() || !hashes.v2.is_empty(),
                    "Invalid torrent metadata: {}",
                    path.display()
                );
                let mut duplicate = false;
                for hash in [&hashes.v1, &hashes.v2]
                    .into_iter()
                    .filter(|hash| !hash.is_empty())
                {
                    duplicate |= self.db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM processed_torrents WHERE hash=?1)",
                        [hash],
                        |row| row.get::<_, bool>(0),
                    )?;
                }
                let destination = root.clone();
                let resume = if resume_matches(&resume, &hashes) {
                    resume
                } else {
                    Vec::new()
                };
                let missing = PreparedSource::from_torrent_bytes(bytes.clone())?
                    .files
                    .iter()
                    .any(|file| !destination.join(&file.path).exists());
                items.push(MigrationItem {
                    source: profile.clone(),
                    torrent: path,
                    destination,
                    category: String::new(),
                    tags: String::new(),
                    duplicate,
                    missing,
                    resume,
                    bytes,
                });
            }
            ensure!(
                !items.is_empty(),
                "No torrents.db or .torrent backups found in the selected profile"
            );
            return Ok(MigrationPreview { items });
        };
        let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let columns = {
            let mut statement = db.prepare("PRAGMA table_info(torrents)")?;
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<HashSet<_>>>()?
        };
        for required in ["torrent_id", "target_save_path", "libtorrent_resume_data"] {
            ensure!(
                columns.contains(required),
                "Unsupported qBittorrent torrents.db: missing {required}"
            );
        }
        let category = if columns.contains("category") {
            "category"
        } else {
            "''"
        };
        let tags = if columns.contains("tags") {
            "tags"
        } else {
            "''"
        };
        let metadata = if columns.contains("metadata") {
            "metadata"
        } else {
            "NULL"
        };
        let mut statement = db.prepare(&format!(
            "SELECT torrent_id, target_save_path, {category}, {tags}, libtorrent_resume_data, {metadata} FROM torrents"
        ))?;
        let mut rows = statement.query([])?;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            let id: rusqlite::types::Value = row.get(0)?;
            let hash = match &id {
                rusqlite::types::Value::Blob(bytes) if bytes.len() == 20 || bytes.len() == 32 => {
                    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
                }
                rusqlite::types::Value::Text(text) => text.clone(),
                rusqlite::types::Value::Blob(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                _ => anyhow::bail!("Unsupported torrent_id in qBittorrent database"),
            };
            let torrent = root.join(format!("{hash}.torrent"));
            let destination = PathBuf::from(row.get::<_, String>(1)?);
            let item_category: String = row.get(2)?;
            let item_tags: String = row.get(3)?;
            let resume: Vec<u8> = row.get(4)?;
            let bytes = fs::read(&torrent).unwrap_or_default();
            let metadata: Option<Vec<u8>> = row.get(5)?;
            let bytes = if bytes.is_empty() {
                metadata
                    .and_then(|data| extract_torrent_from_resume(&data).ok())
                    .or_else(|| extract_torrent_from_resume(&resume).ok())
                    .unwrap_or_default()
            } else {
                bytes
            };
            ensure!(
                bytes.is_empty()
                    || matches!(&id, rusqlite::types::Value::Blob(raw) if *raw == ffi::torrent_hashes(&bytes, false)?.v1 || *raw == ffi::torrent_hashes(&bytes, false)?.v2)
                    || matches!(&id, rusqlite::types::Value::Text(text) if *text == hex_hash(&ffi::torrent_hashes(&bytes, false)?.v1) || *text == hex_hash(&ffi::torrent_hashes(&bytes, false)?.v2)),
                "Backup torrent hash does not match database record: {}",
                torrent.display()
            );
            let duplicate = if bytes.is_empty() {
                false
            } else {
                let hashes = ffi::torrent_hashes(&bytes, false)?;
                let mut exists = false;
                for hash in [hashes.v1, hashes.v2]
                    .into_iter()
                    .filter(|hash| !hash.is_empty())
                {
                    exists |= self.db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM processed_torrents WHERE hash=?1)",
                        [hash],
                        |row| row.get::<_, bool>(0),
                    )?;
                }
                exists
            };
            let resume_valid =
                !resume.is_empty() && resume_matches(&resume, &ffi::torrent_hashes(&bytes, false)?);
            let missing = !destination.is_dir()
                || (!bytes.is_empty()
                    && PreparedSource::from_torrent_bytes(bytes.clone())?
                        .files
                        .iter()
                        .any(|file| !destination.join(&file.path).exists()));
            items.push(MigrationItem {
                source: profile.clone(),
                torrent,
                destination,
                category: item_category,
                tags: item_tags,
                duplicate,
                missing,
                resume: if resume_valid { resume } else { Vec::new() },
                bytes,
            });
        }
        Ok(MigrationPreview { items })
    }

    pub fn import_qbittorrent(
        &mut self,
        preview: &MigrationPreview,
        selected: &[(usize, PathBuf)],
    ) -> Result<usize> {
        let mut imported = 0;
        let mut selected_hashes = HashSet::new();
        for (index, destination) in selected {
            let item = preview
                .items
                .get(*index)
                .context("Invalid migration selection")?;
            ensure!(!item.duplicate, "Selected transfer is already present");
            let hashes = ffi::torrent_hashes(&item.bytes, false)?;
            ensure!(
                [hashes.v1.clone(), hashes.v2.clone()]
                    .into_iter()
                    .filter(|hash| !hash.is_empty())
                    .all(|hash| selected_hashes.insert(hash)),
                "The selected list contains duplicate torrents"
            );
            ensure!(
                !item.bytes.is_empty(),
                "Missing torrent metadata for {}",
                item.torrent.display()
            );
            download_folder(destination)?;
            let freshly_checked = self.preview_qbittorrent(&item.source)?;
            let current = freshly_checked
                .items
                .iter()
                .find(|candidate| candidate.torrent == item.torrent)
                .context("Torrent was removed from the source profile after preview")?;
            ensure!(
                !current.duplicate && current.bytes == item.bytes && current.resume == item.resume,
                "The source torrent changed since preview; preview the profile again"
            );
            ensure!(
                !item.bytes.is_empty(),
                "Missing .torrent file: {}",
                item.torrent.display()
            );
            let source = PreparedSource {
                bytes: item.bytes.clone(),
                magnet: false,
                resume: item.resume.clone(),
                files: Vec::new(),
            };
            if let Some(id) = self.add_transfer(
                &source,
                destination,
                true,
                &[],
                Some(hashes),
                None,
                !item.resume.is_empty(),
            )? {
                self.set_transfer_options(
                    id,
                    TransferOptions {
                        category: item.category.clone(),
                        tags: item.tags.clone(),
                        ..TransferOptions::default()
                    },
                )?;
                self.engine.pin_mut().force_recheck(id)?;
                imported += 1;
            }
        }
        Ok(imported)
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_network(path, false)
    }

    fn open_with_network(path: &Path, local_only: bool) -> Result<Self> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS transfers (
                id INTEGER PRIMARY KEY,
                source BLOB NOT NULL,
                magnet INTEGER NOT NULL,
                destination TEXT NOT NULL,
                resume BLOB NOT NULL DEFAULT X'',
                paused INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS processed_torrents (hash BLOB PRIMARY KEY NOT NULL);",
        )?;
        let mut management = Management::load(&db)?;
        let network = management.network.clone();
        let (proxy_host, proxy_port) = network
            .socks5
            .map(|proxy| (proxy.ip().to_string(), proxy.port()))
            .unwrap_or_default();
        management.network_problem = if network.vpn_interface.is_empty() {
            network.socks5.and_then(proxy_problem)
        } else {
            Some(format!(
                "Waiting to bind VPN interface {}",
                network.vpn_interface
            ))
        };
        let mut engine = ffi::new_engine(&ffi::EngineOptions {
            local_only,
            listen_port: network.listen_port,
            connections: network.connections,
            proxy_host,
            proxy_port,
            vpn_interface: network.vpn_interface.clone(),
            vpn_dns: network
                .vpn_dns
                .map(|dns| dns.to_string())
                .unwrap_or_default(),
            network_ready: management.network_problem.is_none(),
        })?;
        let bandwidth = automation::scheduled_bandwidth(
            management.limits.bandwidth,
            management.bandwidth_schedule,
        );
        management.effective_bandwidth = bandwidth;
        engine
            .pin_mut()
            .set_limits(0, bandwidth.download, bandwidth.upload)?;
        let mut paused = HashSet::new();
        {
            let mut query = db.prepare(
                "SELECT id, source, magnet, destination, resume, paused FROM transfers ORDER BY id",
            )?;
            let mut rows = query.query([])?;
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                let source: Vec<u8> = row.get(1)?;
                let destination: String = row.get(3)?;
                let resume: Vec<u8> = row.get(4)?;
                if row.get(5)? {
                    paused.insert(id);
                }
                engine
                    .pin_mut()
                    .add(
                        id,
                        &source,
                        &destination,
                        &resume,
                        &[],
                        ffi::AddOptions {
                            magnet: row.get(2)?,
                            paused: true,
                            from_preview: false,
                            skip_existing: false,
                            require_resume: false,
                        },
                    )
                    .with_context(|| format!("Restore transfer {id}"))?;
                if let Some(options) = management.options.get(&id) {
                    let bandwidth = options.bandwidth;
                    engine
                        .pin_mut()
                        .set_limits(id, bandwidth.download, bandwidth.upload)?;
                }
            }
        }
        let mut library = Self {
            db,
            engine,
            management: Arc::new(management),
            paused,
            next_route_check: Instant::now(),
            applied_bandwidth: bandwidth,
            watched_files: HashMap::new(),
        };
        library.snapshot()?;
        Ok(library)
    }

    pub fn add(&mut self, source: Source, destination: &Path) -> Result<i64> {
        self.add_prepared(&PreparedSource::new(&source)?, destination, false, &[])
    }

    pub fn add_prepared(
        &mut self,
        source: &PreparedSource,
        destination: &Path,
        paused: bool,
        skipped_files: &[u32],
    ) -> Result<i64> {
        self.add_transfer(
            source,
            destination,
            paused,
            skipped_files,
            None,
            None,
            false,
        )?
        .context("Transfer already exists")
    }

    fn add_transfer(
        &mut self,
        source: &PreparedSource,
        destination: &Path,
        paused: bool,
        skipped_files: &[u32],
        hashes: Option<ffi::TorrentHashes>,
        feed_item: Option<(&str, &str)>,
        require_resume: bool,
    ) -> Result<Option<i64>> {
        let destination = download_folder(destination)?;
        ensure!(
            skipped_files.is_empty()
                || source
                    .files
                    .iter()
                    .any(|file| !skipped_files.contains(&file.index)),
            "Select at least one file"
        );
        let transaction = self.db.transaction()?;
        if let Some((feed, item)) = feed_item
            && transaction.execute(
                "INSERT OR IGNORE INTO processed_feed_items VALUES (?1, ?2)",
                params![feed, item],
            )? == 0
        {
            return Ok(None);
        }
        let automated = hashes.is_some();
        let mark_watched_hash = hashes.is_some();
        let feed_duplicate = if let Some((_, _)) = feed_item {
            let mut found = false;
            if let Some(hashes) = &hashes {
                let mut statement = transaction.prepare("SELECT source, magnet FROM transfers")?;
                let mut rows = statement.query([])?;
                while let Some(row) = rows.next()? {
                    let source: Vec<u8> = row.get(0)?;
                    let magnet: bool = row.get(1)?;
                    let existing = ffi::torrent_hashes(&source, magnet)?;
                    found |= (!hashes.v1.is_empty() && hashes.v1 == existing.v1)
                        || (!hashes.v2.is_empty() && hashes.v2 == existing.v2);
                    if found {
                        break;
                    }
                }
            }
            found
        } else {
            false
        };
        if feed_duplicate {
            transaction.commit()?;
            return Ok(None);
        }
        let mut hashes_to_record = Vec::new();
        if let Some(hashes) = hashes {
            let automated_duplicate = if automated {
                let mut found = false;
                for hash in [&hashes.v1, &hashes.v2]
                    .into_iter()
                    .filter(|hash| !hash.is_empty())
                {
                    found |= transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM processed_torrents WHERE hash=?1)",
                        [hash],
                        |row| row.get::<_, bool>(0),
                    )?;
                }
                found
            } else {
                false
            };
            if automated_duplicate {
                transaction.commit()?;
                return Ok(None);
            }
            hashes_to_record = if mark_watched_hash {
                [hashes.v1, hashes.v2]
                    .into_iter()
                    .filter(|hash| !hash.is_empty())
                    .collect()
            } else {
                Vec::new()
            };
        }
        transaction.execute(
            "INSERT INTO transfers (source, magnet, destination, paused) VALUES (?1, ?2, ?3, ?4)",
            params![source.bytes, source.magnet, destination, paused],
        )?;
        let id = transaction.last_insert_rowid();
        let mut engine = self.engine.pin_mut();
        let added = engine.as_mut().add(
            id,
            &source.bytes,
            destination,
            &source.resume,
            skipped_files,
            ffi::AddOptions {
                magnet: source.magnet,
                paused: true,
                from_preview: !source.resume.is_empty(),
                skip_existing: automated,
                require_resume,
            },
        )?;
        if !added {
            transaction.execute("DELETE FROM transfers WHERE id = ?1", [id])?;
            transaction.commit()?;
            return Ok(None);
        }
        let save = (|| -> Result<()> {
            for hash in hashes_to_record {
                transaction.execute(
                    "INSERT OR IGNORE INTO processed_torrents (hash) VALUES (?1)",
                    [hash],
                )?;
            }
            if require_resume {
                transaction.execute(
                    "UPDATE transfers SET resume=?1 WHERE id=?2",
                    params![source.resume, id],
                )?;
            } else if !skipped_files.is_empty() || !source.resume.is_empty() {
                let resume = engine.as_mut().checkpoint(id)?;
                transaction.execute(
                    "UPDATE transfers SET resume = ?1 WHERE id = ?2",
                    params![resume, id],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = save {
            engine.as_mut().remove(id, false)?;
            return Err(error);
        }
        Arc::make_mut(&mut self.management).queue.push(id);
        if paused {
            self.paused.insert(id);
        } else if !automated {
            self.snapshot()?;
        }
        Ok(Some(id))
    }

    pub fn set_watch_folders(&mut self, mut folders: Vec<PathBuf>) -> Result<()> {
        folders.sort();
        folders.dedup();
        let transaction = self.db.transaction()?;
        transaction.execute("DELETE FROM watch_folders", [])?;
        for folder in &folders {
            ensure!(
                folder.is_absolute() && folder.is_dir(),
                "Choose an existing watch folder"
            );
            let path = folder
                .to_str()
                .context("Watch folder must be valid UTF-8")?;
            transaction.execute("INSERT INTO watch_folders (path) VALUES (?1)", [path])?;
        }
        transaction.commit()?;
        let management = Arc::make_mut(&mut self.management);
        management.watch_folders = folders;
        management.watch_problem = None;
        Ok(())
    }

    pub fn scan_watch_folders(&mut self, destination: Option<&Path>, paused: bool) {
        let folders = self.management.watch_folders.clone();
        let mut problem = None;
        let previous = std::mem::take(&mut self.watched_files);
        for folder in folders {
            let result = (|| -> Result<()> {
                let destination = destination.context("Choose a default save folder")?;
                download_folder(destination)?;
                for entry in fs::read_dir(&folder)? {
                    let result = (|| -> Result<()> {
                        let entry = entry?;
                        let path = entry.path();
                        if !is_torrent(&path) || !entry.file_type()?.is_file() {
                            return Ok(());
                        }
                        let metadata = entry.metadata()?;
                        let stamp = (metadata.modified()?, metadata.len());
                        if previous.get(&path) != Some(&stamp) {
                            self.import_watched_torrent(&path, destination, paused)
                                .with_context(|| format!("Import {}", path.display()))?;
                        }
                        self.watched_files.insert(path, stamp);
                        Ok(())
                    })();
                    if let Err(error) = result {
                        problem.get_or_insert_with(|| format!("{error:#}"));
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                problem.get_or_insert_with(|| format!("Watch {}: {error:#}", folder.display()));
            }
        }
        let hashes = self
            .management
            .watch_folders
            .iter()
            .filter_map(|folder| fs::read_dir(folder).ok())
            .flatten()
            .flatten()
            .filter(|entry| is_torrent(&entry.path()))
            .filter_map(|entry| fs::read(entry.path()).ok())
            .filter_map(|bytes| ffi::torrent_hashes(&bytes, false).ok())
            .flat_map(|hashes| [hashes.v1, hashes.v2])
            .filter(|hash| !hash.is_empty())
            .collect::<Vec<_>>();
        if problem.is_none() {
            let transaction = self.db.transaction();
            if let Ok(transaction) = transaction {
                let save = (|| -> Result<()> {
                    for hash in hashes {
                        transaction.execute(
                            "INSERT OR IGNORE INTO processed_torrents(hash) VALUES(?1)",
                            [hash],
                        )?;
                    }
                    transaction.commit()?;
                    Ok(())
                })();
                if let Err(error) = save {
                    problem = Some(format!("Record watched torrents: {error:#}"));
                }
            }
        }
        if problem != self.management.watch_problem {
            Arc::make_mut(&mut self.management).watch_problem = problem;
        }
    }

    fn import_watched_torrent(
        &mut self,
        path: &Path,
        destination: &Path,
        paused: bool,
    ) -> Result<()> {
        // Match libtorrent's default metainfo size limit.
        const MAX_TORRENT_BYTES: u64 = 10_000_000;
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_TORRENT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_TORRENT_BYTES,
            "Torrent file exceeds 10 MB"
        );
        let hashes = ffi::torrent_hashes(&bytes, false)?;
        ensure!(
            !hashes.v1.is_empty() || !hashes.v2.is_empty(),
            "Missing torrent info hash"
        );
        let source = PreparedSource {
            bytes,
            magnet: false,
            resume: Vec::new(),
            files: Vec::new(),
        };
        // Watched-folder processing is idempotent via the processed hash table.
        // Removing the transfer deliberately leaves that table entry, so a copied
        // torrent cannot be re-added by a future scan.
        self.add_transfer(&source, destination, paused, &[], Some(hashes), None, false)?;
        Ok(())
    }

    pub fn fetch_metadata(&mut self, magnet: &str, destination: &Path) -> Result<()> {
        self.check_route()?;
        if let Some(problem) = &self.management.network_problem {
            anyhow::bail!("{problem}");
        }
        self.cancel_metadata()?;
        Ok(self
            .engine
            .pin_mut()
            .fetch_metadata(magnet, download_folder(destination)?)?)
    }

    pub fn fetch_feed(&mut self, url: &str) -> Result<Vec<u8>> {
        let mut request = self.request_feed(url)?;
        loop {
            if let Some(bytes) = request.poll()? {
                return Ok(bytes);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn request_feed(&mut self, url: &str) -> Result<FeedRequest> {
        let url = automation::feed_url(url)?;
        self.check_route()?;
        if let Some(problem) = &self.management.network_problem {
            anyhow::bail!("{problem}");
        }
        Ok(FeedRequest(self.engine.request_feed(url.as_str())?))
    }

    pub fn poll_metadata(&mut self, magnet: &str) -> Result<Option<PreparedSource>> {
        self.check_route()?;
        if let Some(problem) = &self.management.network_problem {
            anyhow::bail!("{problem}");
        }
        let files = self.engine.metadata()?;
        if files.is_empty() {
            return Ok(None);
        }
        let resume = self.engine.pin_mut().checkpoint(0)?;
        self.cancel_metadata()?;
        Ok(Some(PreparedSource {
            bytes: magnet.as_bytes().to_vec(),
            magnet: true,
            resume,
            files,
        }))
    }

    pub fn cancel_metadata(&mut self) -> Result<()> {
        Ok(self.engine.pin_mut().remove(0, false)?)
    }

    pub fn remove(&mut self, id: i64, delete_files: bool) -> Result<()> {
        let removed = self
            .db
            .execute("DELETE FROM transfers WHERE id = ?1", [id])?;
        ensure!(removed == 1, "Transfer does not exist");
        let management = Arc::make_mut(&mut self.management);
        management.queue.retain(|&queued| queued != id);
        management.options.remove(&id);
        self.paused.remove(&id);
        self.engine
            .pin_mut()
            .remove(id, delete_files)
            .context(if delete_files {
                "Removed the transfer. Could not confirm deletion of its files"
            } else {
                "Remove transfer"
            })?;
        Ok(())
    }

    pub fn remove_many(&mut self, ids: &[i64]) -> Result<()> {
        let transaction = self.db.transaction()?;
        {
            let mut delete = transaction.prepare_cached("DELETE FROM transfers WHERE id = ?1")?;
            for &id in ids {
                ensure!(delete.execute([id])? == 1, "Transfer does not exist");
            }
        }
        transaction.commit()?;
        let removed: HashSet<_> = ids.iter().copied().collect();
        let management = Arc::make_mut(&mut self.management);
        management.queue.retain(|id| !removed.contains(id));
        for &id in ids {
            management.options.remove(&id);
            self.paused.remove(&id);
            self.engine.pin_mut().remove(id, false)?;
        }
        Ok(())
    }

    pub fn set_paused(&mut self, id: i64, paused: bool) -> Result<()> {
        self.set_paused_many(&[id], paused)
    }

    pub fn stop(&mut self, id: i64) -> Result<()> {
        self.save_paused(&[id], true)?;
        self.engine.pin_mut().set_paused(id, true, false)?;
        self.save_resume(id)
    }

    fn save_paused(&mut self, ids: &[i64], paused: bool) -> Result<()> {
        let transaction = self.db.transaction()?;
        {
            let mut update =
                transaction.prepare_cached("UPDATE transfers SET paused = ?1 WHERE id = ?2")?;
            for &id in ids {
                ensure!(
                    update.execute(params![paused, id])? == 1,
                    "Transfer does not exist"
                );
            }
        }
        transaction.commit()?;
        for &id in ids {
            if paused {
                self.paused.insert(id);
            } else {
                self.paused.remove(&id);
            }
        }
        Ok(())
    }

    pub fn set_paused_many(&mut self, ids: &[i64], paused: bool) -> Result<()> {
        self.save_paused(ids, paused)?;
        if !paused {
            for &id in ids {
                self.engine.pin_mut().clear_error(id)?;
            }
        }
        self.snapshot()?;
        Ok(())
    }

    pub fn snapshot(&mut self) -> Result<Vec<Transfer>> {
        self.apply_bandwidth_schedule()?;
        self.check_route()?;
        let network_blocked = self.management.network_problem.is_some();
        let mut by_id: HashMap<_, _> = self
            .engine
            .pin_mut()
            .snapshot()?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
        let limits = self.management.limits;
        let mut downloads = 0;
        let mut seeds = 0;
        let mut rows = Vec::with_capacity(by_id.len());
        for id in &self.management.queue {
            let Some(mut row) = by_id.remove(id) else {
                continue;
            };
            let manually_paused = self.paused.contains(id);
            let ratio = self
                .management
                .options
                .get(id)
                .and_then(|options| options.seed_ratio)
                .unwrap_or(limits.seed_ratio);
            let denominator = if row.downloaded > 0 {
                row.downloaded
            } else {
                row.wanted
            };
            let ratio_reached = row.finished
                && ratio > 0
                && denominator > 0
                && i128::from(row.uploaded) * 100 >= i128::from(denominator) * i128::from(ratio);
            let (active, limit) = if row.finished {
                (&mut seeds, limits.seeds)
            } else {
                (&mut downloads, limits.downloads)
            };
            let queued = limit != 0 && *active >= limit;
            let should_pause = manually_paused || queued || ratio_reached;
            if row.error.is_empty() {
                if !should_pause {
                    *active += 1;
                }
                let checking = row.state == "Checking files";
                if checking && row.paused && queued && !manually_paused && !network_blocked {
                    self.engine.pin_mut().check_until_ready(*id)?;
                } else if should_pause != row.paused && (!checking || !should_pause) {
                    self.engine
                        .pin_mut()
                        .set_paused(*id, should_pause, manually_paused)?;
                }
                row.state = match (manually_paused, ratio_reached, queued, row.state.as_str()) {
                    (false, false, false, _) if network_blocked => "Waiting for network".into(),
                    (_, _, _, "Checking files") if !row.paused => row.state,
                    (true, _, _, _) => "Paused".into(),
                    (_, true, _, _) => "Ratio reached".into(),
                    (_, _, true, _) => "Queued".into(),
                    (_, _, _, "Paused") if row.finished => "Seeding".into(),
                    (_, _, _, "Paused") if !row.has_metadata => "Fetching metadata".into(),
                    (_, _, _, "Paused") if row.peers == 0 => "Waiting for peers".into(),
                    (_, _, _, "Paused") => "Downloading".into(),
                    _ => row.state,
                };
            }
            row.paused = manually_paused;
            rows.push(row);
        }
        Ok(rows)
    }

    pub fn management(&self) -> Arc<Management> {
        self.management.clone()
    }

    pub fn save_network_settings(&mut self, network: NetworkSettings) -> Result<()> {
        network.validate()?;
        Management::save_network(&mut self.db, &network)?;
        let management = Arc::make_mut(&mut self.management);
        let active = &management.active_network;
        if (network.socks5 != active.socks5
            || network.vpn_interface != active.vpn_interface
            || network.vpn_dns != active.vpn_dns)
            && !management.network_restart_required
        {
            self.engine.pin_mut().suspend_for_network_restart()?;
            management.network_problem = Some("Restart Fero to apply the new network route".into());
            management.network_restart_required = true;
        }
        management.network = network;
        Ok(())
    }

    fn check_route(&mut self) -> Result<()> {
        if self.management.network_restart_required || Instant::now() < self.next_route_check {
            return Ok(());
        }
        let active = &self.management.active_network;
        if active.socks5.is_none() && active.vpn_interface.is_empty() {
            return Ok(());
        }
        let problem = if let Some(proxy) = active.socks5 {
            proxy_problem(proxy)
        } else {
            let status = self
                .engine
                .pin_mut()
                .vpn_status(&active.vpn_interface, active.vpn_dns.is_some())?;
            (!status.is_empty()).then_some(status)
        };
        self.next_route_check = Instant::now() + Duration::from_secs(2);
        if problem != self.management.network_problem {
            self.engine.pin_mut().set_network_ready(problem.is_none())?;
            Arc::make_mut(&mut self.management).network_problem = problem;
        }
        Ok(())
    }

    pub fn set_library_limits(&mut self, limits: LibraryLimits) -> Result<()> {
        limits.bandwidth.validate()?;
        Management::save_limits(&self.db, limits)?;
        Arc::make_mut(&mut self.management).limits = limits;
        self.snapshot()?;
        Ok(())
    }

    fn apply_bandwidth_schedule(&mut self) -> Result<()> {
        let bandwidth = automation::scheduled_bandwidth(
            self.management.limits.bandwidth,
            self.management.bandwidth_schedule,
        );
        if bandwidth != self.applied_bandwidth {
            self.engine
                .pin_mut()
                .set_limits(0, bandwidth.download, bandwidth.upload)?;
            self.applied_bandwidth = bandwidth;
            Arc::make_mut(&mut self.management).effective_bandwidth = bandwidth;
        }
        Ok(())
    }

    pub fn set_transfer_options(&mut self, id: i64, mut options: TransferOptions) -> Result<()> {
        options.bandwidth.validate()?;
        options.category = options.category.trim().to_owned();
        let mut seen = HashSet::new();
        options.tags = options
            .tags
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty() && seen.insert(tag.to_lowercase()))
            .collect::<Vec<_>>()
            .join(", ");
        Management::save_options(&self.db, id, &options)?;
        self.engine.pin_mut().set_limits(
            id,
            options.bandwidth.download,
            options.bandwidth.upload,
        )?;
        Arc::make_mut(&mut self.management)
            .options
            .insert(id, options);
        self.snapshot()?;
        Ok(())
    }

    pub fn move_in_queue(&mut self, id: i64, up: bool) -> Result<()> {
        let mut queue = self.management.queue.clone();
        let index = queue
            .iter()
            .position(|&queued| queued == id)
            .context("Transfer does not exist")?;
        let next = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|&i| i < queue.len())
        };
        if let Some(next) = next {
            queue.swap(index, next);
            Management::save_queue(&mut self.db, &queue)?;
            Arc::make_mut(&mut self.management).queue = queue;
            self.snapshot()?;
        }
        Ok(())
    }

    pub fn files(&self, id: i64) -> Result<Vec<TorrentFile>> {
        Ok(self.engine.files(id)?)
    }

    pub fn pieces(&self, id: i64) -> Result<PieceMap> {
        Ok(self.engine.pieces(id)?)
    }

    pub fn peers(&self, id: i64) -> Result<Vec<Peer>> {
        Ok(self.engine.peers(id)?)
    }

    pub fn trackers(&self, id: i64) -> Result<Vec<Tracker>> {
        Ok(self.engine.trackers(id)?)
    }

    pub fn reannounce(&mut self, id: i64) -> Result<()> {
        self.engine
            .pin_mut()
            .reannounce(id)
            .context("Reannounce to trackers")
    }

    pub fn edit_tracker(&mut self, id: i64, old: &str, new: &str, tier: u8) -> Result<()> {
        let new = new.trim();
        ensure!(!new.is_empty() || !old.is_empty(), "Enter a tracker URL");
        if !new.is_empty() {
            ensure!(
                !new.contains(char::is_whitespace),
                "Enter a tracker URL without spaces"
            );
            let url = url::Url::parse(new).context("Enter a valid tracker URL")?;
            ensure!(
                url.host_str().is_some()
                    && matches!(url.scheme(), "http" | "https" | "udp" | "ws" | "wss"),
                "Enter an HTTP, HTTPS, UDP, or WebSocket tracker URL"
            );
        }
        self.engine.pin_mut().edit_tracker(id, old, new, tier)?;
        self.save_resume(id)
    }

    pub fn set_file_priority(&mut self, id: i64, index: u32, priority: u8) -> Result<()> {
        ensure!(priority <= 7, "File priority must be between 0 and 7");
        self.engine
            .pin_mut()
            .set_file_priority(id, index, priority)?;
        self.save_resume(id)
    }

    pub fn force_recheck(&mut self, id: i64) -> Result<()> {
        Ok(self.engine.pin_mut().force_recheck(id)?)
    }

    fn save_resume(&mut self, id: i64) -> Result<()> {
        let resume = self.engine.pin_mut().checkpoint(id)?;
        self.db.execute(
            "UPDATE transfers SET resume = ?1 WHERE id = ?2",
            params![resume, id],
        )?;
        Ok(())
    }

    pub fn checkpoint(&mut self) -> Result<()> {
        self.save_checkpoints(true)
    }

    pub fn checkpoint_changed(&mut self) -> Result<()> {
        self.save_checkpoints(false)
    }

    fn save_checkpoints(&mut self, flush: bool) -> Result<()> {
        let changes = self.engine.pin_mut().checkpoints(flush)?;
        let transaction = self.db.transaction()?;
        {
            let mut update =
                transaction.prepare_cached("UPDATE transfers SET resume = ?1 WHERE id = ?2")?;
            for change in changes {
                update.execute(params![change.bytes, change.id])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

fn download_folder(destination: &Path) -> Result<&str> {
    ensure!(destination.is_dir(), "Select an existing download folder");
    destination
        .to_str()
        .context("Download folder must be valid UTF-8")
}

#[cfg(test)]
mod tests;
