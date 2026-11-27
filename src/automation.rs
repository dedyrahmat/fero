use crate::{Bandwidth, Library, PreparedSource, Source, ffi};
use anyhow::{Context, Result, ensure};
use chrono::{Local, NaiveTime, Timelike};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    pub url: String,
    pub include: String,
    pub exclude: String,
}

#[derive(Debug)]
pub struct FeedItem {
    pub title: String,
    pub link: String,
    key: String,
}

impl Subscription {
    pub fn validate(mut self) -> Result<Self> {
        self.url = feed_url(&self.url)?.to_string();
        Ok(self)
    }

    pub fn matches(&self, bytes: &[u8]) -> Result<Vec<FeedItem>> {
        let url = feed_url(&self.url)?;
        let xml = std::str::from_utf8(bytes).context("Feed must use UTF-8")?;
        let doc = roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                nodes_limit: 100_000,
                ..Default::default()
            },
        )
        .context("Read RSS or Atom feed")?;
        ensure!(
            matches!(doc.root_element().tag_name().name(), "rss" | "RDF" | "feed"),
            "Expected an RSS or Atom feed"
        );
        let terms = |rule: &str| {
            rule.split(',')
                .map(|term| term.trim().to_lowercase())
                .filter(|term| !term.is_empty())
                .collect::<Vec<_>>()
        };
        let include = terms(&self.include);
        let exclude = terms(&self.exclude);
        let mut items = Vec::new();
        for item in doc
            .root_element()
            .descendants()
            .filter(|node| node.is_element() && matches!(node.tag_name().name(), "item" | "entry"))
        {
            let children = || item.children().filter(roxmltree::Node::is_element);
            let text = |names: &[&str]| {
                children()
                    .find(|node| names.contains(&node.tag_name().name()))
                    .and_then(|node| node.text())
                    .unwrap_or("")
                    .trim()
            };
            let title = text(&["title"]);
            let candidate = |node: roxmltree::Node<'_, '_>| {
                let link = node
                    .attribute("url")
                    .or_else(|| node.attribute("href"))
                    .or_else(|| node.text())?
                    .trim();
                if link.starts_with("magnet:?") {
                    return Some(link.to_owned());
                }
                let url = url.join(link).ok()?;
                let torrent = node.attribute("type") == Some("application/x-bittorrent")
                    || url.path().to_ascii_lowercase().ends_with(".torrent");
                (torrent && feed_url(url.as_str()).is_ok()).then(|| url.to_string())
            };
            let Some(link) = children()
                .filter(|node| matches!(node.tag_name().name(), "enclosure" | "link" | "magnetURI"))
                .find_map(candidate)
            else {
                continue;
            };
            let title = if title.is_empty() { &link } else { title };
            let searchable = title.to_lowercase();
            if (include.is_empty() || include.iter().any(|word| searchable.contains(word)))
                && !exclude.iter().any(|word| searchable.contains(word))
            {
                let id = text(&["guid", "id"]);
                items.push(FeedItem {
                    title: title.to_owned(),
                    key: if id.is_empty() {
                        link.clone()
                    } else {
                        id.to_owned()
                    },
                    link,
                });
            }
        }
        Ok(items)
    }
}

