use super::*;
use sha1_smol::Sha1;
use sha2::{Digest, Sha256};
use std::{
    io,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while !condition() {
        assert!(Instant::now() < deadline, "Loopback swarm timed out");
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn saved_network_settings_apply_on_reopen() -> Result<()> {
    use std::net::TcpListener;

    let temp = tempfile::tempdir()?;
    let database = temp.path().join("library.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.management().network, NetworkSettings::default());

    let available = TcpListener::bind(("127.0.0.1", 0))?;
    let network = NetworkSettings {
        listen_port: available.local_addr()?.port(),
        connections: 57,
        ..NetworkSettings::default()
    };
    library.save_network_settings(network.clone())?;
    assert!(
        library
            .save_network_settings(NetworkSettings {
                connections: 0,
                ..network.clone()
            })
            .is_err()
    );
    assert_eq!(library.management().network, network);
    assert_eq!(
        library.management().active_network,
        NetworkSettings::default()
    );
    assert!(!library.management().network_restart_required);
    assert_ne!(library.engine.listen_port(), network.listen_port);
    drop(library);
    drop(available);

    let library = Library::open(&database)?;
    assert_eq!(library.management().network, network);
    assert_eq!(library.management().active_network, network);
    assert_eq!(library.engine.listen_port(), network.listen_port);
    Ok(())
}

#[test]
fn unavailable_vpn_route_keeps_restored_transfers_offline() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("library.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    let id = library.add(
        Source::Magnet(format!("magnet:?xt=urn:btih:{}", "2".repeat(40))),
        temp.path(),
    )?;
    let settings = NetworkSettings {
        vpn_interface: "fero-no-such-vpn".into(),
        ..NetworkSettings::default()
    };
    library.save_network_settings(settings.clone())?;
    assert_eq!(library.management().network, settings);
    assert!(library.management().active_network.vpn_interface.is_empty());
    assert!(library.management().network_restart_required);
    assert_eq!(library.snapshot()?[0].state, "Waiting for network");
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    let rows = library.snapshot()?;
    assert_eq!(library.management().network, settings);
    assert_eq!(library.management().active_network, settings);
    assert!(!library.management().network_restart_required);
    assert!(library.management().network_problem.is_some());
    assert_eq!(library.engine.listen_port(), 0);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].state, "Waiting for network");
    Ok(())
}

#[cfg(fero_strict_interface)]
fn loopback_interface() -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[System.Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces() |
                 Where-Object NetworkInterfaceType -eq Loopback |
                 Select-Object -First 1 -ExpandProperty Id",
            ])
            .output()?;
        let id = std::str::from_utf8(&output.stdout)?
            .trim()
            .trim_matches(['{', '}']);
        ensure!(
            output.status.success() && id.len() == 36,
            "Find the Windows loopback adapter"
        );
        Ok(format!("{{{}}}", id.to_ascii_uppercase()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(if cfg!(target_os = "linux") {
            "lo"
        } else {
            "lo0"
        }
        .into())
    }
}

#[cfg(fero_strict_interface)]
#[test]
fn vpn_interface_route_downloads_through_the_bound_device() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let destination = root.join("download");
    fs::create_dir(&destination)?;
    let (torrent, _, bytes) = write_fixture(root, root, "bound.bin", 256 * 1024)?;
    let mut seed = Library::open_with_network(&root.join("seed.sqlite3"), true)?;
    seed.add(Source::File(torrent.clone()), root)?;
    wait(|| seed.snapshot().unwrap()[0].state == "Seeding");

    let database = root.join("library.sqlite3");
    Library::open_with_network(&database, true)?.save_network_settings(NetworkSettings {
        listen_port: 0,
        vpn_interface: loopback_interface()?,
        ..NetworkSettings::default()
    })?;
    let mut library = Library::open_with_network(&database, true)?;
    wait(|| {
        library.snapshot().unwrap();
        library.management().network_problem.is_none()
    });
    assert_ne!(library.engine.listen_port(), 0);
    let id = library.add(Source::File(torrent), &destination)?;
    library
        .engine
        .pin_mut()
        .connect_local(id, seed.engine.listen_port())?;
    wait(|| library.snapshot().unwrap()[0].finished);
    assert_eq!(fs::read(destination.join("bound.bin"))?, bytes);
    Ok(())
}

#[cfg(all(target_os = "linux", fero_strict_interface))]
#[test]
#[ignore = "requires an isolated Linux network namespace"]
fn vpn_link_loss_and_recovery_preserves_download_and_manual_pause() -> Result<()> {
    ensure!(
        fs::read_link("/proc/self/ns/net")? != fs::read_link("/proc/1/ns/net")?,
        "Run this test inside an isolated network namespace"
    );
    let ip = |arguments: &[&str]| -> Result<()> {
        let status = std::process::Command::new("ip").args(arguments).status()?;
        ensure!(status.success(), "Couldn't run ip {}", arguments.join(" "));
        Ok(())
    };
    let set_loopback = |state| ip(&["link", "set", "lo", state]);
    set_loopback("up")?;
    ip(&["address", "add", "10.55.4.1/32", "dev", "lo"])?;
    ip(&["link", "add", "fero-direct", "type", "dummy"])?;
    ip(&["link", "set", "fero-direct", "addrgenmode", "none"])?;
    ip(&["address", "add", "10.55.5.1/24", "dev", "fero-direct"])?;
    ip(&["link", "set", "fero-direct", "up"])?;
    ip(&["route", "add", "default", "dev", "fero-direct"])?;
    let direct_packets_sent = || -> Result<u64> {
        let stats = fs::read_to_string("/proc/net/dev")?;
        let counters = stats
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find_map(|(name, counters)| (name.trim() == "fero-direct").then_some(counters))
            .context("Read the default interface counters")?;
        // Eight receive counters are followed by transmitted bytes and packets.
        Ok(counters
            .split_whitespace()
            .nth(9)
            .context("Read transmitted packets")?
            .parse()?)
    };
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let destination = root.join("download");
    fs::create_dir(&destination)?;
    let (torrent, _, bytes) = write_fixture(root, root, "recover-bound.bin", 4 * 1024 * 1024)?;
    let discovery = std::net::UdpSocket::bind("10.55.4.1:0")?;
    discovery.set_read_timeout(Some(Duration::from_secs(8)))?;
    let mut metainfo = fs::read(&torrent)?;
    assert_eq!(metainfo.pop(), Some(b'e'));
    metainfo.extend_from_slice(
        format!(
            "5:nodesll9:10.55.4.1i{}eeee",
            discovery.local_addr()?.port()
        )
        .as_bytes(),
    );
    fs::write(&torrent, metainfo)?;
    let mut seed = Library::open_with_network(&root.join("seed.sqlite3"), true)?;
    seed.add(Source::File(torrent.clone()), root)?;
    wait(|| seed.snapshot().unwrap()[0].state == "Seeding");
    let database = root.join("library.sqlite3");
    Library::open_with_network(&database, true)?.save_network_settings(NetworkSettings {
        listen_port: 0,
        vpn_interface: "lo".into(),
        ..NetworkSettings::default()
    })?;
    let mut library = Library::open_with_network(&database, true)?;
    let id = library.add(Source::File(torrent), &destination)?;
    let mut packet = [0; 1500];
    let length = discovery.recv(&mut packet)?;
    assert!(
        packet[..length]
            .windows(6)
            .any(|window| window == b"1:y1:q")
    );
    library.edit_tracker(id, "", "http://198.51.100.1/announce", 0)?;
    library.add_prepared(
        &PreparedSource::new(&Source::Magnet(format!(
            "magnet:?xt=urn:btih:{}",
            "3".repeat(40)
        )))?,
        &destination,
        true,
        &[],
    )?;
    library
        .engine
        .pin_mut()
        .connect_local(id, seed.engine.listen_port())?;
    wait(|| {
        let rows = library.snapshot().unwrap();
        rows[0].downloaded > 0 && !rows[0].finished && rows[1].paused
    });

    let before_outage = direct_packets_sent()?;
    set_loopback("down")?;
    wait(|| {
        let rows = library.snapshot().unwrap();
        rows[0].state == "Waiting for network" && rows[0].peers == 0 && rows[1].paused
    });
    let probe = std::net::UdpSocket::bind("0.0.0.0:0")?;
    probe.send_to(b"default route available", "198.51.100.1:9")?;
    assert_eq!(direct_packets_sent()?, before_outage + 1);
    library.reannounce(id)?;
    library.checkpoint()?;
    drop(library);
    let mut library = Library::open_with_network(&database, true)?;
    assert!(library.management().network_problem.is_some());
    assert_eq!(library.snapshot()?[0].state, "Waiting for network");
    assert!(library.snapshot()?[1].paused);
    library.reannounce(id)?;
    assert!(library.fetch_feed("http://198.51.100.1/feed.xml").is_err());
    thread::sleep(Duration::from_secs(1));
    assert_eq!(direct_packets_sent()?, before_outage + 1);

    set_loopback("up")?;
    wait(|| {
        library.snapshot().unwrap();
        library.management().network_problem.is_none() && seed.engine.listen_port() != 0
    });
    library
        .engine
        .pin_mut()
        .connect_local(id, seed.engine.listen_port())?;
    wait(|| library.snapshot().unwrap()[0].finished);
    assert_eq!(fs::read(destination.join("recover-bound.bin"))?, bytes);
    assert!(library.snapshot()?[1].paused);
    Ok(())
}

