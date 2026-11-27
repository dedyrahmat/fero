use crate::{BandwidthSchedule, Subscription, automation};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bandwidth {
    pub download: i32,
    pub upload: i32,
}

impl Bandwidth {
    pub(crate) fn validate(self) -> Result<()> {
        ensure!(
            self.download >= 0 && self.upload >= 0,
            "Speed caps must be nonnegative"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LibraryLimits {
    pub bandwidth: Bandwidth,
    pub downloads: u32,
    pub seeds: u32,
    pub seed_ratio: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkSettings {
    pub listen_port: u16,
    pub connections: i32,
    pub socks5: Option<SocketAddr>,
    pub vpn_interface: String,
    pub vpn_dns: Option<SocketAddr>,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            listen_port: 6881,
            connections: 300,
            socks5: None,
            vpn_interface: String::new(),
            vpn_dns: None,
        }
    }
}

impl NetworkSettings {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.connections > 0,
            "Peer connection limit must be positive"
        );
        if let Some(proxy) = self.socks5 {
            ensure!(
                proxy.port() != 0 && !proxy.ip().is_unspecified() && !proxy.ip().is_multicast(),
                "Enter a SOCKS5 IP address and a port from 1 to 65535"
            );
        }
        if !self.vpn_interface.is_empty() {
            ensure!(
                self.socks5.is_none(),
                "Choose either a VPN interface or SOCKS5"
            );
            ensure!(
                self.vpn_interface.len() < 64
                    && self.vpn_interface.parse::<std::net::IpAddr>().is_err()
                    && self
                        .vpn_interface
                        .bytes()
                        .all(|byte| { byte.is_ascii_alphanumeric() || b"_.-{}".contains(&byte) }),
                "Enter a network interface name or Windows adapter GUID"
            );
        }
        if let Some(dns) = self.vpn_dns {
            ensure!(
                !self.vpn_interface.is_empty(),
                "Choose an interface for VPN DNS"
            );
            ensure!(
                dns.port() != 0 && !dns.ip().is_unspecified() && !dns.ip().is_multicast(),
                "Enter a numeric VPN DNS address"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferOptions {
    pub category: String,
    pub tags: String,
    pub bandwidth: Bandwidth,
    /// Hundredths of a ratio. None uses the library default. Zero disables the goal.
    pub seed_ratio: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Management {
    pub limits: LibraryLimits,
    pub network: NetworkSettings,
    pub active_network: NetworkSettings,
    pub network_restart_required: bool,
    pub network_problem: Option<String>,
    pub watch_folders: Vec<PathBuf>,
    pub watch_problem: Option<String>,
    pub subscriptions: Vec<Subscription>,
    pub bandwidth_schedule: Option<BandwidthSchedule>,
    pub effective_bandwidth: Bandwidth,
    pub queue: Vec<i64>,
    pub options: HashMap<i64, TransferOptions>,
}

impl Management {
    pub(crate) fn load(db: &Connection) -> Result<Self> {
        db.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS library_limits (
                id INTEGER PRIMARY KEY CHECK (id = 1), download INTEGER NOT NULL,
                upload INTEGER NOT NULL, downloads INTEGER NOT NULL, seeds INTEGER NOT NULL,
                seed_ratio INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO library_limits VALUES (1, 0, 0, 0, 0, 0);
             CREATE TABLE IF NOT EXISTS network_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                listen_port INTEGER NOT NULL CHECK (listen_port BETWEEN 0 AND 65535),
                connections INTEGER NOT NULL CHECK (connections > 0)
             );
             CREATE TABLE IF NOT EXISTS network_proxy (
                id INTEGER PRIMARY KEY CHECK (id = 1), endpoint TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS network_vpn (
                id INTEGER PRIMARY KEY CHECK (id = 1), interface TEXT NOT NULL, dns TEXT
             );
             CREATE TABLE IF NOT EXISTS watch_folders (path TEXT PRIMARY KEY NOT NULL);
             CREATE TABLE IF NOT EXISTS transfer_options (
                id INTEGER PRIMARY KEY REFERENCES transfers(id) ON DELETE CASCADE,
                position INTEGER, category TEXT NOT NULL DEFAULT '', tags TEXT NOT NULL DEFAULT '',
                download INTEGER NOT NULL DEFAULT 0, upload INTEGER NOT NULL DEFAULT 0,
                seed_ratio INTEGER
             );",
        )?;
        let (subscriptions, bandwidth_schedule) = automation::load(db)?;
        let watch_folders = db
            .prepare("SELECT path FROM watch_folders ORDER BY path")?
            .query_map([], |row| row.get::<_, String>(0).map(PathBuf::from))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut network = db
            .query_row(
                "SELECT listen_port, connections FROM network_settings WHERE id = 1",
                [],
                |row| {
                    Ok(NetworkSettings {
                        listen_port: row.get(0)?,
                        connections: row.get(1)?,
                        socks5: None,
                        vpn_interface: String::new(),
                        vpn_dns: None,
                    })
                },
            )
            .optional()?
            .unwrap_or_default();
        network.socks5 = db
            .query_row(
                "SELECT endpoint FROM network_proxy WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|endpoint| endpoint.parse())
            .transpose()?;
        let (vpn_interface, vpn_dns) = db
            .query_row(
                "SELECT interface, dns FROM network_vpn WHERE id = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?
            .unwrap_or_default();
        network.vpn_interface = vpn_interface;
        network.vpn_dns = vpn_dns.map(|endpoint| endpoint.parse()).transpose()?;
        network.validate()?;
        let limits = db.query_row(
            "SELECT download, upload, downloads, seeds, seed_ratio FROM library_limits WHERE id = 1",
            [],
            |row| Ok(LibraryLimits {
                bandwidth: Bandwidth { download: row.get(0)?, upload: row.get(1)? },
                downloads: row.get(2)?,
                seeds: row.get(3)?,
                seed_ratio: row.get(4)?,
            }),
        )?;
        let mut statement = db.prepare(
            "SELECT t.id, o.category, o.tags, o.download, o.upload, o.seed_ratio
             FROM transfers t LEFT JOIN transfer_options o ON o.id = t.id
             ORDER BY o.position IS NULL, o.position, t.id",
        )?;
        let mut rows = statement.query([])?;
        let mut management = Self {
            limits,
            active_network: network.clone(),
            network,
            watch_folders,
            subscriptions,
            bandwidth_schedule,
            ..Self::default()
        };
        while let Some(row) = rows.next()? {
            let id = row.get(0)?;
            management.queue.push(id);
            if let Some(category) = row.get(1)? {
                management.options.insert(
                    id,
                    TransferOptions {
                        category,
                        tags: row.get(2)?,
                        bandwidth: Bandwidth {
                            download: row.get(3)?,
                            upload: row.get(4)?,
                        },
                        seed_ratio: row.get(5)?,
                    },
                );
            }
        }
        Ok(management)
    }

    pub(crate) fn save_network(db: &mut Connection, network: &NetworkSettings) -> Result<()> {
        let transaction = db.transaction()?;
        transaction.execute(
            "INSERT INTO network_settings VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE
             SET listen_port = excluded.listen_port, connections = excluded.connections",
            params![network.listen_port, network.connections],
        )?;
        if let Some(proxy) = network.socks5 {
            transaction.execute(
                "INSERT INTO network_proxy VALUES (1, ?1) ON CONFLICT(id) DO UPDATE
                 SET endpoint = excluded.endpoint",
                [proxy.to_string()],
            )?;
        } else {
            transaction.execute("DELETE FROM network_proxy WHERE id = 1", [])?;
        }
        transaction.execute(
            "INSERT INTO network_vpn VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE
             SET interface = excluded.interface, dns = excluded.dns",
            params![
                network.vpn_interface,
                network.vpn_dns.map(|dns| dns.to_string())
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn save_limits(db: &Connection, limits: LibraryLimits) -> Result<()> {
        db.execute(
            "UPDATE library_limits SET download = ?1, upload = ?2, downloads = ?3,
             seeds = ?4, seed_ratio = ?5 WHERE id = 1",
            params![
                limits.bandwidth.download,
                limits.bandwidth.upload,
                limits.downloads,
                limits.seeds,
                limits.seed_ratio
            ],
        )?;
        Ok(())
    }

    pub(crate) fn save_options(db: &Connection, id: i64, options: &TransferOptions) -> Result<()> {
        db.execute(
            "INSERT INTO transfer_options (id, category, tags, download, upload, seed_ratio)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(id) DO UPDATE SET
             category = excluded.category, tags = excluded.tags, download = excluded.download,
             upload = excluded.upload, seed_ratio = excluded.seed_ratio",
            params![
                id,
                options.category,
                options.tags,
                options.bandwidth.download,
                options.bandwidth.upload,
                options.seed_ratio
            ],
        )?;
        Ok(())
    }

    pub(crate) fn save_queue(db: &mut Connection, queue: &[i64]) -> Result<()> {
        let transaction = db.transaction()?;
        {
            let mut statement = transaction.prepare_cached(
                "INSERT INTO transfer_options (id, position) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET position = excluded.position",
            )?;
            for (position, id) in queue.iter().enumerate() {
                statement.execute(params![id, position])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}
