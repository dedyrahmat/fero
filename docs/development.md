# Building and testing Fero

[Build from source](#build-fero-locally) · [Package a test release](#build-a-test-release) · [Qualification results](#qualification-progress) · [Back to the project overview](../readme.md)

## First-release roadmap

For the product decisions, engine design, and release criteria behind these phases, see the [project plan](../PLAN.md). Follow current work in the [Settings and production readiness plan](../PLAN.md#settings-and-production-readiness).

Each phase ends with an acceptance check. Check off a build task when it works through the app and an acceptance check when someone has run the scenario.

Start each phase on macOS, then run its cross-platform acceptance checks.

### Phase 1 — Prove a real transfer on macOS

- [x] Create the Rust application, pin GPUI, and embed libtorrent 2.1.2 through a small `cxx` bridge. Open a native window on macOS.
- [x] Give the window a working Add action and a live progress row. Accept a local `.torrent` file or magnet and a destination folder. Run libtorrent and disk work off the UI thread.
- [x] Save the transfer identity, destination, pause state, and libtorrent resume data transactionally in SQLite. Restore the session after quitting or an interrupted write.
- [x] **macOS acceptance** — Download from a controlled local seeder, quit partway through, reopen, and finish. Compare the downloaded bytes with the original and seed them to a second peer. Pass this gate before expanding the interface.

**Verification as of 2026-11-21**

- **macOS 15.8.3 (Intel)** — Both **Choose torrent** and **Paste magnet** passed native-window acceptance. After a clean libtorrent 2.1.2 build, the locked desktop build and automated loopback test passed again. In the repeated `.torrent` test, Fero quit with Command-Q, reopened at 19%, and finished. The download and second peer's copy matched the source byte for byte.
- **macOS interrupted-write recovery** — After Fero was forcibly stopped mid-download, a test process wrote to the library database and was stopped before committing. On launch, Fero rolled back the interrupted write and loaded the last committed resume data. The finished download and second peer's copy matched the original byte for byte.
- **Fedora Linux 45 (x86-64, X11 VM)** — The desktop build and automated loopback test passed. The native torrent-file and magnet runs showed live progress in the selected folders and quit mid-transfer with Ctrl-Q. They reopened at 27% and 32% respectively, then finished. The download and second peer's copy matched the source byte for byte in both runs.
- **Windows x64** — The locked Cargo dependency graph resolves for MSVC. The desktop build and acceptance checks are deferred.

Use the [desktop acceptance helper](#run-phase-1-desktop-acceptance) for the native-window checks.

### Phase 2 — Build the everyday desktop experience

Build **Library**, **Settings**, and **Add Torrent** in native GPUI. [Desktop mockup](../design/fero-desktop.html) · [Design requirements](../PLAN.md#desktop-experience)

- [x] Build the window with a left filter sidebar, central transfer list, right inspector, and footer. Use native window controls. Place search and **Add Torrent** above the list. Move the inspector below the list at narrow widths and wrap the filters as needed.
- [x] Apply the rounded rows, panels, and dialogs, pill-shaped actions, warm graphite and parchment palette, Instrument Sans headings, and DM Sans interface text. Keep visible keyboard focus and status labels. Add the pronunciation and Latin meaning to About.
- [x] Add a persistent Settings page for theme, accent, row spacing, default folder, magnet timing, and starting paused. Collapse the sidebar into an icon rail for Focus mode and retain access to filters and Settings.
- [x] Implement the searchable list, status filters with counts, and sorting by recently added, name, progress, or rate. Give rows a name, state, size, progress bar and percentage, transfer rate, and ETA where available. Bind row, inspector, and footer values to the live engine. Expand the footer to see one-minute download and upload graphs with current and peak rates across the library. Cover a fresh library, no results, fetching metadata, waiting for peers, paused, and error states.
- [x] Build the **Add Torrent** dialog with **Magnet link** and **Torrent file** inputs and a highlighted drop zone. Support paste, native file picking, window-level drag-and-drop, and OS-opened magnets and torrent files. Keep the source and destination available after validation or engine errors.
- [x] Review real metadata from either source. Show filenames, folders, sizes, Selected or Skipped, and the full save path. Support file search, select all, clear, a selected download total, and a typed or browsed folder. For magnets, allow an immediate start or fetch metadata before committing.
- [x] Bind row selection to the **Overview** and **Files** inspector tabs. Wire pause and resume. Show selected and skipped files after adding. Make **Remove from Fero** keep downloaded files and require separate confirmation to delete content.
- [x] **macOS design acceptance** — Compare wide and narrow windows with the rounded mockup in both appearances. Check long names, live rates, empty and filtered states, keyboard focus, and the collapsed sidebar. Change appearance and download defaults in Settings and verify them after relaunch.
- [x] **macOS workflow acceptance** — Paste a magnet and use only the keyboard to fetch metadata, confirm the destination, and add it. Drop a multi-file torrent, preview a file’s destination, select one file, and choose a folder with **Start paused** enabled. Resume it from the inspector and verify that removing the transfer keeps its downloaded content.

**macOS verification (Intel)** — The locked build, Clippy, and automated tests passed. Native checks covered 1060 × 700 and 480 × 540 windows in light and dark, Settings persistence, sorting, search in a 90-file review, and the window and dialog drop targets through AppKit. Keyboard input fetched real magnet metadata without creating payload files and confirmed the transfer with the saved folder and paused default. A dropped torrent downloaded one of three files byte for byte to a different folder after resuming. The inspector showed the two skipped files, and **Remove from Fero** kept the completed download. The automated tests also covered restart and reseeding, selected file priorities across restart, and explicit deletion.

### Phase 3 — Manage and inspect a large library

- [x] Add multi-select actions, queue ordering, active-transfer limits, categories, and tags. Support global and per-torrent speed caps and seeding ratio goals.
- [x] Add **Peers** for connected peers and **Trackers** for announce status and reannounce.
- [x] Add priority editing to **Files**, tracker editing to **Trackers**, and **Force Recheck** to the transfer actions.
- [x] Persist settings, queue order, deliberate pauses, and file selections across restarts. Keep search, scrolling, and selection responsive as transfer statistics change.
- [x] **macOS acceptance** — Loaded and scrolled 1,000 transfers at 1060 × 700 in light and dark. Found a stalled transfer, inspected its tracker error, moved it one position earlier, and bulk-paused 12 filtered transfers. The queue position, pause states, category, tags, and library download limit persisted across relaunch.

Use **Select** to pause, resume, or remove a group of transfers. **Select visible** selects the current search and status results. Sort by **Queue position** and use **Earlier** or **Later** in the Overview tab to change the order. **Transfer Options** holds categories, tags, speed caps, and a per-torrent seeding ratio. Search accepts `category:` and `tag:` for exact matches. Set library-wide defaults in **Settings → Transfer limits**.

**Delete Files** removes the torrent's complete and partial data from disk, including the part file used by selective downloads. **Remove from Fero** keeps the data.

### Phase 4 — Control how Fero connects

- [x] Save a preferred listening port and peer connection limit for direct connections. Apply both when the engine starts, before restoring transfers.
- [x] Add SOCKS5 for BitTorrent traffic. Apply the saved proxy before restoring transfers and suspend eligible work with a library status when the proxy goes down. Resume eligible transfers when it returns and preserve manual pauses.
- [x] Add VPN-interface-bound mode. Suspend traffic and show the reason when the interface becomes unavailable. Restore eligible work when the interface returns and preserve manual pauses.
- [x] Apply the same connection policy to HTTP and HTTPS feed requests.
- [x] **macOS acceptance** — Exercise peer, tracker, discovery, and RSS traffic in each mode. Disconnect the selected VPN and stop the proxy mid-transfer. Verify that affected traffic cannot fall back to the default route, including after restart.

Windows and Linux acceptance is deferred to a later manual pass using the same checks.

Open **Settings → Network & remote access → Edit Network…** to choose **Direct**, **SOCKS5**, or **VPN** and set the preferred listening port and peer connection limit. The defaults are port 6881 and 300 connections. Fero may use another port if the preferred port is busy. Use port 0 to let the operating system choose a port. The connection limit is a target across the library. Libtorrent can exceed low limits to allow at least two connections per torrent. Settings shows the saved route alongside the current route. Quit and reopen Fero to apply changes.

**Initial macOS verification (Intel)** — The dark-appearance dialog rejected an out-of-range port and retained both saved values after relaunch. The relaunched engine was listening on the configured TCP port. The locked build, Clippy, and ten automated tests passed.

Choose **SOCKS5** and enter a numeric endpoint such as `127.0.0.1:1080` in **SOCKS5 IP and port**, then restart Fero. Changing the proxy pauses transfers until restart. Fero supports proxies that accept unauthenticated connections. TCP peers and trackers use the proxy. UDP trackers and DHT require the proxy to support UDP relay. DHT uses numeric nodes provided by torrents or learned from peers. Local peer discovery and automatic port mapping are disabled in proxy mode.

Choose **VPN** and enter its adapter name in **VPN interface**, such as `utun3` on macOS or `tun0` on Linux. On Windows use the adapter’s braced GUID. Enter a numeric DNS server reachable through the VPN for hostname-based trackers and DHT bootstrap. A blank DNS field allows numeric peers and trackers. If the adapter disappears, Fero waits for it to return and leaves manually paused transfers paused.

**SOCKS5 loopback verification (macOS Intel)** — A controlled SOCKS5 relay carried TCP peer traffic, hostname-based HTTP and UDP tracker announces, and DHT discovery packets. Stopping the relay mid-download blocked the transfer through restart. Restoring the relay allowed byte-for-byte completion, and a manually paused transfer stayed paused.

**VPN and feed loopback verification (macOS Intel)** — The bound interface carried a byte-for-byte peer download and an HTTP tracker announce resolved by the configured DNS server. A missing saved interface kept transfers offline after restart. RSS responses arrived over HTTP and HTTPS in all three modes. An unavailable VPN or stopped proxy kept feed requests from reaching a reachable direct listener, including after restart. The locked build, Clippy, and 15 automated tests passed.

**Live network verification (macOS Intel)** — Two Direct-mode instances discovered each other locally and downloaded a matching file. In VPN mode, a disposable WireGuard tunnel carried peer traffic, HTTP tracker announces, a DHT query, and an RSS request. Removing the tunnel mid-download stopped the transfer and kept a fresh Fero process offline. The peer address resolved through the default route during the outage, and a packet capture on that interface recorded only a separate control packet sent after the test. Reconnecting the tunnel resumed the download to byte-for-byte completion, and the manually paused transfer stayed paused.

To repeat the HTTPS check, serve `<rss version="2.0"><channel><title>Test feed</title></channel></rss>` at a numeric IPv4 loopback HTTPS URL with a trusted certificate for that IP. Set `FERO_HTTPS_FEED_URL` to the URL and run `cargo test --locked --no-default-features https_feed_uses_the_saved_network_route -- --ignored --nocapture` against the patched engine. On macOS, the test run used `SSL_CERT_FILE` to trust the local test certificate.

**Fedora 45 (x86-64)** — The locked desktop build and 15 automated tests passed against the patched engine. Fero launched under X11 and initialized a fresh library. Local HTTP and HTTPS RSS requests succeeded in Direct, VPN, and SOCKS5 modes. A socket trace captured Direct-mode local peer discovery in a private network namespace. In VPN mode, Fero sent DHT traffic through the selected interface. Bringing that interface down mid-download stopped the transfer through restart. The default IPv4 interface transmitted only the test's control packet during the outage, including across restart. After the VPN interface returned, the file completed byte for byte and the manually paused transfer stayed paused. A socket trace confirmed that the initial HTTP tracker connection and its retry both bound to the selected adapter before connecting.

To reproduce the Linux disconnect check, run `cargo test --locked --no-default-features --no-run` with the patched engine. Use the unit-test executable path printed by Cargo in `sudo unshare --net /path/to/test-executable vpn_link_loss_and_recovery --ignored --nocapture`. Inside that namespace, the test toggles loopback and uses a dummy interface to observe any packets sent through the IPv4 default route.

**Windows x64 cross-build (macOS Intel)** — LLVM-MinGW built the patched libtorrent DLL and compiled Fero's native engine and generated CXX bridge. All 84 libtorrent imports from the engine matched DLL exports. The Windows desktop build and runtime acceptance remain pending.

### Phase 5 — Automate transfers and run in the background

- [x] Watch user-selected folders for torrent files. Record each processed torrent in the same SQLite transaction as its transfer addition so repeated scans and restarts do not add it twice.
- [x] Add RSS subscriptions with include/exclude rules and a preview of matching feed items. Record processed feed entries with their transfer additions so repeats and restarts do not add them twice. Support scheduled bandwidth limits.
- [x] Keep downloads, seeding, and rules running while signed in after the window closes. Add menu-bar or tray actions to reopen Fero and quit, and an optional start-at-login setting.
- [x] Provide an opt-in, mobile-friendly HTTPS interface bound to a user-selected LAN address. Require authentication. Route browser, desktop, and automation actions through the same Rust transfer commands.
- [x] **Acceptance** — Restart during an RSS import and confirm exactly one transfer appears. With the desktop window closed, add a torrent from a phone, view its progress, pause and resume it, then reopen the window and verify the same state. Confirm that Quit stops background work.

**Watched-folder verification (macOS 15.8.3, Intel)** — Two copies in different watched folders produced one transfer. After quitting partway through a loopback download and adding another copy, Fero reopened with one transfer at 9%. The completed download and a second peer's copy both matched the source byte for byte. Settings was checked at the default width in light and dark and at a narrow width in light.

**Phase 5 acceptance (macOS 15.8.3, Intel)** — A controlled feed preview returned one matching release and skipped the excluded item. Fero was stopped while fetching the matching torrent attachment. After restart the library held one transfer and one processed entry, including after another restart. The daily cap was saved through Settings and applied during the swarm download. An isolated macOS login agent launched Fero in background mode. The libtorrent build includes a fix for Darwin’s routing record alignment.

**Phone workflow (Android 13 emulator)** — Android WebView used a trusted test certificate on the selected IPv4 LAN address. It uploaded a torrent with Fero running without a desktop window, displayed partial progress, paused at 43%, and resumed. The reopened native window showed the transfer downloading under the scheduled cap. The finished payload matched the seeder byte for byte. **Quit Fero** exited and closed the HTTPS listener. Requests with missing or invalid credentials returned HTTP 401, and a form submission from another origin returned HTTP 403.

**Fedora 45 (x86-64, X11 VM)** — The locked desktop build and 18 automated engine tests passed against the routed libtorrent build. The app registered its StatusNotifier tray menu at background startup, opened from the tray, minimized on Close, reopened, and exited through **Quit Fero**.

**Windows x64** — The locked MSVC dependency graph resolved, and LLVM-MinGW compiled the updated engine and CXX bridge. The Windows desktop build and runtime acceptance remain pending.

### Phase 6 — Bring over a qBittorrent library

- [x] Let users add an original torrent, point it at existing files, and verify the content before downloading missing pieces or seeding. Use **Add Torrent**, choose the original `.torrent`, select the existing save folder, and keep the transfer paused while Fero checks it.
- [x] Add a guided, read-only importer for qBittorrent `torrents.db` profiles and `BT_backup` directories containing `.torrent`/`.fastresume` files. Preview save locations, categories, tags, duplicates, missing paths, and editable per-transfer destinations before importing selected rows.
- [x] Import selected transfers paused, preserve category and tags, validate matching resume metadata, and force a content recheck. Repeat imports skip transfers already present. Synthetic acceptance tests cover complete and missing content, duplicate/repeat behavior, paused imports, hash-matched SQLite entries, and unchanged profile data for both `torrents.db` and file-only `BT_backup` layouts.
- [ ] **Real-profile acceptance** — Validate complete, partial, duplicate, and missing downloads from actual qBittorrent `torrents.db` and `BT_backup` profiles. Compare preview with imported rows, repeat the import, and verify the source profile is unchanged. Synthetic fixtures pass; real-profile acceptance has not been run.

### Phase 7 — Package and qualify the first test release

- [ ] Produce downloadable macOS DMGs covering Intel and Apple Silicon, a Windows x64 installer, and a Linux x64 AppImage. Bundle the runtime dependencies.
- [ ] Install and launch each artifact on a clean machine. Check file and magnet associations, tray or menu-bar behavior, start at login, and graceful quit from the installed app.
- [ ] Run the local swarm tests on each OS with v1, v2, and hybrid torrents. Cover magnet and file intake, selective downloading, byte-for-byte completion, seeding to a second peer, and interrupted-download recovery.
- [ ] **Release acceptance** — Repeat the keyboard-only add flow, 1,000-transfer library test in both appearances, VPN disconnect, interrupted RSS import, repeat migration, and phone control with the desktop window closed. Record the result for each platform alongside its build.

#### Build a test release

Use the patched libtorrent and locked Cargo setup below. On each target, run `cargo build --release --locked`, `cargo test --locked --no-default-features`, and `cargo test --locked --bin fero`, then package the release binary from the same checkout. Packages go in `dist/`.

| Target | Package command | Installation notes |
| --- | --- | --- |
| macOS Intel and Apple Silicon | `bash native/macos/package.sh` on each architecture | Drag **Fero** to Applications. The script bundles and signs non-system dylibs; use `FERO_CODESIGN_IDENTITY` for a Developer ID build, then notarize and staple the DMG before distribution. An ad-hoc-signed DMG is for local testing only. |
| Windows x64 | `powershell -File native/windows/package.ps1` in the x64 Visual Studio developer environment with NSIS on `PATH` | Run the per-user installer, then choose Fero in Windows **Default apps** for `.torrent` and `magnet`. The installer bundles libtorrent, vcpkg runtime DLLs, and the x64 Visual C++ runtime. |
| Linux x64 | `LINUXDEPLOY=/absolute/path/to/linuxdeploy sh native/linux/package.sh` | Use [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy) with its AppImage output plugin. If linuxdeploy is itself an AppImage and FUSE is unavailable, set `APPIMAGE_EXTRACT_AND_RUN=1` while packaging. Build on the oldest Linux distribution you intend to support; the host still supplies glibc and graphics drivers. |

For Linux file and URL associations, move the AppImage to a permanent path and import it with a desktop AppImage integration tool. Verify `.torrent` and `magnet` resolve to Fero in the desktop's default-app settings. **Start at login** uses that permanent image path; disable and re-enable it if you relocate the image. The tray needs a StatusNotifier-compatible desktop.

#### Qualify the installed build

Use a fresh user profile on a clean OS installation (without the build dependencies). Record the artifact name and SHA-256, OS version, architecture, and the result of each check. **Not run** is not a pass.

1. Launch the installed app, open a `.torrent` and a `magnet:` link through the OS with Fero already running, and verify both open the add dialog without starting another library. Close the window and reopen it from the tray or menu bar. Enable start at login, sign out and back in, and verify the tray appears in background mode. Quit through the tray during a transfer; reopen and verify saved progress. Disable start at login afterward.
2. Run the automated swarm suite for v1, v2, and hybrid torrents. The restart/reseed tests check a partial magnet download, SQLite rollback, byte-for-byte completion, and a second peer. The file-selection tests exercise `.torrent` and magnet preview, persist priorities across restart, and verify a skipped file is not downloaded.
3. Re-run the keyboard-only add and 1,000-transfer checks from Phases 2–3 in light and dark, the VPN disconnect check from Phase 4, and the interrupted RSS and phone checks from Phase 5. Repeat migration with a copied real qBittorrent profile as in Phase 6. Record failures or skipped checks alongside the build.

#### Qualification progress

| Target | Local swarm suite | Installed release acceptance |
| --- | --- | --- |
| macOS 15.8.3 (Intel) | v1, v2, hybrid passed; 28 engine tests and two desktop unit tests passed, two manual tests ignored, using the patched engine | Built `Fero-0.1.0-macos-x86_64.dmg` (SHA-256 `52d8f9be0d6ce9ff1429b5043dad00868bcd63bd59b0e3f1b703865ea9011084`). Verified the image and app signature, launched a copied app with a fresh library, and quit normally. Both a `.torrent` and a magnet sent by Launch Services opened a window in the running background instance. Verified the menu-bar item exists, not its commands or the add dialog contents. Clean-OS acceptance, login, and notarization remain open. |
| macOS (Apple Silicon) | Not run | Not run |
| Fedora 45 (x86-64 VM) | v1, v2, hybrid passed; 25 tests passed, one manual test ignored, using the unpatched engine. Patched libtorrent built, but the test run against it was interrupted. | Packaging smoke produced an extractable AppImage with bundled OpenSSL. The isolated X11 tray smoke timed out. The VM stopped before the patched release artifact was verified; clean-OS acceptance is not complete. |
| Windows x64 | Not run | NSIS compiled with placeholder inputs and warnings treated as errors. No Fero installer or Windows runtime test yet. |

Closing Phase 7 still requires an Apple Silicon host, a Windows x64 build and clean test environment, a clean Linux desktop, and the real qBittorrent profiles from Phase 6. The local smoke results do not replace those checks.

## Build Fero locally

Use stable Rust, a C++17 compiler, CMake, Boost, and OpenSSL. Build the [libtorrent 2.1.2 source archive](https://github.com/arvidn/libtorrent/releases/download/v2.1.2/libtorrent-rasterbar-2.1.2.tar.gz) in Release mode. The archive's SHA-256 is `3362546d9cd71b9e49ee6cac7d3f1f914ce9cdb217c86b63d5b22cbed0334dbc`.

VPN mode and feed fetching use Fero's patches for that archive. Install [c-ares 1.34.8 or later](https://github.com/c-ares/c-ares/releases/tag/v1.34.8) with CMake package files, then apply the patches to a fresh extraction. When building static c-ares on Linux, configure it with `-DCMAKE_POSITION_INDEPENDENT_CODE=ON`. If c-ares is installed outside the usual package search paths, pass its prefix with `-DCMAKE_PREFIX_PATH=/path/to/c-ares`. With an unpatched libtorrent build, a library configured for a VPN stays offline.

Build with the supplied lockfile. It selects `libc` 0.2.189 and `mio` 1.2.4 because GPUI 0.2.2's archive dependency still uses the Linux `ENOATTR` constant.

On macOS or Linux, run these commands from the Fero directory after extracting the archive. Set the first path to the extracted source and add `-DBoost_ROOT=...` or `-DOPENSSL_ROOT_DIR=...` if CMake needs help finding those libraries. Linux also needs GPUI's Wayland, X11, font, and graphics dependencies. [GPUI platform requirements](https://github.com/zed-industries/zed/tree/main/crates/gpui)

```sh
source_dir=/path/to/libtorrent-rasterbar-2.1.2
patch -d "$source_dir" -p1 -i "$PWD/native/libtorrent-strict-interface.patch"
patch -d "$source_dir" -p1 -i "$PWD/native/libtorrent-bound-dns.patch"
patch -d "$source_dir" -p1 -i "$PWD/native/libtorrent-routed-http.patch"
patch -d "$source_dir" -p1 -i "$PWD/native/libtorrent-darwin-routes.patch"
cmake -S "$source_dir" -B .native/build \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PWD/.native/install" \
  -DCMAKE_INSTALL_LIBDIR=lib -DBUILD_SHARED_LIBS=OFF \
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON -Dwebtorrent=OFF
cmake --build .native/build --target install --parallel
export PKG_CONFIG_PATH="$PWD/.native/install/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
cargo run --locked
cargo test --locked --no-default-features
```

On Windows, open an x64 Visual Studio Developer PowerShell with the C++ tools and Windows SDK installed. Run these commands in the Fero directory with x64 MSVC Rust, CMake, Ninja, and a standalone vcpkg installation. Set `VCPKG_ROOT` to that installation and adjust `$source` to the extracted libtorrent archive. [Visual Studio command-line tools](https://learn.microsoft.com/en-us/cpp/build/building-on-the-command-line) · [vcpkg classic mode](https://learn.microsoft.com/en-us/vcpkg/consume/classic-mode)

```powershell
$source = 'C:\src\libtorrent-rasterbar-2.1.2'
git -C $source apply (Join-Path (Get-Location) 'native/libtorrent-strict-interface.patch')
git -C $source apply (Join-Path (Get-Location) 'native/libtorrent-bound-dns.patch')
git -C $source apply (Join-Path (Get-Location) 'native/libtorrent-routed-http.patch')
& "$env:VCPKG_ROOT\vcpkg.exe" install --triplet x64-windows `
    boost-asio boost-container boost-container-hash boost-crc boost-intrusive `
    boost-logic boost-multi-index boost-pool boost-predef boost-range boost-smart-ptr openssl c-ares

$env:FERO_LIBTORRENT_PREFIX = Join-Path (Get-Location) '.native\windows-install'
$env:FERO_NATIVE_DEPS = Join-Path $env:VCPKG_ROOT 'installed\x64-windows'
cmake -S $source -B .native/windows-build -G Ninja `
    "-DCMAKE_TOOLCHAIN_FILE=$env:VCPKG_ROOT/scripts/buildsystems/vcpkg.cmake" `
    -DVCPKG_TARGET_TRIPLET=x64-windows -DCMAKE_BUILD_TYPE=Release `
    "-DCMAKE_INSTALL_PREFIX=$env:FERO_LIBTORRENT_PREFIX" -DCMAKE_INSTALL_LIBDIR=lib `
    -DBUILD_SHARED_LIBS=ON -Dwebtorrent=OFF
cmake --build .native/windows-build --target install --parallel
$env:Path = "$env:FERO_LIBTORRENT_PREFIX\bin;$env:FERO_NATIVE_DEPS\bin;$env:Path"
cargo build --locked
cargo test --locked --no-default-features
```

The automated loopback tests cover a partial magnet download, SQLite restart, reseeding to a second peer, file selection from both sources, and removal with and without deleting files.

### Run as a macOS development app

Build with `cargo build --locked` and assemble a local app bundle to test Finder and macOS magnet delivery.

```sh
mkdir -p .native/desktop/Fero.app/Contents/{MacOS,Resources}
cp native/macos/Fero.icns .native/desktop/Fero.app/Contents/Resources/
cp native/macos/Info.plist .native/desktop/Fero.app/Contents/Info.plist
cp target/debug/fero .native/desktop/Fero.app/Contents/MacOS/fero
open -a "$PWD/.native/desktop/Fero.app"
```

Use Finder’s **Open With** menu or `open -a "$PWD/.native/desktop/Fero.app" /path/to/example.torrent` to open a file. Pass a quoted magnet link in place of the file to test URL delivery.

### Run Phase 1 desktop acceptance

Build with `cargo build --locked` and close any running Fero instance before starting. In a terminal with the native dependencies configured, start the controlled seeder.

```sh
cargo test --locked --no-default-features desktop_acceptance -- --ignored --nocapture
```

In another terminal with the same native dependency environment, set `FERO_DATA_DIR` to the temporary library path printed by the seeder. Launch `target/debug/fero` on macOS or Linux or `.\target\debug\fero.exe` on Windows. Use the same directory when reopening Fero. The seeder expects Fero to listen on loopback port 6881. If Fero uses another port, set `FERO_DESKTOP_PORT` before starting the seeder.

Follow the terminal prompts to add the test torrent, set **Save to** to the printed download folder, watch the progress row, and quit partway through. Reopen Fero and check that the row shows saved progress. Leave Fero running until the helper confirms that the finished download and the second peer's copy both match the seeder's bytes. Run once using the **Torrent file** tab and once using the **Magnet link** tab with **Start immediately** selected.

## Technical references

- Desktop rendering and platform setup. [GPUI](https://gpui.rs/) · [GPUI platform guide](https://github.com/zed-industries/zed/tree/main/crates/gpui)
- Native engine and Rust integration. [libtorrent 2.1.2](https://github.com/arvidn/libtorrent/releases/tag/v2.1.2) · [CXX bridge](https://cxx.rs/) · [Resume data](https://www.libtorrent.org/reference-Resume_Data.html) · [Network settings](https://www.libtorrent.org/reference-Settings.html)
- qBittorrent migration formats. [File-backed storage](https://github.com/qbittorrent/qBittorrent/blob/master/src/base/bittorrent/bencoderesumedatastorage.cpp) · [SQLite storage](https://github.com/qbittorrent/qBittorrent/blob/master/src/base/bittorrent/dbresumedatastorage.cpp)