#[cfg(fero_bound_dns)]
#[test]
fn vpn_resolves_tracker_using_the_bound_dns_server() -> Result<()> {
    use std::net::{TcpListener, UdpSocket};

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let (_, hash, _) = write_fixture(root, root, "bound-tracker.bin", 16 * 1024)?;
    let dns = UdpSocket::bind("127.0.0.1:0")?;
    dns.set_read_timeout(Some(Duration::from_secs(8)))?;
    let dns_server = dns.local_addr()?;
    let tracker = TcpListener::bind("127.0.0.1:0")?;
    tracker.set_nonblocking(true)?;
    let tracker_port = tracker.local_addr()?.port();
    let database = root.join("library.sqlite3");
    Library::open_with_network(&database, true)?.save_network_settings(NetworkSettings {
        listen_port: 0,
        vpn_interface: loopback_interface()?,
        vpn_dns: Some(dns_server),
        ..NetworkSettings::default()
    })?;
    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.management().network.vpn_dns, Some(dns_server));
    let resolver = thread::spawn(move || -> io::Result<()> {
        let domain = b"\x07tracker\x07invalid\0";
        let question_end = 12 + domain.len() + 4;
        for _ in 0..2 {
            let mut query = [0; 1500];
            let (size, client) = dns.recv_from(&mut query)?;
            assert!(size >= question_end);
            assert_eq!(&query[12..12 + domain.len()], domain);
            let is_ipv4 = query[question_end - 4..question_end - 2] == [0, 1];
            let mut reply = query[..question_end].to_vec();
            reply[2..12].copy_from_slice(&[
                0x81,
                0x80,
                0,
                1,
                0,
                if is_ipv4 { 2 } else { 0 },
                0,
                0,
                0,
                0,
            ]);
            if is_ipv4 {
                for address in [2, 1] {
                    reply.extend_from_slice(&[
                        0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 127, 0, 0, address,
                    ]);
                }
            }
            dns.send_to(&reply, client)?;
        }
        Ok(())
    });
    let id = library.add(
        Source::Magnet(format!(
            "magnet:?xt=urn:btih:{hash}&tr=http://tracker.invalid:{tracker_port}/announce"
        )),
        root,
    )?;
    let mut connection = None;
    wait(|| match tracker.accept() {
        Ok((stream, _)) => {
            connection = Some(stream);
            true
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
        Err(error) => panic!("Accept tracker connection: {error}"),
    });
    let mut stream = connection.unwrap();
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        request.push(byte[0]);
        assert!(request.len() < 8192);
    }
    assert!(request.starts_with(b"GET /announce?"));
    let body = "d8:intervali3600e5:peers0:e";
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    drop(stream);
    resolver.join().unwrap()?;
    wait(|| library.trackers(id).unwrap()[0].status == "Working");
    Ok(())
}

const TEST_FEED: &str = "<rss version=\"2.0\"><channel><title>Test feed</title></channel></rss>";

#[cfg(fero_routed_http)]
#[test]
#[ignore = "requires a trusted local HTTPS feed"]
fn https_feed_uses_the_saved_network_route() -> Result<()> {
    use std::net::TcpListener;

    let url = std::env::var("FERO_HTTPS_FEED_URL").context("Set FERO_HTTPS_FEED_URL")?;
    let endpoint = url::Url::parse(&url)?;
    ensure!(endpoint.scheme() == "https", "Use an HTTPS feed");
    let Some(url::Host::Ipv4(address)) = endpoint
        .host()
        .filter(|host| matches!(host, url::Host::Ipv4(ip) if ip.is_loopback()))
    else {
        anyhow::bail!("Use a numeric IPv4 loopback feed");
    };
    let target = SocketAddr::from((address, endpoint.port_or_known_default().unwrap_or(443)));
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("library.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.fetch_feed(&url)?, TEST_FEED.as_bytes());
    library.save_network_settings(NetworkSettings {
        listen_port: 0,
        vpn_interface: loopback_interface()?,
        ..NetworkSettings::default()
    })?;
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    wait(|| {
        library.snapshot().unwrap();
        library.management().network_problem.is_none()
    });
    assert_eq!(library.fetch_feed(&url)?, TEST_FEED.as_bytes());
    let reservation = TcpListener::bind("127.0.0.1:0")?;
    let proxy_addr = reservation.local_addr()?;
    drop(reservation);
    let (relay, stop, connects, _) = start_socks5_relay(proxy_addr, target)?;
    library.save_network_settings(NetworkSettings {
        socks5: Some(proxy_addr),
        ..NetworkSettings::default()
    })?;
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.fetch_feed(&url)?, TEST_FEED.as_bytes());
    connects.recv_timeout(Duration::from_secs(5))?;
    stop.send(())?;
    relay.join().unwrap();
    Ok(())
}