pub(crate) fn feed_url(url: &str) -> Result<Url> {
    let mut url = Url::parse(url).context("Enter a valid feed URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none(),
        "Enter an HTTP or HTTPS URL without embedded credentials"
    );
    url.set_fragment(None);
    Ok(url)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BandwidthSchedule {
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub bandwidth: Bandwidth,
}

impl BandwidthSchedule {
    fn active(self, now: NaiveTime) -> bool {
        if self.start < self.end {
            self.start <= now && now < self.end
        } else {
            now >= self.start || now < self.end
        }
    }
}

pub(crate) fn scheduled_bandwidth(
    normal: Bandwidth,
    schedule: Option<BandwidthSchedule>,
) -> Bandwidth {
    schedule
        .filter(|schedule| schedule.active(Local::now().time()))
        .map_or(normal, |schedule| schedule.bandwidth)
}

pub(crate) fn load(db: &Connection) -> Result<(Vec<Subscription>, Option<BandwidthSchedule>)> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS subscriptions (
            url TEXT PRIMARY KEY NOT NULL, include TEXT NOT NULL, exclude TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS processed_feed_items (
            url TEXT NOT NULL, item TEXT NOT NULL, PRIMARY KEY (url, item)
        );
        CREATE TABLE IF NOT EXISTS bandwidth_schedule (
            id INTEGER PRIMARY KEY CHECK (id = 1), start INTEGER NOT NULL, end INTEGER NOT NULL,
            download INTEGER NOT NULL, upload INTEGER NOT NULL
        );",
    )?;
    let feeds = db
        .prepare("SELECT url, include, exclude FROM subscriptions ORDER BY url")?
        .query_map([], |row| {
            Ok(Subscription {
                url: row.get(0)?,
                include: row.get(1)?,
                exclude: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let schedule = db
        .query_row(
            "SELECT start, end, download, upload FROM bandwidth_schedule WHERE id = 1",
            [],
            |row| {
                let seconds = |i| {
                    NaiveTime::from_num_seconds_from_midnight_opt(row.get(i)?, 0)
                        .ok_or(rusqlite::Error::InvalidQuery)
                };
                Ok(BandwidthSchedule {
                    start: seconds(0)?,
                    end: seconds(1)?,
                    bandwidth: Bandwidth {
                        download: row.get(2)?,
                        upload: row.get(3)?,
                    },
                })
            },
        )
        .optional()?;
    Ok((feeds, schedule))
}

impl Library {
    pub fn save_subscription(&mut self, previous: &str, feed: Subscription) -> Result<()> {
        let feed = feed.validate()?;
        let transaction = self.db.transaction()?;
        if previous != feed.url {
            transaction.execute("DELETE FROM subscriptions WHERE url = ?1", [previous])?;
        }
        transaction.execute(
            "INSERT INTO subscriptions VALUES (?1, ?2, ?3)
             ON CONFLICT(url) DO UPDATE SET include = excluded.include, exclude = excluded.exclude",
            params![feed.url, feed.include, feed.exclude],
        )?;
        transaction.commit()?;
        self.reload_automation()
    }

    pub fn remove_subscription(&mut self, url: &str) -> Result<()> {
        self.db
            .execute("DELETE FROM subscriptions WHERE url = ?1", [url])?;
        self.reload_automation()
    }

    pub fn set_bandwidth_schedule(&mut self, schedule: Option<BandwidthSchedule>) -> Result<()> {
        if let Some(schedule) = schedule {
            schedule.bandwidth.validate()?;
            ensure!(
                schedule.start != schedule.end,
                "Choose different start and end times"
            );
            self.db.execute(
                "REPLACE INTO bandwidth_schedule VALUES (1, ?1, ?2, ?3, ?4)",
                params![
                    schedule.start.num_seconds_from_midnight(),
                    schedule.end.num_seconds_from_midnight(),
                    schedule.bandwidth.download,
                    schedule.bandwidth.upload
                ],
            )?;
        } else {
            self.db.execute("DELETE FROM bandwidth_schedule", [])?;
        }
        self.reload_automation()?;
        self.apply_bandwidth_schedule()
    }

    fn reload_automation(&mut self) -> Result<()> {
        let (feeds, schedule) = load(&self.db)?;
        let management = std::sync::Arc::make_mut(&mut self.management);
        management.subscriptions = feeds;
        management.bandwidth_schedule = schedule;
        Ok(())
    }

    pub fn feed_item_processed(&self, feed: &str, item: &FeedItem) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM processed_feed_items WHERE url = ?1 AND item = ?2)",
            params![feed, item.key],
            |row| row.get(0),
        )?)
    }

    pub fn import_feed_item(
        &mut self,
        feed: &str,
        item: &FeedItem,
        bytes: Option<Vec<u8>>,
        destination: &Path,
        paused: bool,
    ) -> Result<Option<i64>> {
        let source = match bytes {
            Some(bytes) => PreparedSource::from_torrent_bytes(bytes)?,
            None => PreparedSource::new(&Source::Magnet(item.link.clone()))?,
        };
        let hashes = ffi::torrent_hashes(&source.bytes, source.magnet)?;
        ensure!(
            !hashes.v1.is_empty() || !hashes.v2.is_empty(),
            "Missing torrent info hash"
        );
        self.add_transfer(
            &source,
            destination,
            paused,
            &[],
            Some(hashes),
            Some((feed, &item.key)),
            false,
        )
    }
}
