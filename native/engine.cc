#include "fero/src/bridge.rs.h"
#include <libtorrent/alert_types.hpp>
#include <libtorrent/announce_entry.hpp>
#include <libtorrent/error_code.hpp>
#include <libtorrent/ip_filter.hpp>
#include <libtorrent/load_torrent.hpp>
#include <libtorrent/magnet_uri.hpp>
#include <libtorrent/peer_class.hpp>
#include <libtorrent/peer_info.hpp>
#include <libtorrent/read_resume_data.hpp>
#include <libtorrent/session_params.hpp>
#include <libtorrent/torrent_status.hpp>
#include <libtorrent/version.hpp>
#include <libtorrent/write_resume_data.hpp>
#if __has_include(<libtorrent/fero_http.hpp>)
#include <libtorrent/fero_http.hpp>
#endif
#ifdef FERO_LIBTORRENT_STRICT_INTERFACE
#include <libtorrent/aux_/enum_net.hpp>
#endif
#include <algorithm>
#include <chrono>
#include <set>
#include <sstream>
#include <stdexcept>

namespace fero {
namespace lt = libtorrent;
static_assert(LIBTORRENT_VERSION_NUM == 20102, "Fero requires libtorrent 2.1.2");
static_assert(TORRENT_USE_RTC == 0, "Build libtorrent with -Dwebtorrent=OFF for Fero's network modes");

static lt::session_params session_parameters(EngineOptions const& options) {
    lt::settings_pack result;
    result.set_str(lt::settings_pack::user_agent, "Fero/0.1.0");
    result.set_int(lt::settings_pack::connections_limit, options.connections);
    const auto alerts = lt::alert_category::error | lt::alert_category::storage;
    result.set_int(lt::settings_pack::alert_mask, static_cast<int>(static_cast<uint32_t>(alerts)));
    if (options.local_only) {
        result.set_str(lt::settings_pack::listen_interfaces, "127.0.0.1:0");
        result.set_str(lt::settings_pack::outgoing_interfaces, "127.0.0.1");
        result.set_bool(lt::settings_pack::enable_dht, false);
        result.set_bool(lt::settings_pack::enable_lsd, false);
        result.set_bool(lt::settings_pack::enable_upnp, false);
        result.set_bool(lt::settings_pack::enable_natpmp, false);
        result.set_bool(lt::settings_pack::enable_incoming_utp, false);
        result.set_bool(lt::settings_pack::enable_outgoing_utp, false);
        result.set_bool(lt::settings_pack::allow_multiple_connections_per_ip, true);
    } else {
        const auto port = std::to_string(options.listen_port);
        result.set_str(lt::settings_pack::listen_interfaces, "0.0.0.0:" + port + ",[::]:" + port);
        result.set_int(lt::settings_pack::connection_speed, 45);
    }
    if (options.proxy_port != 0) {
        result.set_str(lt::settings_pack::outgoing_interfaces, "");
        result.set_int(lt::settings_pack::proxy_type, lt::settings_pack::socks5);
        result.set_str(lt::settings_pack::proxy_hostname, std::string(options.proxy_host));
        result.set_int(lt::settings_pack::proxy_port, options.proxy_port);
        result.set_bool(lt::settings_pack::proxy_hostnames, true);
        result.set_bool(lt::settings_pack::proxy_peer_connections, true);
        result.set_bool(lt::settings_pack::proxy_tracker_connections, true);
        result.set_bool(lt::settings_pack::enable_lsd, false);
        result.set_bool(lt::settings_pack::enable_upnp, false);
        result.set_bool(lt::settings_pack::enable_natpmp, false);
        // Libtorrent resolves DHT bootstrap hostnames with the system resolver.
        result.set_str(lt::settings_pack::dht_bootstrap_nodes, "");
        result.set_bool(lt::settings_pack::enable_dht, options.network_ready);
    }
    if (!options.vpn_interface.empty()) {
        result.set_bool(lt::settings_pack::enable_dht, false);
        result.set_bool(lt::settings_pack::enable_lsd, false);
        result.set_bool(lt::settings_pack::enable_upnp, false);
        result.set_bool(lt::settings_pack::enable_natpmp, false);
#ifdef FERO_LIBTORRENT_STRICT_INTERFACE
        const auto name = std::string(options.vpn_interface);
        result.set_bool(lt::settings_pack::fero_strict_interface, true);
        result.set_str(lt::settings_pack::listen_interfaces, name + ":" + std::to_string(options.listen_port));
        result.set_str(lt::settings_pack::outgoing_interfaces, name);
#ifdef FERO_LIBTORRENT_BOUND_DNS
        result.set_str(lt::settings_pack::fero_dns_server, std::string(options.vpn_dns));
#endif
        if (options.vpn_dns.empty() || options.local_only) {
            result.set_str(lt::settings_pack::dht_bootstrap_nodes, "");
        }
#else
        // A library with a saved VPN route must also stay offline in an unpatched build.
        result.set_str(lt::settings_pack::listen_interfaces, "");
        result.set_bool(lt::settings_pack::enable_outgoing_tcp, false);
        result.set_bool(lt::settings_pack::enable_outgoing_utp, false);
#endif
    }
    lt::session_params params(std::move(result));
    if (!options.network_ready) params.flags |= lt::session::paused;
    return params;
}

Engine::Engine(EngineOptions const& options)
    : session_(session_parameters(options)), proxy_(options.proxy_port != 0) {
    auto const global_class = 1u << static_cast<uint32_t>(lt::session::global_peer_class_id);
    auto const local_class = 1u << static_cast<uint32_t>(lt::session::local_peer_class_id);
    // Include LAN peers in the same bandwidth budget as internet peers.
    lt::ip_filter peers;
    auto const classes = global_class | (options.local_only ? local_class : 0u);
    peers.add_rule(lt::address_v4::any(), lt::address_v4::broadcast(), classes);
    peers.add_rule(lt::address_v6::any(), lt::make_address_v6("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), classes);
    session_.set_peer_class_filter(peers);
    if (options.local_only) {
        auto local = session_.get_peer_class(lt::session::local_peer_class_id);
        // Keep local swarm transfers slow enough to exercise interruptions.
        local.download_limit = 512 * 1024;
        local.upload_limit = 512 * 1024;
        session_.set_peer_class(lt::session::local_peer_class_id, local);
    }
}

std::unique_ptr<Engine> new_engine(EngineOptions const& options) {
    return std::make_unique<Engine>(options);
}

rust::String Engine::vpn_status(rust::Str adapter_name, bool dns_configured) {
#ifdef FERO_LIBTORRENT_STRICT_INTERFACE
#ifndef FERO_LIBTORRENT_BOUND_DNS
    if (dns_configured) return "VPN DNS requires Fero's bound DNS patch";
#else
    (void)dns_configured;
#endif
    const auto name = std::string(adapter_name);
    lt::io_context io;
    lt::error_code error;
    const auto adapters = lt::aux::enum_net_interfaces(io, error);
    const auto available = std::any_of(adapters.begin(), adapters.end(), [&](auto const& adapter) {
        return name == adapter.name && adapter.preferred && bool(adapter.flags & lt::aux::if_flags::up)
            && (adapter.state == lt::aux::if_state::up || adapter.state == lt::aux::if_state::unknown)
            && !adapter.interface_address.is_unspecified();
    });
    if (error || !available) return rust::String::lossy("VPN interface " + name + " is unavailable");
    if (!session_.is_listening()) {
        session_.reopen_network_sockets({});
        if (!session_.is_listening()) {
            return rust::String::lossy("Waiting to bind VPN interface " + name);
        }
    }
    return {};
#else
    (void)adapter_name;
    (void)dns_configured;
    return "VPN binding requires Fero's libtorrent patch";
#endif
}

using FeedResponse = std::pair<std::string, std::vector<char>>;

HttpRequest::HttpRequest(std::future<FeedResponse> response)
    : response_(std::move(response)), started_(std::chrono::steady_clock::now()) {}

HttpResponse HttpRequest::poll() {
    if (response_.wait_for(std::chrono::seconds(0)) != std::future_status::ready) {
        if (std::chrono::steady_clock::now() - started_ >= std::chrono::seconds(25)) {
            throw std::runtime_error("Timed out fetching the feed");
        }
        return {false, {}};
    }
    auto [error, body] = response_.get();
    if (!error.empty()) throw std::runtime_error(error);
    rust::Vec<uint8_t> result;
    result.reserve(body.size());
    for (auto byte : body) result.push_back(static_cast<uint8_t>(byte));
    return {true, std::move(result)};
}

std::unique_ptr<HttpRequest> Engine::request_feed(rust::Str url) const {
#ifdef FERO_LIBTORRENT_HTTP_HPP
    auto completion = std::make_shared<std::promise<FeedResponse>>();
    auto response = completion->get_future();
    lt::fero_fetch_feed(session_, std::string(url), [completion](std::string error, std::vector<char> body) {
        completion->set_value({std::move(error), std::move(body)});
    });
    return std::make_unique<HttpRequest>(std::move(response));
#else
    (void)url;
    throw std::runtime_error("Feed fetching requires Fero's routed HTTP patch");
#endif
}

static lt::span<char const> bytes(rust::Slice<const uint8_t> data) {
    return {reinterpret_cast<char const*>(data.data()), static_cast<std::ptrdiff_t>(data.size())};
}

static rust::Vec<TorrentFile> list_files(
    lt::torrent_info const& info, std::vector<lt::download_priority_t> const& priorities = {}) {
    rust::Vec<TorrentFile> result;
    auto const& files = info.layout();
    for (auto index : files.file_range()) {
        if (!files.pad_file_at(index)) {
            auto const number = static_cast<uint32_t>(static_cast<int>(index));
            auto const priority = number < priorities.size() ? priorities[number] : lt::default_priority;
            result.push_back({number, rust::String::lossy(files.file_path(index)),
                              files.file_size(index), priority != lt::dont_download,
                              static_cast<uint8_t>(priority)});
        }
    }
    return result;
}

rust::Vec<TorrentFile> inspect_torrent(rust::Slice<const uint8_t> source) {
    return list_files(*lt::load_torrent_buffer(bytes(source)).ti);
}

TorrentHashes torrent_hashes(rust::Slice<const uint8_t> source, bool magnet) {
    auto const hashes = magnet
        ? lt::parse_magnet_uri(lt::string_view(reinterpret_cast<char const*>(source.data()), source.size())).info_hashes
        : lt::load_torrent_buffer(bytes(source)).info_hashes;
    TorrentHashes result;
    if (hashes.has_v1()) {
        for (auto byte : hashes.v1) result.v1.push_back(byte);
    }
    if (hashes.has_v2()) {
        for (auto byte : hashes.v2) result.v2.push_back(byte);
    }
    return result;
}

bool Engine::add(int64_t id, rust::Slice<const uint8_t> source, rust::Str destination,
                rust::Slice<const uint8_t> resume, rust::Slice<const uint32_t> skipped_files,
                AddOptions options) {
    if (torrents_.count(id)) throw std::runtime_error("Transfer already exists");
    auto params = options.magnet
        ? lt::parse_magnet_uri(lt::string_view(reinterpret_cast<char const*>(source.data()), source.size()))
        : lt::load_torrent_buffer(bytes(source));
    if (!resume.empty()) {
        lt::error_code error;
        auto saved = lt::read_resume_data(bytes(resume), error);
        auto const& expected = params.info_hashes;
        if (!error
            && (!expected.has_v1() || saved.info_hashes.v1 == expected.v1)
            && (!expected.has_v2() || saved.info_hashes.v2 == expected.v2)) {
            if (options.from_preview) {
                // Keep the magnet's parameters and discard the preview's disabled file
                // priorities and any disk state from the preview's original directory.
                if (!saved.ti) throw std::runtime_error("Magnet preview metadata is missing");
                params.ti = std::move(saved.ti);
                params.merkle_trees = std::move(saved.merkle_trees);
                params.merkle_tree_mask = std::move(saved.merkle_tree_mask);
                params.verified_leaf_hashes = std::move(saved.verified_leaf_hashes);
            } else {
                if (!saved.ti) saved.ti = params.ti;
                params = std::move(saved);
            }
        } else if (options.from_preview || options.require_resume) {
            throw std::runtime_error("Cannot read matching qBittorrent resume data");
        }
    }
    params.save_path = std::string(destination);
    if (proxy_) {
        // Numeric nodes can bootstrap DHT through the SOCKS5 UDP relay.
        auto& nodes = params.dht_nodes;
        nodes.erase(std::remove_if(nodes.begin(), nodes.end(), [](auto const& node) {
            lt::error_code error;
            lt::make_address(node.first, error);
            return bool(error);
        }), nodes.end());
    }
    if (!skipped_files.empty()) {
        if (!params.ti) throw std::runtime_error("File selection requires torrent metadata");
        params.file_priorities.assign(params.ti->num_files(), lt::default_priority);
        for (auto index : skipped_files) {
            params.file_priorities.at(index) = lt::dont_download;
        }
        params.piece_priorities.clear();
    }
    params.flags &= ~(lt::torrent_flags::auto_managed | lt::torrent_flags::stop_when_ready);
    params.flags |= lt::torrent_flags::duplicate_is_error;
    if (options.paused) params.flags |= lt::torrent_flags::paused;
    else params.flags &= ~lt::torrent_flags::paused;
    lt::error_code error;
    auto handle = session_.add_torrent(std::move(params), error);
    if (options.skip_existing && error == lt::errors::duplicate_torrent && !torrents_.count(0)) {
        return false;
    }
    if (error) throw lt::system_error(error);
    torrents_.emplace(id, std::move(handle));
    return true;
}

void Engine::fetch_metadata(rust::Str magnet, rust::Str destination) {
    auto params = lt::parse_magnet_uri(lt::string_view(magnet.data(), magnet.size()));
    params.save_path = std::string(destination);
    params.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    params.flags |= lt::torrent_flags::duplicate_is_error | lt::torrent_flags::default_dont_download
        | lt::torrent_flags::upload_mode;
    torrents_.emplace(0, session_.add_torrent(std::move(params)));
}

rust::Vec<TorrentFile> Engine::metadata() const {
    auto const& handle = torrents_.at(0);
    auto status = handle.status();
    if (status.errc) throw std::runtime_error(status.errc.message());
    auto info = handle.torrent_file();
    return info ? list_files(*info) : rust::Vec<TorrentFile>{};
}

rust::Vec<TorrentFile> Engine::files(int64_t id) const {
    auto const& handle = torrents_.at(id);
    auto info = handle.torrent_file();
    return info ? list_files(*info, handle.get_file_priorities()) : rust::Vec<TorrentFile>{};
}

PieceMap Engine::pieces(int64_t id) const {
    auto const& handle = torrents_.at(id);
    auto const status = handle.status(lt::torrent_handle::query_pieces
                                     | lt::torrent_handle::query_torrent_file);
    auto const info = status.torrent_file.lock();
    if (!info || info->num_pieces() == 0) return {};

    auto const total = info->num_pieces();
    // Keep the panel and bridge payload bounded even for very large torrents.
    auto const cells = std::min(total, 640);
    PieceMap result{static_cast<uint32_t>(total), 0,
                    static_cast<uint32_t>(info->piece_length()), {}};
    result.cells.reserve(cells);
    auto const boundary = [total, cells](int cell) {
        return static_cast<int>(int64_t{cell} * total / cells);
    };
    for (int cell = 0; cell < cells; ++cell) {
        auto const begin = boundary(cell);
        auto const end = boundary(cell + 1);
        int have = 0;
        for (int piece = begin; piece < end; ++piece) {
            have += status.is_seeding
                || (piece < status.pieces.size() && status.pieces[lt::piece_index_t{piece}]);
        }
        result.have += have;
        result.cells.push_back(have == end - begin ? 2 : have > 0 ? 1 : 0);
    }
    for (auto const& piece : handle.get_download_queue()) {
        auto const index = static_cast<int>(piece.piece_index);
        if (index < 0 || index >= total || (piece.finished == 0 && piece.writing == 0)) continue;
        // Invert the rounded-down boundaries to find the cell containing this piece.
        auto const cell = ((int64_t{index} + 1) * cells - 1) / total;
        if (result.cells[cell] == 0) result.cells[cell] = 1;
    }
    return result;
}

rust::Vec<Peer> Engine::peers(int64_t id) const {
    std::vector<lt::peer_info> peers;
    torrents_.at(id).get_peer_info(peers);
    rust::Vec<Peer> result;
    result.reserve(peers.size());
    for (auto const& peer : peers) {
        if (peer.flags & (lt::peer_info::connecting | lt::peer_info::handshake)) continue;
        std::ostringstream endpoint;
        auto const i2p = bool(peer.flags & lt::peer_info::i2p_socket);
        if (i2p) endpoint << "I2P peer";
        else endpoint << peer.remote_endpoint();
        auto transport = "TCP";
        if (peer.connection_type != lt::peer_info::standard_bittorrent) transport = "Web seed";
        else if (i2p) transport = "I2P";
        else if (peer.flags & lt::peer_info::utp_socket) transport = "uTP";
        result.push_back({endpoint.str(), rust::String::lossy(peer.client), transport,
                          bool(peer.flags & lt::peer_info::seed), peer.payload_down_speed,
                          peer.payload_up_speed, peer.total_download});
    }
    return result;
}

rust::Vec<Tracker> Engine::trackers(int64_t id) const {
    rust::Vec<Tracker> result;
    for (auto const& tracker : torrents_.at(id).trackers()) {
        bool updating = false;
        bool working = false;
        std::string error;
        for (auto const& endpoint : tracker.endpoints) {
            for (auto const& announce : endpoint.info_hashes) {
                updating |= announce.updating;
                working |= announce.start_sent && !announce.last_error;
                if (error.empty() && announce.last_error) {
                    error = announce.message.empty()
                        ? announce.last_error.message() : announce.message;
                }
            }
        }
        auto const failed = !updating && !working && !error.empty();
        auto const status = updating ? "Announcing" : working ? "Working"
            : failed ? "Error" : tracker.verified ? "Responded" : "Awaiting announce";
        result.push_back({rust::String::lossy(tracker.url), tracker.tier, status,
                          failed ? rust::String::lossy(error) : rust::String{}});
    }
    return result;
}

void Engine::reannounce(int64_t id) {
    torrents_.at(id).force_reannounce();
}

void Engine::force_recheck(int64_t id) {
    auto const& handle = torrents_.at(id);
    if (!handle.torrent_file()) throw std::runtime_error("Waiting for torrent metadata");
    auto const was_paused = bool(handle.flags() & lt::torrent_flags::paused);
    handle.force_recheck();
    if (was_paused) check_until_ready(id);
}

void Engine::check_until_ready(int64_t id) {
    auto const& handle = torrents_.at(id);
    // Let a paused torrent finish checking its disk data before it can transfer.
    handle.set_flags(lt::torrent_flags::stop_when_ready);
    handle.resume();
}

void Engine::edit_tracker(int64_t id, rust::Str old, rust::Str url, uint8_t tier) {
    auto const& handle = torrents_.at(id);
    auto trackers = handle.trackers();
    auto const previous = std::string(old);
    auto const next = std::string(url);
    if (!previous.empty()) {
        auto const entry = std::find_if(trackers.begin(), trackers.end(), [&](auto const& tracker) {
            return tracker.url == previous;
        });
        if (entry == trackers.end()) throw std::runtime_error("Tracker no longer exists");
        trackers.erase(entry);
    }
    if (!next.empty()) {
        if (std::any_of(trackers.begin(), trackers.end(), [&](auto const& tracker) {
            return tracker.url == next;
        })) throw std::runtime_error("Tracker already exists");
        lt::announce_entry entry(next);
        entry.tier = tier;
        trackers.push_back(std::move(entry));
    }
    handle.replace_trackers(trackers);
}

void Engine::set_file_priority(int64_t id, uint32_t index, uint8_t priority) {
    auto const& handle = torrents_.at(id);
    auto const info = handle.torrent_file();
    if (!info || index >= static_cast<uint32_t>(info->num_files())
        || info->layout().pad_file_at(lt::file_index_t(index)))
        throw std::runtime_error("Torrent file does not exist");
    if (handle.status().is_seeding) throw std::runtime_error("All files are already downloaded");
    auto const file = lt::file_index_t(index);
    auto const value = lt::download_priority_t(priority);
    if (handle.file_priority(file) == value) return;
    if (!priority) {
        auto const files = list_files(*info, handle.get_file_priorities());
        if (std::none_of(files.begin(), files.end(), [&](auto const& item) {
            return item.index != index && item.selected;
        })) throw std::runtime_error("Keep at least one file selected");
    }
    handle.file_priority(file, value);
    auto const deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    std::vector<lt::alert*> alerts;
    while (std::chrono::steady_clock::now() < deadline) {
        session_.wait_for_alert(lt::milliseconds(100));
        session_.pop_alerts(&alerts);
        for (auto const* alert : alerts) {
            if (auto const* changed = lt::alert_cast<lt::file_prio_alert>(alert);
                changed && changed->handle == handle) return;
            if (auto const* failed = lt::alert_cast<lt::file_error_alert>(alert);
                failed && failed->handle == handle) throw std::runtime_error(failed->message());
        }
    }
    throw std::runtime_error("Timed out applying file priority");
}

void Engine::set_limits(int64_t id, int32_t download, int32_t upload) {
    if (id != 0) {
        auto const& handle = torrents_.at(id);
        handle.set_download_limit(download);
        handle.set_upload_limit(upload);
        return;
    }
    lt::settings_pack rates;
    rates.set_int(lt::settings_pack::download_rate_limit, download);
    rates.set_int(lt::settings_pack::upload_rate_limit, upload);
    session_.apply_settings(std::move(rates));
}

void Engine::set_network_ready(bool ready) {
    lt::settings_pack settings;
    settings.set_bool(lt::settings_pack::enable_dht, ready);
    if (!ready) session_.pause();
    session_.apply_settings(std::move(settings));
    if (ready) session_.resume();
}

void Engine::suspend_for_network_restart() {
    lt::settings_pack settings;
    settings.set_str(lt::settings_pack::listen_interfaces, "");
    settings.set_bool(lt::settings_pack::enable_outgoing_tcp, false);
    settings.set_bool(lt::settings_pack::enable_outgoing_utp, false);
    settings.set_bool(lt::settings_pack::enable_dht, false);
    settings.set_bool(lt::settings_pack::enable_lsd, false);
    settings.set_bool(lt::settings_pack::enable_upnp, false);
    settings.set_bool(lt::settings_pack::enable_natpmp, false);
    session_.apply_settings(std::move(settings));
    session_.pause();
}

void Engine::clear_error(int64_t id) {
    torrents_.at(id).clear_error();
}

void Engine::remove(int64_t id, bool delete_files) {
    auto entry = torrents_.find(id);
    if (entry == torrents_.end()) return;
    auto const handle = entry->second;
    auto const hashes = delete_files ? handle.info_hashes() : lt::info_hash_t{};
    session_.remove_torrent(handle, delete_files ? lt::session::delete_files : lt::remove_flags_t{});
    torrents_.erase(entry);
    if (!delete_files) return;

    auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    std::vector<lt::alert*> alerts;
    while (std::chrono::steady_clock::now() < deadline) {
        session_.wait_for_alert(lt::milliseconds(100));
        session_.pop_alerts(&alerts);
        for (auto alert : alerts) {
            auto removed = lt::alert_cast<lt::torrent_deleted_alert>(alert);
            if (removed && (removed->handle == handle || removed->info_hashes == hashes)) return;
            auto failed = lt::alert_cast<lt::torrent_delete_failed_alert>(alert);
            if (failed && (failed->handle == handle || failed->info_hashes == hashes))
                throw std::runtime_error(failed->error.message());
        }
    }
    throw std::runtime_error("Timed out waiting for the disk to confirm deletion");
}

void Engine::set_paused(int64_t id, bool paused, bool graceful) {
    auto const& handle = torrents_.at(id);
    if (paused) handle.pause(graceful ? lt::torrent_handle::graceful_pause : lt::pause_flags_t{});
    else {
        handle.clear_error();
        handle.resume();
    }
}

rust::Vec<Transfer> Engine::snapshot() {
    std::vector<lt::alert*> alerts;
    session_.pop_alerts(&alerts);
    rust::Vec<Transfer> transfers;
    std::map<lt::torrent_handle, int64_t> ids;
    for (auto const& [id, handle] : torrents_) {
        if (id != 0) ids.emplace(handle, id);
    }
    auto const statuses = session_.get_torrent_status([](auto const&) { return true; }, lt::torrent_handle::query_name
        | lt::torrent_handle::query_save_path | lt::torrent_handle::query_accurate_download_counters);
    transfers.reserve(ids.size());
    for (auto const& status : statuses) {
        auto const entry = ids.find(status.handle);
        if (entry == ids.end()) continue;
        auto const id = entry->second;
        auto paused = bool(status.flags & lt::torrent_flags::paused);
        std::string state;
        if (status.errc) state = "Error";
        else if (status.state == lt::torrent_status::checking_files
                 || status.state == lt::torrent_status::checking_resume_data) state = "Checking files";
        else if (paused) state = "Paused";
        else if (status.is_finished) state = "Seeding";
        else if (!status.has_metadata) state = "Fetching metadata";
        else if (status.num_peers == 0) state = "Waiting for peers";
        else state = "Downloading";
        transfers.push_back({id, rust::String::lossy(status.name), status.progress,
                             status.download_payload_rate, status.upload_payload_rate,
                             status.total, status.total_wanted,
                             status.total_wanted - status.total_wanted_done, status.num_peers,
                             rust::String::lossy(status.save_path), state,
                             rust::String::lossy(status.errc ? status.errc.message() : ""), paused,
                             status.is_finished, status.has_metadata, status.all_time_download,
                             status.all_time_upload});
    }
    return transfers;
}

static rust::Vec<uint8_t> resume_bytes(lt::add_torrent_params const& params) {
    auto const data = lt::write_resume_data_buf(params);
    rust::Vec<uint8_t> result;
    result.reserve(data.size());
    for (auto const byte : data) result.push_back(static_cast<uint8_t>(byte));
    return result;
}

rust::Vec<uint8_t> Engine::checkpoint(int64_t id) {
    auto const& handle = torrents_.at(id);
    // save_resume_data(flush_disk_cache) schedules a flush, but can snapshot
    // the piece state before the flush completes.
    std::vector<lt::alert*> alerts;
    session_.pop_alerts(&alerts);
    // A magnet still fetching metadata has no storage and never posts a flush alert.
    bool flushing = bool(handle.torrent_file());
    if (flushing) handle.flush_cache();
    else handle.save_resume_data(lt::torrent_handle::save_info_dict);
    auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    while (std::chrono::steady_clock::now() < deadline) {
        session_.wait_for_alert(lt::milliseconds(100));
        session_.pop_alerts(&alerts);
        for (auto alert : alerts) {
            auto flushed = lt::alert_cast<lt::cache_flushed_alert>(alert);
            if (flushing && flushed && flushed->handle == handle) {
                flushing = false;
                handle.save_resume_data(lt::torrent_handle::save_info_dict);
            }
            auto saved = lt::alert_cast<lt::save_resume_data_alert>(alert);
            if (saved && saved->handle == handle) return resume_bytes(saved->params);
            auto failed = lt::alert_cast<lt::save_resume_data_failed_alert>(alert);
            if (failed && failed->handle == handle) throw std::runtime_error(failed->message());
        }
    }
    throw std::runtime_error("Timed out saving transfer progress");
}

rust::Vec<ResumeData> Engine::checkpoints(bool flush) {
    auto flags = lt::torrent_handle::save_info_dict;
    if (!flush) flags |= lt::torrent_handle::only_if_modified;

    std::vector<lt::alert*> alerts;
    session_.pop_alerts(&alerts);
    std::map<lt::torrent_handle, int64_t> pending;
    std::set<lt::torrent_handle> flushing;
    for (auto const& [id, handle] : torrents_) {
        if (id == 0) continue;
        pending.emplace(handle, id);
        if (flush && handle.torrent_file()) {
            flushing.insert(handle);
            handle.flush_cache();
        } else {
            handle.save_resume_data(flags);
        }
    }

    rust::Vec<ResumeData> result;
    auto const deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
    std::string error;
    while (!pending.empty() && std::chrono::steady_clock::now() < deadline) {
        session_.wait_for_alert(lt::milliseconds(100));
        session_.pop_alerts(&alerts);
        for (auto const* alert : alerts) {
            if (auto const* flushed = lt::alert_cast<lt::cache_flushed_alert>(alert)) {
                if (flushing.erase(flushed->handle)) flushed->handle.save_resume_data(flags);
            } else if (auto const* saved = lt::alert_cast<lt::save_resume_data_alert>(alert)) {
                auto const entry = pending.find(saved->handle);
                if (entry == pending.end()) continue;
                result.push_back({entry->second, resume_bytes(saved->params)});
                pending.erase(entry);
            } else if (auto const* failed = lt::alert_cast<lt::save_resume_data_failed_alert>(alert)) {
                if (!pending.erase(failed->handle)) continue;
                if (failed->error != lt::errors::resume_data_not_modified && error.empty())
                    error = failed->message();
            }
        }
    }
    if (!error.empty()) throw std::runtime_error(error);
    if (!pending.empty()) throw std::runtime_error("Timed out saving transfer progress");
    return result;
}

uint16_t Engine::listen_port() const noexcept {
    return session_.listen_port();
}

void Engine::connect_local(int64_t id, uint16_t port) {
    torrents_.at(id).connect_peer({lt::address_v4::loopback(), port});
}
}