#[cfg(fero_routed_http)]
#[test]
fn feed_requests_use_the_saved_network_route() -> Result<()> {
    use std::net::TcpListener;

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let http = TcpListener::bind("127.0.0.1:0")?;
    http.set_nonblocking(true)?;
    let target = http.local_addr()?;
    let http_worker = http.try_clone()?;
    let server = thread::spawn(move || -> Result<()> {
        for _ in 0..2 {
            let mut connection = None;
            wait(|| match http_worker.accept() {
                Ok((stream, _)) => {
                    connection = Some(stream);
                    true
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
                Err(error) => panic!("Accept feed request: {error}"),
            });
            let mut stream = connection.unwrap();
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            assert!(read_http_request(&mut stream)?.starts_with(b"GET /feed.xml HTTP/1.1\r\n"));
            write_http_response(stream, TEST_FEED)?;
        }
        Ok(())
    });
    let url = format!("http://{target}/feed.xml");
    let database = root.join("library.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.fetch_feed(&url)?, TEST_FEED.as_bytes());
    library.save_network_settings(NetworkSettings {
        listen_port: 0,
        vpn_interface: loopback_interface()?,
        ..NetworkSettings::default()
    })?;
    assert!(library.fetch_feed(&url).is_err());
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    wait(|| {
        library.snapshot().unwrap();
        library.management().network_problem.is_none()
    });
    assert_eq!(library.fetch_feed(&url)?, TEST_FEED.as_bytes());
    server.join().unwrap()?;

    library.save_network_settings(NetworkSettings {
        vpn_interface: "fero-no-such-vpn".into(),
        ..NetworkSettings::default()
    })?;
    assert!(library.fetch_feed(&url).is_err());
    assert_eq!(http.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    assert!(
        library
            .management()
            .network_problem
            .as_deref()
            .is_some_and(|problem| problem.contains("fero-no-such-vpn"))
    );
    assert!(library.fetch_feed(&url).is_err());
    assert_eq!(http.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);

    let reservation = TcpListener::bind("127.0.0.1:0")?;
    let proxy_addr = reservation.local_addr()?;
    drop(reservation);
    let (server, stop, _, _) = start_socks5_relay(proxy_addr, target)?;
    library.save_network_settings(NetworkSettings {
        socks5: Some(proxy_addr),
        ..NetworkSettings::default()
    })?;
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(
        library.fetch_feed("http://tracker.invalid/feed.xml")?,
        TEST_FEED.as_bytes()
    );
    stop.send(())?;
    server.join().unwrap();
    drop(library);
    let mut library = Library::open_with_network(&database, true)?;
    assert!(library.fetch_feed(&url).is_err());
    assert_eq!(http.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    Ok(())
}

#[test]
fn socks5_outage_blocks_transfers_across_restart_and_preserves_manual_pause() -> Result<()> {
    use std::net::TcpListener;

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let database = root.join("library.sqlite3");
    let download = root.join("download");
    fs::create_dir(&download)?;
    let (torrent, _, payload) = write_fixture(root, root, "proxied.bin", 4 * 1024 * 1024)?;
    let mut seeder = Library::open_with_network(&root.join("seed.sqlite3"), true)?;
    seeder.add(Source::File(torrent.clone()), root)?;
    wait(|| seeder.snapshot().unwrap()[0].state == "Seeding");
    let seed_addr: SocketAddr = ([127, 0, 0, 1], seeder.engine.listen_port()).into();

    let reservation = TcpListener::bind(("127.0.0.1", 0))?;
    let proxy_addr = reservation.local_addr()?;
    let mut library = Library::open_with_network(&database, true)?;
    let id = library.add(Source::File(torrent), &download)?;
    library.edit_tracker(id, "", "http://tracker.invalid/announce", 0)?;
    let paused = library.add_prepared(
        &PreparedSource::new(&Source::Magnet(format!(
            "magnet:?xt=urn:btih:{}",
            "1".repeat(40)
        )))?,
        &download,
        true,
        &[],
    )?;
    library.save_network_settings(NetworkSettings {
        socks5: Some(proxy_addr),
        ..NetworkSettings::default()
    })?;
    assert_eq!(library.snapshot()?[0].state, "Waiting for network");
    drop(library);
    drop(reservation);

    let mut library = Library::open_with_network(&database, true)?;
    let rows = library.snapshot()?;
    assert_eq!(rows[0].state, "Waiting for network");
    assert_eq!(rows[1].id, paused);
    assert_eq!(rows[1].state, "Paused");
    assert!(library.management().network_problem.is_some());
    library
        .engine
        .pin_mut()
        .connect_local(id, seed_addr.port())?;
    thread::sleep(Duration::from_millis(500));
    assert_eq!(library.snapshot()?[0].downloaded, 0);

    let (server, stop, connects, _) = start_socks5_relay(proxy_addr, seed_addr)?;
    wait(|| {
        let rows = library.snapshot().unwrap();
        assert!(rows[1].paused);
        library.management().network_problem.is_none()
    });
    library
        .engine
        .pin_mut()
        .connect_local(id, seed_addr.port())?;
    let connection = connects.recv_timeout(Duration::from_secs(12))?;
    wait(|| library.trackers(id).unwrap()[0].status == "Working");
    wait(|| {
        let row = &library.snapshot().unwrap()[0];
        row.downloaded > 0 && !row.finished
    });
    stop.send(())?;
    server.join().unwrap();
    connection.shutdown(std::net::Shutdown::Both)?;
    wait(|| {
        let rows = library.snapshot().unwrap();
        rows[0].state == "Waiting for network" && rows[0].peers == 0 && rows[1].paused
    });
    library.checkpoint()?;
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    let rows = library.snapshot()?;
    assert_eq!(rows[0].state, "Waiting for network");
    assert!(!rows[0].finished);
    assert!(rows[1].paused);
    library.edit_tracker(
        id,
        "http://tracker.invalid/announce",
        "udp://tracker.invalid:80/announce",
        0,
    )?;
    let (server, stop, connects, discovery) = start_socks5_relay(proxy_addr, seed_addr)?;
    wait(|| {
        library.snapshot().unwrap();
        library.management().network_problem.is_none()
    });
    library
        .engine
        .pin_mut()
        .connect_local(id, seed_addr.port())?;
    connects.recv_timeout(Duration::from_secs(12))?;
    wait(|| library.snapshot().unwrap()[0].finished);
    wait(|| library.trackers(id).unwrap()[0].status == "Working");
    library
        .engine
        .pin_mut()
        .connect_local(id, seed_addr.port())?;
    wait(|| discovery.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(fs::read(download.join("proxied.bin"))?, payload);
    assert!(library.snapshot()?[1].paused);
    stop.send(())?;
    server.join().unwrap();
    Ok(())
}

type Socks5Relay = (
    thread::JoinHandle<()>,
    std::sync::mpsc::Sender<()>,
    std::sync::mpsc::Receiver<TcpStream>,
    Arc<std::sync::atomic::AtomicBool>,
);

fn start_socks5_relay(proxy_addr: SocketAddr, seed_addr: SocketAddr) -> io::Result<Socks5Relay> {
    use std::{net::TcpListener, sync::mpsc};

    let proxy = TcpListener::bind(proxy_addr)?;
    proxy.set_nonblocking(true)?;
    let (stop, stopped) = mpsc::channel();
    let (connected, connects) = mpsc::channel();
    let discovery = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relayed_discovery = discovery.clone();
    let server = thread::spawn(move || {
        while matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty)) {
            match proxy.accept() {
                Ok((stream, _)) => {
                    let connected = connected.clone();
                    let relayed_discovery = relayed_discovery.clone();
                    thread::spawn(move || {
                        let _ = serve_socks5(stream, seed_addr, connected, &relayed_discovery);
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        }
    });
    Ok((server, stop, connects, discovery))
}

fn serve_socks5(
    mut stream: TcpStream,
    seed_addr: SocketAddr,
    connected: std::sync::mpsc::Sender<TcpStream>,
    discovery: &std::sync::atomic::AtomicBool,
) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut greeting = [0; 2];
    stream.read_exact(&mut greeting)?;
    assert_eq!(greeting[0], 5);
    let mut methods = vec![0; usize::from(greeting[1])];
    stream.read_exact(&mut methods)?;
    assert!(methods.contains(&0));
    stream.write_all(&[5, 0])?;
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    if header == [5, 3, 0, 1] {
        return serve_socks5_udp(stream, discovery);
    }
    if header == [5, 1, 0, 3] {
        let mut length = [0];
        stream.read_exact(&mut length)?;
        let mut destination = vec![0; usize::from(length[0]) + 2];
        stream.read_exact(&mut destination)?;
        assert_eq!(destination, b"tracker.invalid\x00\x50");
        stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
        let request = read_http_request(&mut stream)?;
        if request.starts_with(b"GET /feed.xml ") {
            return write_http_response(stream, TEST_FEED);
        }
        assert!(request.starts_with(b"GET /announce?"));
        return write_http_response(stream, "d8:intervali60e5:peers0:e");
    }
    if header != [5, 1, 0, 1] {
        return stream.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
    }
    let mut address = [0; 6];
    stream.read_exact(&mut address)?;
    let target = SocketAddr::from((
        [address[0], address[1], address[2], address[3]],
        u16::from_be_bytes([address[4], address[5]]),
    ));
    assert_eq!(target, seed_addr);
    let mut peer = TcpStream::connect(seed_addr)?;
    stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
    connected.send(stream.try_clone()?).unwrap();
    let mut inbound = stream.try_clone()?;
    let mut outbound = peer.try_clone()?;
    let upstream = thread::spawn(move || {
        let _ = io::copy(&mut inbound, &mut outbound);
        let _ = outbound.shutdown(std::net::Shutdown::Both);
    });
    let _ = io::copy(&mut peer, &mut stream);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    upstream.join().unwrap();
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        request.push(byte[0]);
        assert!(request.len() < 8192);
    }
    Ok(request)
}

fn write_http_response(mut stream: TcpStream, body: &str) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn serve_socks5_udp(
    mut stream: TcpStream,
    discovery: &std::sync::atomic::AtomicBool,
) -> io::Result<()> {
    use std::net::UdpSocket;

    let mut endpoint = [0; 6];
    stream.read_exact(&mut endpoint)?;
    let relay = UdpSocket::bind(("127.0.0.1", 0))?;
    relay.set_read_timeout(Some(Duration::from_millis(100)))?;
    let port = relay.local_addr()?.port().to_be_bytes();
    stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, port[0], port[1]])?;
    stream.set_nonblocking(true)?;

    let tracker = b"\0\0\0\x03\x0ftracker.invalid\0\x50";
    let mut request = [0; 1024];
    while stream
        .read(&mut [0])
        .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
    {
        let (length, client) = match relay.recv_from(&mut request) {
            Ok(packet) => packet,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        let Some(payload) = request[..length].strip_prefix(tracker) else {
            if length > 10
                && request[..4] == [0, 0, 0, 1]
                && request[10..length]
                    .windows(6)
                    .any(|window| window == b"1:y1:q")
            {
                discovery.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            continue;
        };
        assert!(payload.len() >= 16);
        let action = &payload[8..12];
        let mut response = tracker.to_vec();
        response.extend_from_slice(action);
        response.extend_from_slice(&payload[12..16]);
        match action {
            [0, 0, 0, 0] => response.extend_from_slice(&1_u64.to_be_bytes()),
            [0, 0, 0, 1] => response.extend_from_slice(&[0, 0, 0, 60, 0, 0, 0, 0, 0, 0, 0, 1]),
            _ => continue,
        }
        relay.send_to(&response, client)?;
    }
    Ok(())
}

fn write_fixture(
    root: &Path,
    seed_dir: &Path,
    name: &str,
    length: usize,
) -> Result<(PathBuf, String, Vec<u8>)> {
    let payload: Vec<u8> = (0..length).map(|index| (index % 251) as u8).collect();
    fs::write(seed_dir.join(name), &payload)?;

    let piece_size = 16 * 1024;
    let mut info = format!(
        "d6:lengthi{length}e4:name{}:{name}12:piece lengthi{piece_size}e6:pieces{}:",
        name.len(),
        20 * length.div_ceil(piece_size)
    )
    .into_bytes();
    for piece in payload.chunks(piece_size) {
        info.extend(Sha1::from(piece).digest().bytes());
    }
    info.push(b'e');
    let info_hash = Sha1::from(&info).digest().to_string();
    let mut metainfo = b"d4:info".to_vec();
    metainfo.extend(info);
    metainfo.push(b'e');
    let torrent_path = root.join("test.torrent");
    fs::write(&torrent_path, metainfo)?;
    Ok((torrent_path, info_hash, payload))
}

#[test]
fn qbit_profile_preview_is_read_only_and_marks_missing_backup_files() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let profile = temp.path().join("qbit");
    let backup = profile.join("BT_backup");
    let database = profile.join("torrents.db");
    fs::create_dir_all(&backup)?;
    let db = Connection::open(&database)?;
    db.execute_batch("CREATE TABLE torrents (torrent_id BLOB, target_save_path TEXT, category TEXT, tags TEXT, libtorrent_resume_data BLOB);")?;
    let download = temp.path().join("gone");
    fs::create_dir_all(&download)?;
    let (torrent, info_hash, _) = write_fixture(temp.path(), &download, "missing.bin", 2048)?;
    fs::remove_file(download.join("missing.bin"))?;
    let torrent_id = (0..info_hash.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&info_hash[index..index + 2], 16).unwrap())
        .collect::<Vec<_>>();
    fs::copy(torrent, backup.join(format!("{info_hash}.torrent")))?;
    db.execute(
        "INSERT INTO torrents VALUES (?1, ?2, 'TV', 'one,two', X'')",
        params![torrent_id, download.to_string_lossy().as_ref()],
    )?;
    drop(db);
    let library = Library::open_with_network(&temp.path().join("fero.sqlite3"), true)?;
    let preview = library.preview_qbittorrent(&profile)?;
    assert_eq!(preview.items.len(), 1);
    assert!(preview.items[0].missing);
    assert!(!preview.items[0].bytes.is_empty());
    assert_eq!(preview.items[0].category, "TV");
    let reopened = Connection::open(&database)?;
    assert_eq!(
        reopened.query_row("SELECT count(*) FROM torrents", [], |row| row
            .get::<_, usize>(0))?,
        1
    );
    Ok(())
}

#[test]
fn qbit_sqlite_mixed_profile_import_is_repeatable_and_read_only() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let profile = temp.path().join("qbit");
    let backup = profile.join("BT_backup");
    let complete = temp.path().join("complete");
    let absent = temp.path().join("absent");
    fs::create_dir_all(&backup)?;
    fs::create_dir_all(&complete)?;
    fs::create_dir_all(&absent)?;

    let complete_src = temp.path().join("complete-src");
    let missing_src = temp.path().join("missing-src");
    fs::create_dir(&complete_src)?;
    fs::create_dir(&missing_src)?;
    let (complete_torrent, complete_hash, _) =
        write_fixture(&complete_src, &complete_src, "complete.bin", 2048)?;
    let (missing_torrent, missing_hash, _) =
        write_fixture(&missing_src, &missing_src, "missing.bin", 2048)?;
    fs::copy(
        complete_src.join("complete.bin"),
        complete.join("complete.bin"),
    )?;

    let database = profile.join("torrents.db");
    let db = Connection::open(&database)?;
    db.execute_batch("CREATE TABLE torrents (torrent_id BLOB, target_save_path TEXT, category TEXT, tags TEXT, libtorrent_resume_data BLOB);")?;
    for (torrent, hash, destination, category) in [
        (complete_torrent, complete_hash, &complete, "complete"),
        (missing_torrent, missing_hash, &absent, "missing"),
    ] {
        let hash_bytes = ffi::torrent_hashes(&fs::read(&torrent)?, false)?.v1;
        fs::copy(torrent, backup.join(format!("{hash}.torrent")))?;
        db.execute(
            "INSERT INTO torrents VALUES (?1, ?2, ?3, 'tag', X'')",
            params![hash_bytes, destination.to_string_lossy().as_ref(), category],
        )?;
    }
    fs::remove_file(missing_src.join("missing.bin"))?;
    drop(db);
    let source_before = fs::read(&database)?;
    let mut library = Library::open_with_network(&temp.path().join("fero.sqlite3"), true)?;
    let preview = library.preview_qbittorrent(&profile)?;
    assert_eq!(preview.items.len(), 2);
    assert!(
        preview
            .items
            .iter()
            .any(|item| !item.missing && item.category == "complete")
    );
    assert!(
        preview
            .items
            .iter()
            .any(|item| item.missing && item.category == "missing")
    );
    let selection = preview
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| (index, item.destination.clone()))
        .collect::<Vec<_>>();
    assert_eq!(library.import_qbittorrent(&preview, &selection)?, 2);
    let transfers = library.snapshot()?;
    assert_eq!(transfers.len(), 2);
    assert!(transfers.iter().all(|transfer| transfer.paused));
    assert!(
        library
            .preview_qbittorrent(&profile)?
            .items
            .iter()
            .all(|item| item.duplicate)
    );
    assert_eq!(fs::read(&database)?, source_before);
    Ok(())
}

#[test]
fn qbit_backup_file_only_profile_previews_torrents_idempotently() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let profile = temp.path().join("qbit");
    let backup = profile.join("BT_backup");
    fs::create_dir_all(&backup)?;
    let (fixture, _, _) = write_fixture(temp.path(), temp.path(), "existing.bin", 2048)?;
    fs::copy(&fixture, backup.join("only.torrent"))?;
    let mut library = Library::open_with_network(&temp.path().join("fero.sqlite3"), true)?;
    let preview = library.preview_qbittorrent(&profile)?;
    assert_eq!(preview.items.len(), 1);
    assert!(!preview.items[0].bytes.is_empty());
    assert_eq!(
        library.import_qbittorrent(&preview, &[(0, temp.path().to_path_buf())])?,
        1
    );
    assert!(library.snapshot()?[0].paused);
    drop(library);
    let mut library = Library::open_with_network(&temp.path().join("fero.sqlite3"), true)?;
    let second = library.preview_qbittorrent(&profile)?;
    assert!(second.items[0].duplicate);
    let alternate = temp.path().join("alternate");
    fs::create_dir(&alternate)?;
    assert!(
        library
            .import_qbittorrent(&second, &[(0, alternate)])
            .is_err()
    );
    Ok(())
}

