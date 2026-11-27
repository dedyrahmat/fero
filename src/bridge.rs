#[cxx::bridge(namespace = "fero")]
pub(crate) mod ffi {
    #[derive(Clone, Debug)]
    struct Transfer {
        id: i64,
        name: String,
        progress: f32,
        download_rate: i32,
        upload_rate: i32,
        size: i64,
        wanted: i64,
        remaining: i64,
        peers: i32,
        destination: String,
        state: String,
        error: String,
        paused: bool,
        finished: bool,
        has_metadata: bool,
        downloaded: i64,
        uploaded: i64,
    }

    #[derive(Debug)]
    struct PieceMap {
        total: u32,
        have: u32,
        piece_size: u32,
        // Each cell is missing (0), partial or mixed (1), or complete (2).
        cells: Vec<u8>,
    }

    #[derive(Debug)]
    struct Peer {
        endpoint: String,
        client: String,
        transport: String,
        seed: bool,
        download_rate: i32,
        upload_rate: i32,
        downloaded: i64,
    }

    struct Tracker {
        url: String,
        tier: u8,
        status: String,
        error: String,
    }

    #[derive(Clone, Debug)]
    struct TorrentFile {
        index: u32,
        path: String,
        size: i64,
        selected: bool,
        priority: u8,
    }

    struct AddOptions {
        magnet: bool,
        paused: bool,
        from_preview: bool,
        skip_existing: bool,
        require_resume: bool,
    }

    struct TorrentHashes {
        v1: Vec<u8>,
        v2: Vec<u8>,
    }

    struct HttpResponse {
        ready: bool,
        bytes: Vec<u8>,
    }

    struct ResumeData {
        id: i64,
        bytes: Vec<u8>,
    }

    struct EngineOptions {
        local_only: bool,
        listen_port: u16,
        connections: i32,
        proxy_host: String,
        proxy_port: u16,
        vpn_interface: String,
        vpn_dns: String,
        network_ready: bool,
    }

    unsafe extern "C++" {
        include!("native/engine.h");

        type Engine;

        fn new_engine(options: &EngineOptions) -> Result<UniquePtr<Engine>>;
        fn set_network_ready(self: Pin<&mut Engine>, ready: bool) -> Result<()>;
        type HttpRequest;
        fn request_feed(self: &Engine, url: &str) -> Result<UniquePtr<HttpRequest>>;
        fn poll(self: Pin<&mut HttpRequest>) -> Result<HttpResponse>;
        fn vpn_status(
            self: Pin<&mut Engine>,
            adapter_name: &str,
            dns_configured: bool,
        ) -> Result<String>;
        fn suspend_for_network_restart(self: Pin<&mut Engine>) -> Result<()>;
        fn inspect_torrent(source: &[u8]) -> Result<Vec<TorrentFile>>;
        fn torrent_hashes(source: &[u8], magnet: bool) -> Result<TorrentHashes>;
        fn add(
            self: Pin<&mut Engine>,
            id: i64,
            source: &[u8],
            destination: &str,
            resume: &[u8],
            skipped_files: &[u32],
            options: AddOptions,
        ) -> Result<bool>;
        fn fetch_metadata(self: Pin<&mut Engine>, magnet: &str, destination: &str) -> Result<()>;
        fn metadata(self: &Engine) -> Result<Vec<TorrentFile>>;
        fn files(self: &Engine, id: i64) -> Result<Vec<TorrentFile>>;
        fn pieces(self: &Engine, id: i64) -> Result<PieceMap>;
        fn peers(self: &Engine, id: i64) -> Result<Vec<Peer>>;
        fn trackers(self: &Engine, id: i64) -> Result<Vec<Tracker>>;
        fn reannounce(self: Pin<&mut Engine>, id: i64) -> Result<()>;
        fn force_recheck(self: Pin<&mut Engine>, id: i64) -> Result<()>;
        fn check_until_ready(self: Pin<&mut Engine>, id: i64) -> Result<()>;
        fn edit_tracker(
            self: Pin<&mut Engine>,
            id: i64,
            old: &str,
            url: &str,
            tier: u8,
        ) -> Result<()>;
        fn set_file_priority(
            self: Pin<&mut Engine>,
            id: i64,
            index: u32,
            priority: u8,
        ) -> Result<()>;
        fn set_limits(self: Pin<&mut Engine>, id: i64, download: i32, upload: i32) -> Result<()>;
        fn clear_error(self: Pin<&mut Engine>, id: i64) -> Result<()>;
        fn remove(self: Pin<&mut Engine>, id: i64, delete_files: bool) -> Result<()>;
        fn set_paused(self: Pin<&mut Engine>, id: i64, paused: bool, graceful: bool) -> Result<()>;
        fn snapshot(self: Pin<&mut Engine>) -> Result<Vec<Transfer>>;
        fn checkpoint(self: Pin<&mut Engine>, id: i64) -> Result<Vec<u8>>;
        fn checkpoints(self: Pin<&mut Engine>, flush: bool) -> Result<Vec<ResumeData>>;
        fn listen_port(self: &Engine) -> u16;
        fn connect_local(self: Pin<&mut Engine>, id: i64, port: u16) -> Result<()>;
    }
}
