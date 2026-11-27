use crate::{Event, Preferences};
use anyhow::{Context, Result};
use fero::{FeedItem, FeedRequest, Library, Subscription};
use std::{
    collections::VecDeque,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

enum Job {
    Feed(Subscription),
    Preview(Subscription),
    Torrent(Subscription, FeedItem),
}

pub struct RssWorker {
    next_scan: Instant,
    jobs: VecDeque<Job>,
    pending: Option<(Job, FeedRequest)>,
}

impl RssWorker {
    pub fn new() -> Self {
        Self {
            next_scan: Instant::now(),
            jobs: VecDeque::new(),
            pending: None,
        }
    }

    pub fn rescan(&mut self) {
        self.next_scan = Instant::now();
        self.jobs.retain(|job| matches!(job, Job::Preview(_)));
    }

    pub fn preview(&mut self, feed: Subscription) {
        self.jobs.push_front(Job::Preview(feed));
    }

    pub fn tick(&mut self, library: &mut Library, prefs: &Preferences, events: &Sender<Event>) {
        let management = library.management();
        if self.pending.is_none() && self.jobs.is_empty() && Instant::now() >= self.next_scan {
            self.jobs
                .extend(management.subscriptions.iter().cloned().map(Job::Feed));
            self.next_scan = Instant::now() + Duration::from_secs(300);
        }

        if let Some((job, mut request)) = self.pending.take() {
            match request.poll() {
                Ok(None) => self.pending = Some((job, request)),
                result => {
                    if let Err(error) = self.finish(job, result, library, prefs, events) {
                        let _ = events.send(Event::Error(format!("RSS import: {error:#}")));
                    }
                }
            }
            return;
        }

        let Some(job) = self.jobs.pop_front() else {
            return;
        };
        let url = match &job {
            Job::Feed(feed) if management.subscriptions.contains(feed) => &feed.url,
            Job::Preview(feed) => &feed.url,
            Job::Torrent(feed, item) if management.subscriptions.contains(feed) => {
                match library.feed_item_processed(&feed.url, item) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        let _ = events.send(Event::Error(format!("RSS import: {error:#}")));
                        return;
                    }
                }
                if item.link.starts_with("magnet:?") {
                    if let Err(error) = Self::import(library, prefs, feed, item, None) {
                        let _ = events.send(Event::Error(format!("RSS import: {error:#}")));
                    }
                    return;
                }
                &item.link
            }
            _ => return,
        };
        match library.request_feed(url) {
            Ok(request) => self.pending = Some((job, request)),
            Err(error) => {
                if let Err(error) = self.finish(job, Err(error), library, prefs, events) {
                    let _ = events.send(Event::Error(format!("RSS import: {error:#}")));
                }
            }
        }
    }

    fn finish(
        &mut self,
        job: Job,
        result: Result<Option<Vec<u8>>>,
        library: &mut Library,
        prefs: &Preferences,
        events: &Sender<Event>,
    ) -> Result<()> {
        let bytes = result.and_then(|bytes| bytes.context("Missing HTTP response"));
        let management = library.management();
        match job {
            Job::Preview(feed) => {
                let preview = bytes.and_then(|bytes| feed.matches(&bytes));
                let _ = events.send(Event::FeedPreview(
                    preview.map_err(|error| format!("{error:#}")),
                ));
            }
            Job::Feed(feed) if management.subscriptions.contains(&feed) => {
                let items = bytes
                    .and_then(|bytes| feed.matches(&bytes))
                    .context(feed.url.clone())?;
                for item in items.into_iter().rev() {
                    if !library.feed_item_processed(&feed.url, &item)? {
                        self.jobs.push_front(Job::Torrent(feed.clone(), item));
                    }
                }
            }
            Job::Torrent(feed, item) if management.subscriptions.contains(&feed) => {
                Self::import(library, prefs, &feed, &item, Some(bytes?))?;
            }
            _ => {}
        }
        Ok(())
    }

    fn import(
        library: &mut Library,
        prefs: &Preferences,
        feed: &Subscription,
        item: &FeedItem,
        bytes: Option<Vec<u8>>,
    ) -> Result<()> {
        let destination = prefs
            .download_folder()
            .context("Choose a default save folder")?;
        library
            .import_feed_item(&feed.url, item, bytes, &destination, prefs.start_paused)
            .with_context(|| format!("Import {}", item.title))?;
        Ok(())
    }
}