#[test]
fn qbit_backup_mixed_profile_preview_import_and_repeat_are_safe() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let profile = temp.path().join("qbit");
    let backup = profile.join("BT_backup");
    fs::create_dir_all(&backup)?;
    let complete_src = temp.path().join("complete-src");
    let missing_src = temp.path().join("missing-src");
    fs::create_dir(&complete_src)?;
    fs::create_dir(&missing_src)?;
    let (complete_torrent, _, _) =
        write_fixture(&complete_src, &complete_src, "complete.bin", 2048)?;
    let (missing_torrent, _, _) = write_fixture(&missing_src, &missing_src, "missing.bin", 2048)?;
    let complete_file = backup.join("complete.torrent");
    let missing_file = backup.join("missing.torrent");
    fs::copy(complete_torrent, &complete_file)?;
    fs::copy(missing_torrent, &missing_file)?;
    fs::copy(
        complete_src.join("complete.bin"),
        backup.join("complete.bin"),
    )?;
    fs::remove_file(missing_src.join("missing.bin"))?;
    let source_before = [
        (&complete_file, fs::read(&complete_file)?),
        (&missing_file, fs::read(&missing_file)?),
        (
            &backup.join("complete.bin"),
            fs::read(backup.join("complete.bin"))?,
        ),
    ];
    let complete_destination = backup.canonicalize()?;
    let mut library = Library::open_with_network(&temp.path().join("fero.sqlite3"), true)?;
    let first_preview = library.preview_qbittorrent(&profile)?;
    assert_eq!(first_preview.items.len(), 2);
    let complete_item = first_preview
        .items
        .iter()
        .find(|item| item.torrent.ends_with("complete.torrent"))
        .unwrap();
    assert!(!complete_item.duplicate && !complete_item.missing);
    let complete_index = first_preview
        .items
        .iter()
        .position(|item| item.torrent.ends_with("complete.torrent"))
        .unwrap();
    assert_eq!(
        library.import_qbittorrent(&first_preview, &[(complete_index, complete_destination)])?,
        1
    );
    let preview = library.preview_qbittorrent(&profile)?;
    assert_eq!(preview.items.len(), 2);
    let complete = preview
        .items
        .iter()
        .find(|item| item.torrent.ends_with("complete.torrent"))
        .unwrap();
    let missing = preview
        .items
        .iter()
        .find(|item| item.torrent.ends_with("missing.torrent"))
        .unwrap();
    assert!(complete.duplicate && !complete.missing);
    assert!(!missing.duplicate && missing.missing);
    let missing_index = preview
        .items
        .iter()
        .position(|item| item.torrent.ends_with("missing.torrent"))
        .unwrap();
    assert_eq!(
        library.import_qbittorrent(&preview, &[(missing_index, backup.clone())])?,
        1
    );
    assert_eq!(library.snapshot()?.len(), 2);
    assert!(library.snapshot()?.iter().all(|transfer| transfer.paused));
    let repeated = library.preview_qbittorrent(&profile)?;
    assert!(repeated.items.iter().all(|item| item.duplicate));
    for (path, original) in source_before {
        assert_eq!(fs::read(path)?, original);
    }
    Ok(())
}

