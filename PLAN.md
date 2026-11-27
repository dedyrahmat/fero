# Fero — a native torrent client for daily use

Build Fero in Rust with GPUI for macOS, Windows, and Linux. Start development and verification on macOS. The first release should let someone paste a magnet link, see where the files will go, and choose whether to review them before adding. Someone managing hundreds of torrents should be able to find a stalled transfer, inspect its trackers, change its priority, and close the window while everything keeps running.

This guide defines the first release. Track implementation and platform verification in the [first-release roadmap](docs/development.md#first-release-roadmap).

## Desktop experience

Use the **Library**, **Settings**, and **Add Torrent** screens in the interactive desktop mockup as the Phase 2 design reference. Build them in GPUI with each platform’s native window controls and folder picker. [Fero desktop mockup](design/fero-desktop.html)

- **Window layout and Focus mode.** Arrange the sidebar, transfer list, and selected-transfer inspector from left to right. Keep search and one prominent **Add Torrent** action above the list. Show the visible transfer count and total download and upload rates in the footer. The sidebar toggle beside the Fero mark collapses it into an icon rail, with status filters and Settings still available. Move the inspector below the list at narrow widths and wrap the expanded filters as needed.
- **Transfer list and appearance.** Round the corners of rows, panels, inputs, and dialogs, and use pill-shaped actions. Show the name, state, size, progress bar and percentage, transfer rate, and time remaining where available. Sort by recently added, name, progress, or rate. Pair text with color and distinguish a fresh library, no search results, waiting for peers, paused transfers, and actionable errors such as disk full. Use cool blue surfaces by default with Harbor Current, warm graphite and parchment with Ember, Instrument Sans headings, DM Sans interface text, and tabular rate and progress figures. Make keyboard focus visible. Put *FEH-roh* and the Latin meaning “to carry, bring, bear” in About.
- **Settings.** Save System, Light, or Dark appearance, Harbor Current, Ember, Sage, or Indigo palette, Roomy or Compact rows, and the sidebar’s current state. Save the default download folder and whether new magnets open for review or start immediately. Use Downloads and review first by default. Offer a default **Start paused** preference. Apply download defaults to each new Add Torrent dialog, where users can override them. Preserve these choices across relaunch.
- **Add Torrent and file review.** Accept pasted and OS-opened magnets, a native torrent file picker, and `.torrent` drops on the window or the highlighted drop zone. Local torrent files open for review. For magnets, fetch metadata from peers without downloading payload files until the user confirms, or offer **Start immediately**. After metadata arrives, show filenames, subfolders, sizes, and Selected or Skipped state. Let users preview each file’s full save path, search larger file lists, select or clear files, and see the selected count and total size. Let users choose or type a destination and see the preview update before confirming. Keep the source, selection, and destination available on validation or engine errors. Persist the selected files, destination, and pause state with the transfer.
- **Inspector and transfer actions.** Phase 2 delivers **Overview** with state, progress, rates, recent download activity, time remaining, save path, and pause or resume, plus a read-only **Files** tab showing the selected and skipped files. Phase 3 adds file priority editing, **Peers**, **Trackers**, reannounce, and recheck. **Remove from Fero** keeps downloaded files by default. Deleting content has its own confirmed action.

## Engine and daily-use features

1. **Prove the transfer path first.** Build one working GPUI window that adds a torrent, downloads from a local seeder, quits, and resumes. Verify this on macOS before expanding the interface. Pin GPUI and embed libtorrent 2.1.2 through a small `cxx` bridge. Keep networking and disk work off the UI thread. [GPUI platform guide](https://github.com/zed-industries/zed/tree/main/crates/gpui) · [libtorrent release](https://github.com/arvidn/libtorrent/releases/tag/v2.1.2) · [CXX](https://cxx.rs/)
2. **Manage the library.** Add multi-select actions, queue ordering, active-transfer limits, categories, tags, global and per-torrent speed caps, and seeding ratio goals. Store library settings and libtorrent resume data transactionally in SQLite. Preserve deliberate pauses and file selections across restarts. [libtorrent resume data](https://www.libtorrent.org/reference-Resume_Data.html)
3. **Control the network.** Provide direct, VPN-interface-bound, and SOCKS5 proxy modes, plus listening-port and connection limits. Apply the selected mode before starting transfers. If a required interface or proxy disappears, suspend affected traffic and show the reason. Verify peer, tracker, discovery, and feed traffic under each mode. [libtorrent network settings](https://www.libtorrent.org/reference-Settings.html)
4. **Automate recurring work.** Support watched folders, RSS include/exclude rules with a match preview, and scheduled bandwidth limits. Record processed items so restarting Fero cannot add them twice. Closing the window keeps transfers and rules running while signed in. Provide a tray or menu-bar control to reopen Fero or quit, with optional start at login.
5. **Control Fero from a browser on the LAN.** Add an opt-in, mobile-friendly web interface for adding torrents, viewing progress, and pausing or resuming. Enable it from the desktop with a selected LAN address and authenticated HTTPS access. The browser, desktop, and automation rules use the same Rust transfer commands so actions agree across all three.

## Moving from qBittorrent

Fero opens directly to an empty library on a fresh install. Users with existing downloads can add a torrent and point it at its files for verification. For a whole library, provide a guided importer for qBittorrent’s `BT_backup` files and `torrents.db`. Preview save locations, categories, tags, duplicates, and missing paths before import. Open the source profile read-only, import transfers paused, and verify existing data before resuming. A second import must skip transfers already present. [qBittorrent file storage](https://github.com/qbittorrent/qBittorrent/blob/master/src/base/bittorrent/bencoderesumedatastorage.cpp) · [qBittorrent SQLite storage](https://github.com/qbittorrent/qBittorrent/blob/master/src/base/bittorrent/dbresumedatastorage.cpp)

## Release and acceptance

- Produce downloadable test builds with bundled runtime dependencies. Target a macOS DMG for Intel and Apple Silicon, a Windows x64 installer, and a Linux x64 AppImage. Install and launch each on a clean machine.
- On each OS, use a controlled local swarm to verify magnet and file intake, selective downloading, byte-for-byte completion, seeding to a second peer, and recovery from an interrupted download. Cover v1, v2, and hybrid torrents.
- Disconnect the selected VPN during a transfer, restart during an RSS import, re-import a partially downloaded library, and pause from a phone while the desktop window is closed. Check that each action leaves the expected files, transfer state, and network activity.
- Verify the everyday add flow using only the keyboard. Drop a multi-file torrent, inspect a skipped file’s save path, choose another folder, and confirm that only the selected files download. Verify that Settings and the sidebar state survive relaunch. Test search, scrolling, filtering, and bulk actions with a 1,000-transfer library in both appearances.

## Settings and production readiness

These phases build on the [first-release roadmap](docs/development.md#first-release-roadmap). Installed-build results belong in the [release qualification matrix](docs/development.md#qualification-progress).

### Phase 1 — Make Settings easier to use

- [x] Move Settings rendering to `src/desktop/settings.rs`. Reuse the existing GPUI controls, preference storage, and worker commands.
- [x] Replace library filters with Settings categories while Settings is open: **Downloads** (default), **Appearance**, **Network & remote access**, **Automation & startup**, and **Import library**. On narrow windows, show a category list and a Back button. Render only the active section.
- [x] Show saved values at a glance, including the default folder and transfer limits. Separate **Before adding a magnet: Review files / Skip file review** from **After adding: Start downloading / Keep paused**. Preserve existing preference behavior.
- [x] Offer a Direct / SOCKS5 / VPN selector and show only the relevant route fields. Distinguish the saved route from the active route when a restart is required. Keep existing routing behavior.
- [ ] Give qBittorrent import its own choose → review → import flow. Validate selected destinations on the worker and show row-level errors instead of silently omitting selections. Reuse the library’s visible-row rendering pattern for large previews. Preserve paused imports and duplicate handling.
- [ ] Verify keyboard navigation and visible focus at 480 and 1060 pixels wide in light and dark. Check that users can change the save folder, identify the active network route, return to the same library state, and reopen Fero with their changes saved. Verify that an unavailable import destination produces an actionable error.

Phase 1 is in progress. The import page now flags unavailable destinations per row and keeps the preview available for correction. Large-preview rendering and native desktop acceptance remain open.

### Phase 2 — Warn before a download runs out of space

- [ ] Show selected download size alongside available space on the chosen destination volume. Check available space on the worker when the destination changes, and update the warning when file selection changes.
- [ ] Keep the warning advisory: existing data may reduce the space required, and magnets without metadata have an unknown size. Handle unavailable folders and ignore stale results from earlier destinations. Use no background polling.
- [ ] Verify a destination with insufficient space, a folder on another volume, rapid destination changes, and a magnet without metadata.

### Phase 3 — Qualify installed releases

- [ ] Build and test installed artifacts on each platform Fero will advertise. The existing matrix covers macOS Intel, Apple Silicon, Linux x64, and Windows x64. Sign and notarize macOS artifacts for distribution.
- [ ] Run the documented clean-install, file and magnet association, tray, login, graceful quit, and restart checks. Run locked v1/v2/hybrid swarm tests and repeat the keyboard, large-library, VPN disconnect, RSS restart, and phone workflows.
- [ ] Run migration against a copied real qBittorrent profile, covering complete and partial data, duplicates, missing destinations, and repeat imports. Record artifact hashes, OS versions, and actual pass or fail results in the release matrix.

**Deferred:** Moving existing transfers between drives. Revisit after release; it needs safe handling of partial data, interrupted moves, and destination failures.