#[test]
fn qbit_import_uses_existing_complete_content_and_rechecks_to_seed() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let profile = temp.path().join("qbit");
    let backup = profile.join("BT_backup");
    fs::create_dir_all(&backup)?;
    let (fixture, _, _) = write_fixture(temp.path(), temp.path(), "have.bin", 16 * 1024)?;
    let data = backup.clone();
    fs::copy(&fixture, backup.join("seed.torrent"))?;
    let seed = PreparedSource::from_torrent_bytes(fs::read(&fixture)?)?;
    let file_path = data.join(&seed.files[0].path);
    let hash = seed.files[0].path.clone();
    assert_eq!(hash, "have.bin");
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(temp.path().join("have.bin"), &file_path)?;
    assert!(file_path.is_file());
    let mut library = Library::open_with_network(&temp.path().join("fero.sqlite3"), true)?;
    let preview = library.preview_qbittorrent(&profile)?;
    let index = preview
        .items
        .iter()
        .position(|item| item.torrent.ends_with("seed.torrent"))
        .unwrap();
    assert!(!preview.items[index].missing);
    assert_eq!(library.import_qbittorrent(&preview, &[(index, data)])?, 1);
    wait(|| library.snapshot().unwrap()[0].finished);
    let snapshot = library.snapshot()?;
    assert!(snapshot[0].paused);
    assert_eq!(snapshot[0].progress, 1.0);
    drop(library);
    let library = Library::open_with_network(&temp.path().join("fero.sqlite3"), true)?;
    let repeated = library.preview_qbittorrent(&profile)?;
    assert!(repeated.items[0].duplicate);
    Ok(())
}

#[test]
fn qbit_fastresume_file_supplies_embedded_torrent_metadata() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let backup = temp.path().join("BT_backup");
    fs::create_dir_all(&backup)?;
    let (torrent, _, _) = write_fixture(temp.path(), temp.path(), "payload.bin", 1024)?;
    let bytes = fs::read(torrent)?;
    let info = bytes
        .strip_prefix(b"d4:info")
        .unwrap()
        .strip_suffix(b"e")
        .unwrap();
    let mut resume = b"d4:info".to_vec();
    resume.extend_from_slice(info);
    resume.extend_from_slice(b"e");
    fs::write(backup.join("backup.fastresume"), resume)?;
    let library = Library::open_with_network(&temp.path().join("fero.sqlite3"), true)?;
    let preview = library.preview_qbittorrent(temp.path())?;
    assert_eq!(preview.items.len(), 1);
    assert!(!preview.items[0].bytes.is_empty());
    assert_eq!(
        preview.items[0].resume.len(),
        info.len() + b"d4:infoe".len()
    );
    Ok(())
}

#[test]
fn deleting_active_download_removes_partial_data_and_keeps_unrelated_files() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let seed_dir = root.join("seed");
    let destination = root.join("download");
    fs::create_dir(&seed_dir)?;
    fs::create_dir(&destination)?;
    let (torrent, hash, payload) = write_fixture(root, &seed_dir, "partial.bin", 8 * 1024 * 1024)?;
    let unrelated = destination.join("unrelated.txt");
    fs::write(&unrelated, b"keep this")?;
    let mut seed = Library::open_with_network(&root.join("seed.sqlite3"), true)?;
    seed.add(Source::File(torrent), &seed_dir)?;
    wait(|| seed.snapshot().unwrap()[0].state == "Seeding");

    let database = root.join("download.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    let id = library.add(
        Source::Magnet(format!("magnet:?xt=urn:btih:{hash}")),
        &destination,
    )?;
    library
        .engine
        .pin_mut()
        .connect_local(id, seed.engine.listen_port())?;
    let partial = destination.join("partial.bin");
    wait(|| {
        let row = &library.snapshot().unwrap()[0];
        row.state == "Downloading"
            && row.progress > 0.05
            && fs::read(&partial)
                .is_ok_and(|bytes| bytes != payload && bytes.iter().any(|&byte| byte != 0))
    });
    library.remove(id, true)?;
    assert!(!partial.exists());
    assert_eq!(fs::read(&unrelated)?, b"keep this");
    assert!(library.snapshot()?.is_empty());
    drop(library);
    assert!(
        Library::open_with_network(&database, true)?
            .snapshot()?
            .is_empty()
    );
    assert!(!partial.exists());
    Ok(())
}

#[test]
fn deleting_selective_download_cleans_up_its_part_file() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let seed_dir = root.join("seed");
    let download = root.join("download");
    fs::create_dir_all(seed_dir.join("bundle"))?;
    fs::create_dir(&download)?;
    let mut payload = vec![37; 128 * 1024];
    payload[0] = 91;
    fs::write(seed_dir.join("bundle/skip.bin"), &payload[..1])?;
    fs::write(seed_dir.join("bundle/keep.bin"), &payload[1..16 * 1024])?;
    let mut metainfo = b"d4:infod5:filesld6:lengthi1e4:pathl8:skip.bineed6:lengthi131071e4:pathl8:keep.bineee4:name6:bundle12:piece lengthi16384e6:pieces160:".to_vec();
    for piece in payload.chunks(16 * 1024) {
        metainfo.extend(Sha1::from(piece).digest().bytes());
    }
    metainfo.extend(b"ee");
    let torrent = root.join("selective.torrent");
    fs::write(&torrent, metainfo)?;
    let source = Source::File(torrent);
    let mut seed = Library::open_with_network(&root.join("seed.sqlite3"), true)?;
    seed.add(source.clone(), &seed_dir)?;
    wait(|| seed.snapshot().unwrap()[0].progress > 0.0);

    let mut library = Library::open_with_network(&root.join("partial.sqlite3"), true)?;
    let source = PreparedSource::new(&source)?;
    let id = library.add_prepared(&source, &download, false, &[0])?;
    library
        .engine
        .pin_mut()
        .connect_local(id, seed.engine.listen_port())?;
    let find_part_file = || {
        fs::read_dir(&download)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|ext| ext == "parts"))
    };
    wait(|| {
        let row = &library.snapshot().unwrap()[0];
        row.progress > 0.0 && row.progress < 1.0 && find_part_file().is_some()
    });
    let part_file = find_part_file().unwrap();
    let partial_payload = download.join("bundle/keep.bin");
    assert!(partial_payload.exists());
    library.remove(id, true)?;
    assert!(!partial_payload.exists());
    assert!(!part_file.exists());
    Ok(())
}

#[test]
fn tracker_status_updates_after_reannounce() -> Result<()> {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let (_, hash, _) = write_fixture(root, root, "tracked.bin", 16 * 1024)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let tracker_url = format!("http://{}/announce", listener.local_addr()?);
    let mut library = Library::open_with_network(&root.join("tracked.sqlite3"), true)?;
    let id = library.add(
        Source::Magnet(format!("magnet:?xt=urn:btih:{hash}&tr={tracker_url}")),
        root,
    )?;

    let respond = |body: &str| -> Result<()> {
        let mut stream = None;
        wait(|| match listener.accept() {
            Ok((socket, _)) => {
                stream = Some(socket);
                true
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("Accept tracker connection: {error}"),
        });
        let mut stream = stream.unwrap();
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        assert!(line.starts_with("GET /announce?"), "{line}");
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 || line == "\r\n" {
                break;
            }
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        Ok(())
    };

    respond("d8:intervali3600e12:min intervali1e5:peers0:e")?;
    wait(|| library.trackers(id).unwrap()[0].status == "Working");
    assert_eq!(library.trackers(id)?[0].url, tracker_url);
    library.reannounce(id)?;
    respond("d14:failure reason14:tracker closede")?;
    wait(|| library.trackers(id).unwrap()[0].status == "Error");
    assert!(library.trackers(id)?[0].error.contains("tracker closed"));
    let replacement = "http://127.0.0.1:1/announce";
    library.edit_tracker(id, &tracker_url, replacement, 2)?;
    drop(library);

    let mut library = Library::open_with_network(&root.join("tracked.sqlite3"), true)?;
    assert_eq!(library.trackers(id)?[0].url, replacement);
    assert_eq!(library.trackers(id)?[0].tier, 2);
    library.edit_tracker(id, replacement, "", 0)?;
    drop(library);
    let library = Library::open_with_network(&root.join("tracked.sqlite3"), true)?;
    assert!(library.trackers(id)?.is_empty());
    Ok(())
}

#[test]
fn queue_limits_options_and_bulk_pauses_survive_restart() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let database = root.join("queue.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    let limits = LibraryLimits {
        bandwidth: Bandwidth {
            download: 512 * 1024,
            upload: 64 * 1024,
        },
        downloads: 1,
        seeds: 2,
        seed_ratio: 200,
    };
    library.set_library_limits(limits)?;
    let mut ids = Vec::new();
    for name in ["first.bin", "second.bin", "third.bin"] {
        let (_, hash, _) = write_fixture(root, root, name, 16 * 1024)?;
        ids.push(library.add(
            Source::Magnet(format!("magnet:?xt=urn:btih:{hash}&dn={name}")),
            root,
        )?);
    }
    let [first, second, third] = ids[..] else {
        unreachable!()
    };
    let states = |library: &mut Library| -> Result<Vec<(i64, String)>> {
        Ok(library
            .snapshot()?
            .into_iter()
            .map(|row| (row.id, row.state))
            .collect())
    };
    assert_eq!(
        states(&mut library)?,
        vec![
            (first, "Fetching metadata".into()),
            (second, "Queued".into()),
            (third, "Queued".into())
        ]
    );
    library.move_in_queue(second, true)?;
    assert_eq!(
        states(&mut library)?,
        vec![
            (second, "Fetching metadata".into()),
            (first, "Queued".into()),
            (third, "Queued".into())
        ]
    );
    library.set_transfer_options(
        second,
        TransferOptions {
            category: "  Linux  ".into(),
            tags: " iso, , QA, qa ".into(),
            bandwidth: Bandwidth {
                download: 32 * 1024,
                upload: 16 * 1024,
            },
            seed_ratio: Some(150),
        },
    )?;
    assert!(library.set_paused_many(&[second, i64::MAX], true).is_err());
    assert_eq!(states(&mut library)?[0].1, "Fetching metadata");
    library.set_paused_many(&[first, second], true)?;
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.management().limits, limits);
    assert_eq!(
        library.management().options[&second],
        TransferOptions {
            category: "Linux".into(),
            tags: "iso, QA".into(),
            bandwidth: Bandwidth {
                download: 32 * 1024,
                upload: 16 * 1024
            },
            seed_ratio: Some(150),
        }
    );
    assert_eq!(
        states(&mut library)?,
        vec![
            (second, "Paused".into()),
            (first, "Paused".into()),
            (third, "Fetching metadata".into())
        ]
    );
    library.set_paused_many(&[first, second], false)?;
    assert_eq!(
        states(&mut library)?[0],
        (second, "Fetching metadata".into())
    );
    library.remove_many(&[second, third])?;
    assert_eq!(library.management().queue, vec![first]);
    assert_eq!(
        states(&mut library)?,
        vec![(first, "Fetching metadata".into())]
    );
    Ok(())
}

#[test]
fn reaching_the_seed_ratio_frees_a_seed_slot_and_persists() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let (first_torrent, _, _) = write_fixture(root, root, "first.bin", 1024 * 1024)?;
    let mut library = Library::open_with_network(&root.join("queue.sqlite3"), true)?;
    let first = library.add(Source::File(first_torrent.clone()), root)?;
    let peer_dir = root.join("peer");
    fs::create_dir(&peer_dir)?;
    let mut peer = Library::open_with_network(&root.join("peer.sqlite3"), true)?;
    let peer_id = peer.add(Source::File(first_torrent), &peer_dir)?;
    let (second_torrent, _, _) = write_fixture(root, root, "second.bin", 16 * 1024)?;
    let second = library.add(Source::File(second_torrent), root)?;
    library.set_library_limits(LibraryLimits {
        bandwidth: Bandwidth {
            download: 0,
            upload: 96 * 1024,
        },
        seeds: 1,
        seed_ratio: 25,
        ..LibraryLimits::default()
    })?;
    library.set_transfer_options(
        first,
        TransferOptions {
            bandwidth: Bandwidth {
                download: 0,
                upload: 48 * 1024,
            },
            ..TransferOptions::default()
        },
    )?;
    wait(|| {
        let rows = library.snapshot().unwrap();
        rows[0].state == "Seeding" && rows[1].finished && rows[1].state == "Queued"
    });
    let start = Instant::now();
    peer.engine
        .pin_mut()
        .connect_local(peer_id, library.engine.listen_port())?;
    wait(|| {
        let rows = library.snapshot().unwrap();
        rows[0].state == "Ratio reached" && !rows[0].paused && rows[1].state == "Seeding"
    });
    assert!(library.snapshot()?[0].uploaded >= 256 * 1024);
    assert!(
        start.elapsed() >= Duration::from_secs(3),
        "The per-torrent upload cap was bypassed"
    );
    library.checkpoint()?;
    drop(library);
    let mut library = Library::open_with_network(&root.join("queue.sqlite3"), true)?;
    wait(|| {
        let rows = library.snapshot().unwrap();
        rows[0].id == first
            && rows[0].state == "Ratio reached"
            && rows[1].id == second
            && rows[1].state == "Seeding"
    });
    Ok(())
}

#[test]
fn restoring_existing_content_gives_it_a_seed_slot_when_downloads_are_full() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let (torrent, _, _) = write_fixture(root, root, "existing.bin", 16 * 1024)?;
    let database = root.join("library.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    library.set_library_limits(LibraryLimits {
        downloads: 1,
        seeds: 1,
        ..LibraryLimits::default()
    })?;
    let stalled = library.add(
        Source::Magnet(format!("magnet:?xt=urn:btih:{}", "1".repeat(40))),
        root,
    )?;
    let existing = library.add(Source::File(torrent), root)?;
    let ready = |library: &mut Library| {
        let rows = library.snapshot().unwrap();
        rows[0].id == stalled
            && rows[0].state == "Fetching metadata"
            && rows[1].id == existing
            && rows[1].state == "Seeding"
    };
    wait(|| ready(&mut library));
    library.checkpoint()?;
    drop(library);
    let mut library = Library::open_with_network(&database, true)?;
    wait(|| ready(&mut library));
    Ok(())
}

#[test]
fn force_recheck_verifies_files_while_preserving_a_manual_pause() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let (torrent, _, payload) = write_fixture(root, root, "checked.bin", 2 * 1024 * 1024)?;
    let mut library = Library::open_with_network(&root.join("recheck.sqlite3"), true)?;
    let id = library.add(Source::File(torrent), root)?;
    wait(|| library.snapshot().unwrap()[0].state == "Seeding");
    library.stop(id)?;
    let mut corrupted = payload;
    corrupted[0] ^= 1;
    fs::write(root.join("checked.bin"), &corrupted)?;
    library.force_recheck(id)?;
    wait(|| {
        let row = &library.snapshot().unwrap()[0];
        row.paused && row.progress > 0.5 && row.progress < 1.0 && row.remaining == 16 * 1024
    });
    assert_eq!(library.snapshot()?[0].state, "Paused");
    Ok(())
}

#[test]
fn watched_torrents_retry_failed_imports_and_stay_processed_across_restart() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let watch = root.join("watch");
    let other = root.join("other");
    let download = root.join("download");
    for folder in [&watch, &other, &download] {
        fs::create_dir(folder)?;
    }
    let (source, _, _) = write_fixture(root, root, "first.bin", 16 * 1024)?;
    let watched_file = watch.join("first.torrent");
    fs::copy(&source, &watched_file)?;
    write_fixture(root, root, "second.bin", 16 * 1024)?;
    fs::copy(&source, other.join("SECOND.TORRENT"))?;

    let database = root.join("library.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    library.set_watch_folders(vec![watch.clone(), other.clone()])?;
    library.db.execute_batch(
        "CREATE TRIGGER fail_import BEFORE INSERT ON transfers
         BEGIN SELECT RAISE(ABORT, 'test import failure'); END;",
    )?;
    library.scan_watch_folders(Some(&download), true);
    assert!(library.management().watch_problem.is_some());
    let processed: i64 =
        library
            .db
            .query_row("SELECT count(*) FROM processed_torrents", [], |row| {
                row.get(0)
            })?;
    assert_eq!(processed, 0);

    library.db.execute("DROP TRIGGER fail_import", [])?;
    let manual = library.add(Source::File(source), &download)?;
    library.scan_watch_folders(Some(&download), true);
    let rows = library.snapshot()?;
    assert_eq!(rows.len(), 2);
    let imported = rows.iter().find(|row| row.id != manual).unwrap();
    assert!(imported.paused);
    assert_eq!(imported.destination, download.to_str().unwrap());
    assert!(library.management().watch_problem.is_none());
    let imported_id = imported.id;
    drop(library);

    fs::copy(&watched_file, other.join("renamed.torrent"))?;
    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.management().watch_folders, [other, watch]);
    library.scan_watch_folders(Some(&download), false);
    assert_eq!(library.snapshot()?.len(), 2);
    library.remove(imported_id, false)?;
    library.remove(manual, false)?;
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    library.scan_watch_folders(Some(&download), false);
    assert!(library.snapshot()?.is_empty());
    assert!(library.management().watch_problem.is_none());
    Ok(())
}

#[test]
fn rss_rules_preview_and_commit_processed_entries_with_the_transfer() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let (torrent, hash, _) = write_fixture(root, root, "rss-release.bin", 16 * 1024)?;
    let feed = Subscription {
        url: "http://127.0.0.1/feed.xml".into(),
        include: "release, stable".into(),
        exclude: "sample, preview".into(),
    };
    let xml = format!(
        r#"<rss version="2.0"><channel>
        <item><title>Release &amp; Update</title><guid>123</guid><enclosure url="files/a.torrent" type="application/x-bittorrent"/></item>
        <item><title>Preview Release</title><link>magnet:?xt=urn:btih:{hash}</link></item>
        <item><title>Unrelated</title><link>magnet:?xt=urn:btih:{hash}</link></item>
        </channel></rss>"#
    );
    let items = feed.matches(xml.as_bytes())?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "Release & Update");
    assert_eq!(items[0].link, "http://127.0.0.1/files/a.torrent");

    let database = root.join("library.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    library.save_subscription("", feed.clone())?;
    library.db.execute_batch(
        "CREATE TRIGGER interrupt_rss BEFORE INSERT ON transfers
         BEGIN SELECT RAISE(ABORT, 'interrupted'); END;",
    )?;
    let source = fs::read(torrent)?;
    let import = |library: &mut Library| {
        library.import_feed_item(&feed.url, &items[0], Some(source.clone()), root, true)
    };
    assert!(import(&mut library).is_err());
    assert!(!library.feed_item_processed(&feed.url, &items[0])?);
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(
        library.management().subscriptions,
        std::slice::from_ref(&feed)
    );
    library.db.execute("DROP TRIGGER interrupt_rss", [])?;
    let id = import(&mut library)?.context("Expected one import")?;
    assert!(import(&mut library)?.is_none());
    assert!(library.snapshot()?[0].paused);
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.snapshot()?.len(), 1);
    assert!(library.feed_item_processed(&feed.url, &items[0])?);
    library.remove(id, false)?;
    assert!(import(&mut library)?.is_none());

    let new_items = feed.matches(
        format!(r#"<feed><entry><title>Stable release</title><id>alias</id><link href="magnet:?xt=urn:btih:{hash}"/></entry></feed>"#).as_bytes(),
    )?;
    assert!(
        library
            .import_feed_item(&feed.url, &new_items[0], None, root, false)?
            .is_none()
    );
    assert!(library.feed_item_processed(&feed.url, &new_items[0])?);
    assert!(library.snapshot()?.is_empty());
    Ok(())
}

#[test]
fn bandwidth_schedule_overrides_base_limits_and_survives_restart() -> Result<()> {
    use chrono::{Duration as Delta, Local};
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("library.sqlite3");
    let mut library = Library::open_with_network(&database, true)?;
    let base = Bandwidth {
        download: 12_288,
        upload: 4096,
    };
    library.set_library_limits(LibraryLimits {
        bandwidth: base,
        ..Default::default()
    })?;
    let now = Local::now().time();
    let schedule = BandwidthSchedule {
        start: now - Delta::hours(1),
        end: now + Delta::hours(1),
        bandwidth: Bandwidth {
            download: 2048,
            upload: 0,
        },
    };
    library.set_bandwidth_schedule(Some(schedule))?;
    assert_eq!(library.management().effective_bandwidth, schedule.bandwidth);
    drop(library);

    let mut library = Library::open_with_network(&database, true)?;
    assert_eq!(library.management().effective_bandwidth, schedule.bandwidth);
    library.set_bandwidth_schedule(Some(BandwidthSchedule {
        start: schedule.end,
        end: schedule.start,
        ..schedule
    }))?;
    assert_eq!(library.management().effective_bandwidth, base);
    library.set_bandwidth_schedule(None)?;
    assert!(library.management().bandwidth_schedule.is_none());
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum TorrentFormat {
    V1,
    V2,
    Hybrid,
}

// All fixture files are piece-aligned, with a power-of-two number of pieces per
// file. That keeps the hybrid layout identical and the v2 Merkle trees unpadded.
fn swarm_metainfo(format: TorrentFormat, name: &str, files: &[(&str, &[u8])]) -> (Vec<u8>, String) {
    const PIECE: usize = 16 * 1024;
    let v1 = !matches!(format, TorrentFormat::V2);
    let v2 = !matches!(format, TorrentFormat::V1);
    let mut info = b"d".to_vec();
    let mut layers = std::collections::BTreeMap::new();
    if v2 {
        info.extend(b"9:file treed");
        for &(file, content) in files {
            let pieces: Vec<[u8; 32]> = content
                .chunks(PIECE)
                .map(|p| Sha256::digest(p).into())
                .collect();
            assert!(content.len().is_multiple_of(PIECE) && pieces.len().is_power_of_two());
            let mut tree = pieces.clone();
            while tree.len() > 1 {
                tree = tree
                    .chunks_exact(2)
                    .map(|pair| Sha256::digest(pair.as_flattened()).into())
                    .collect();
            }
            info.extend(
                format!(
                    "{}:{file}d0:d6:lengthi{}e11:pieces root32:",
                    file.len(),
                    content.len()
                )
                .bytes(),
            );
            info.extend(tree[0]);
            info.extend(b"ee");
            if pieces.len() > 1 {
                layers.insert(tree[0], pieces);
            }
        }
        info.push(b'e');
    }
    if v1 {
        if let [(_, content)] = files {
            info.extend(format!("6:lengthi{}e", content.len()).bytes());
        } else {
            info.extend(b"5:filesl");
            for &(file, content) in files {
                info.extend(
                    format!("d6:lengthi{}e4:pathl{}:{file}ee", content.len(), file.len()).bytes(),
                );
            }
            info.push(b'e');
        }
    }
    if v2 {
        info.extend(b"12:meta versioni2e");
    }
    info.extend(format!("4:name{}:{name}12:piece lengthi{PIECE}e", name.len()).bytes());
    if v1 {
        let pieces = files.iter().flat_map(|(_, data)| data.chunks(PIECE));
        info.extend(format!("6:pieces{}:", pieces.clone().count() * 20).bytes());
        for piece in pieces {
            info.extend(Sha1::from(piece).digest().bytes());
        }
    }
    info.push(b'e');
    let mut link = "magnet:?".to_string();
    if v1 {
        link.push_str(&format!("xt=urn:btih:{}", Sha1::from(&info).digest()));
    }
    if v2 {
        if v1 {
            link.push('&');
        }
        link.push_str(&format!("xt=urn:btmh:1220{:x}", Sha256::digest(&info)));
    }
    let mut metainfo = b"d4:info".to_vec();
    metainfo.extend(info);
    if v2 {
        metainfo.extend(b"12:piece layersd");
        for (root, pieces) in layers {
            metainfo.extend(b"32:");
            metainfo.extend(root);
            metainfo.extend(format!("{}:", pieces.len() * 32).bytes());
            metainfo.extend(pieces.into_iter().flatten());
        }
        metainfo.push(b'e');
    }
    metainfo.push(b'e');
    (metainfo, link)
}

#[test]
fn download_restart_and_reseed() -> Result<()> {
    swarm_restart_and_reseed(TorrentFormat::V1)
}

#[test]
fn download_restart_and_reseed_v2() -> Result<()> {
    swarm_restart_and_reseed(TorrentFormat::V2)
}

#[test]
fn download_restart_and_reseed_hybrid() -> Result<()> {
    swarm_restart_and_reseed(TorrentFormat::Hybrid)
}

fn swarm_restart_and_reseed(format: TorrentFormat) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let seed_dir = root.join("seed");
    let second_seed_dir = root.join("second-seed");
    let download_dir = root.join("download");
    let peer_dir = root.join("peer");
    for dir in [&seed_dir, &second_seed_dir, &download_dir, &peer_dir] {
        fs::create_dir(dir)?;
    }
    let payload: Vec<u8> = (0..4 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let (metainfo, link) = swarm_metainfo(format, "data.bin", &[("data.bin", &payload)]);
    let torrent_path = root.join("test.torrent");
    fs::write(&torrent_path, metainfo)?;
    fs::write(seed_dir.join("data.bin"), &payload)?;

    let mut seed = Library::open_with_network(&root.join("seed.sqlite3"), true)?;
    seed.add(Source::File(torrent_path.clone()), &seed_dir)?;
    wait(|| seed.snapshot().unwrap()[0].state == "Seeding");
    let seed_port = seed.engine.listen_port();
    ensure!(seed_port != 0, "The loopback seeder could not bind a port");
    fs::write(second_seed_dir.join("data.bin"), &payload)?;
    let mut second_seed = Library::open_with_network(&root.join("second-seed.sqlite3"), true)?;
    second_seed.add(Source::File(torrent_path.clone()), &second_seed_dir)?;
    wait(|| second_seed.snapshot().unwrap()[0].state == "Seeding");
    let second_port = second_seed.engine.listen_port();
    ensure!(
        second_port != 0,
        "The second loopback seeder could not bind a port"
    );

    let database = root.join("download.sqlite3");
    let mut download = Library::open_with_network(&database, true)?;
    let id = download.add(Source::Magnet(link), &download_dir)?;
    assert!(download.pieces(id)?.cells.is_empty());
    assert!(download.peers(id)?.is_empty());
    download.engine.pin_mut().connect_local(id, seed_port)?;
    download.engine.pin_mut().connect_local(id, second_port)?;
    wait(|| {
        let progress = download.snapshot().unwrap()[0].progress;
        assert!(
            progress < 1.0,
            "The first session must finish only part of the download"
        );
        let pieces = download.pieces(id).unwrap();
        let peers = download.peers(id).unwrap();
        progress > 0.05
            && pieces.have > 0
            && pieces.have < pieces.total
            && [seed_port, second_port].into_iter().all(|port| {
                peers.iter().any(|peer| {
                    peer.endpoint == format!("127.0.0.1:{port}")
                        && peer.seed
                        && peer.transport == "TCP"
                        && peer.downloaded > 0
                })
            })
    });
    let pieces = download.pieces(id)?;
    assert_eq!(pieces.piece_size, 16 * 1024);
    assert_eq!(pieces.total as usize, payload.len() / (16 * 1024));
    assert_eq!(pieces.cells.len(), pieces.total as usize);
    assert_eq!(
        pieces.cells.iter().filter(|&&cell| cell == 2).count(),
        pieces.have as usize
    );
    assert!(pieces.cells.contains(&0));
    download.set_paused(id, true)?;
    download.stop(id)?;
    let saved_pieces = download.pieces(id)?.have;
    assert!(saved_pieces > 0);
    assert!(download.peers(id)?.is_empty());
    drop(download);
    drop(second_seed);

    {
        let mut db = Connection::open(&database)?;
        let interrupted = db.transaction()?;
        interrupted.execute("UPDATE transfers SET resume = X'' WHERE id = ?1", [id])?;
        // Dropping the transaction abandons the write before commit.
    }
    let mut download = Library::open_with_network(&database, true)?;
    wait(|| {
        let transfers = download.engine.pin_mut().snapshot().unwrap();
        transfers[0].state != "Checking files"
    });
    let restored = &download.snapshot()?[0];
    assert!(restored.paused, "Restored state: {}", restored.state);
    assert!(
        download.pieces(id)?.have > 0,
        "Lost {saved_pieces} saved pieces ({})",
        restored.state
    );
    download.set_paused(id, false)?;
    download.engine.pin_mut().connect_local(id, seed_port)?;
    wait(|| download.snapshot().unwrap()[0].state == "Seeding");
    let pieces = download.pieces(id)?;
    assert_eq!(pieces.have, pieces.total);
    assert!(pieces.cells.iter().all(|&cell| cell == 2));
    download.checkpoint()?;
    let downloaded = fs::read(download_dir.join("data.bin"))?;
    ensure!(
        downloaded == payload,
        "Downloaded bytes differ ({} of {} bytes; first mismatch at {:?})",
        downloaded.len(),
        payload.len(),
        downloaded
            .iter()
            .zip(&payload)
            .position(|(actual, expected)| actual != expected)
    );

    drop(seed);
    let mut peer = Library::open_with_network(&root.join("peer.sqlite3"), true)?;
    let peer_id = peer.add(Source::File(torrent_path), &peer_dir)?;
    peer.engine
        .pin_mut()
        .connect_local(peer_id, download.engine.listen_port())?;
    wait(|| peer.snapshot().unwrap()[0].state == "Seeding");
    peer.checkpoint()?;
    ensure!(
        fs::read(peer_dir.join("data.bin"))? == payload,
        "Reseeded bytes differ"
    );
    Ok(())
}

fn press_enter(instruction: &str) -> Result<()> {
    println!("{instruction} Press Enter here to continue.");
    ensure!(
        io::stdin().read_line(&mut String::new())? != 0,
        "The desktop acceptance check needs interactive input"
    );
    Ok(())
}

#[test]
fn select_files_from_torrent_and_magnet() -> Result<()> {
    swarm_select_files(TorrentFormat::V1)
}

#[test]
fn select_files_from_torrent_and_magnet_v2() -> Result<()> {
    swarm_select_files(TorrentFormat::V2)
}

#[test]
fn select_files_from_torrent_and_magnet_hybrid() -> Result<()> {
    swarm_select_files(TorrentFormat::Hybrid)
}

fn swarm_select_files(format: TorrentFormat) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let seed_dir = root.join("seed");
    fs::create_dir_all(seed_dir.join("test"))?;
    let wanted = vec![37; 32 * 1024];
    let skipped = vec![91; 32 * 1024];
    fs::write(seed_dir.join("test/keep.bin"), &wanted)?;
    fs::write(seed_dir.join("test/skip.bin"), &skipped)?;
    let (metainfo, link) = swarm_metainfo(
        format,
        "test",
        &[("keep.bin", &wanted), ("skip.bin", &skipped)],
    );
    let torrent = root.join("test.torrent");
    fs::write(&torrent, metainfo)?;

    let mut seed = Library::open_with_network(&root.join("seed.sqlite3"), true)?;
    seed.add(Source::File(torrent.clone()), &seed_dir)?;
    wait(|| seed.snapshot().unwrap()[0].state == "Seeding");
    let port = seed.engine.listen_port();
    let preview_dir = root.join("preview");
    fs::create_dir(&preview_dir)?;
    for mode in ["file", "magnet"] {
        let destination = root.join(mode);
        fs::create_dir(&destination)?;
        let db = root.join(format!("{mode}.sqlite3"));
        let mut library = Library::open_with_network(&db, true)?;
        let source = if mode == "file" {
            PreparedSource::new(&Source::File(torrent.clone()))?
        } else {
            library.fetch_metadata(&link, &preview_dir)?;
            library.engine.pin_mut().connect_local(0, port)?;
            assert!(library.snapshot()?.is_empty());
            let mut prepared = None;
            wait(|| {
                prepared = library.poll_metadata(&link).unwrap();
                prepared.is_some()
            });
            assert!(preview_dir.read_dir()?.next().is_none());
            prepared.unwrap()
        };
        let skip = source
            .files
            .iter()
            .find(|file| file.path.ends_with("skip.bin"))
            .unwrap()
            .index;
        ensure!(source.files.len() == 2);
        let id = library.add_prepared(&source, &destination, true, &[skip])?;
        let keep = source
            .files
            .iter()
            .find(|file| file.index != skip)
            .unwrap()
            .index;
        assert!(library.set_file_priority(id, keep, 0).is_err());
        library.set_file_priority(id, keep, 7)?;
        drop(library);

        let mut library = Library::open_with_network(&db, true)?;
        assert_eq!(library.snapshot()?[0].state, "Paused");
        let files = library.files(id)?;
        assert_eq!(files.len(), 2);
        assert!(
            files
                .iter()
                .any(|file| file.index == skip && !file.selected)
        );
        assert!(
            files
                .iter()
                .any(|file| file.index == keep && file.selected && file.priority == 7)
        );
        library.set_paused(id, false)?;
        library.engine.pin_mut().connect_local(id, port)?;
        wait(|| library.snapshot().unwrap()[0].state == "Seeding");
        let downloaded = destination.join("test/keep.bin");
        assert_eq!(fs::read(&downloaded)?, wanted);
        assert!(!destination.join("test/skip.bin").exists());
        library.remove(id, false)?;
        assert_eq!(fs::read(&downloaded)?, wanted);
        assert!(library.snapshot()?.is_empty());

        let id = library.add_prepared(&source, &destination, true, &[skip])?;
        library.remove(id, true)?;
        assert!(!downloaded.exists());
        drop(library);
        assert!(
            Library::open_with_network(&db, true)?
                .snapshot()?
                .is_empty()
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires using the Fero desktop window"]
fn desktop_acceptance() -> Result<()> {
    let desktop_port = std::env::var("FERO_DESKTOP_PORT")
        .unwrap_or_else(|_| "6881".into())
        .parse::<std::num::NonZeroU16>()
        .context("FERO_DESKTOP_PORT must be a valid TCP port")?
        .get();
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let seed_dir = root.join("seed");
    let download_dir = root.join("download");
    let peer_dir = root.join("peer");
    for dir in [&seed_dir, &download_dir, &peer_dir] {
        fs::create_dir(dir)?;
    }
    let name = "fero-phase-1.bin";
    let (torrent, hash, payload) = write_fixture(root, &seed_dir, name, 12 * 1024 * 1024)?;
    let seed_database = root.join("seed.sqlite3");
    let mut seed = Library::open_with_network(&seed_database, true)?;
    let seed_id = seed.add(Source::File(torrent.clone()), &seed_dir)?;
    wait(|| seed.snapshot().unwrap()[0].state == "Seeding");
    ensure!(
        seed.engine.listen_port() != 0,
        "The loopback seeder could not bind a port"
    );

    println!("Set FERO_DATA_DIR to {}", root.join("library").display());
    println!("Use {} or this magnet.", torrent.display());
    println!("magnet:?xt=urn:btih:{hash}");
    println!("Set Save to to {}.", download_dir.display());
    press_enter("Launch Fero and click Add Torrent.")?;
    seed.engine.pin_mut().connect_local(seed_id, desktop_port)?;
    press_enter("Watch the progress row and quit Fero while the download is partway done.")?;
    let downloaded = download_dir.join(name);
    let partial = fs::read(&downloaded).context("Fero did not write the test download")?;
    ensure!(
        partial != payload && partial.iter().any(|&byte| byte != 0),
        "Quit Fero after it has written some data and before the download completes"
    );
    drop(seed);

    press_enter(
        "Reopen Fero with the same library and confirm that the transfer shows saved progress.",
    )?;
    let mut seed = Library::open_with_network(&seed_database, true)?;
    wait(|| seed.snapshot().unwrap()[0].state == "Seeding");
    seed.engine.pin_mut().connect_local(seed_id, desktop_port)?;
    press_enter("Wait for the row to say Seeding and leave Fero running.")?;
    ensure!(fs::read(downloaded)? == payload, "Downloaded bytes differ");

    drop(seed);
    let mut peer = Library::open_with_network(&root.join("peer.sqlite3"), true)?;
    let peer_id = peer.add(Source::File(torrent), &peer_dir)?;
    peer.engine.pin_mut().connect_local(peer_id, desktop_port)?;
    wait(|| peer.snapshot().unwrap()[0].state == "Seeding");
    peer.checkpoint()?;
    ensure!(
        fs::read(peer_dir.join(name))? == payload,
        "Reseeded bytes differ"
    );
    println!("The download and second peer both match the source byte for byte.");
    press_enter("Quit Fero to release the temporary library.")?;
    Ok(())
}
