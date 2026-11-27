#pragma once

#include "rust/cxx.h"
#include <libtorrent/session.hpp>
#include <chrono>
#include <future>
#include <map>
#include <string>
#include <vector>

namespace fero {
struct Transfer;
struct PieceMap;
struct Peer;
struct Tracker;
struct TorrentFile;
struct TorrentHashes;
struct AddOptions;
struct ResumeData;
struct EngineOptions;
struct HttpResponse;
class HttpRequest {
public:
    explicit HttpRequest(std::future<std::pair<std::string, std::vector<char>>> response);
    HttpResponse poll();
private:
    std::future<std::pair<std::string, std::vector<char>>> response_;
    std::chrono::steady_clock::time_point started_;
};
rust::Vec<TorrentFile> inspect_torrent(rust::Slice<const uint8_t> source);
TorrentHashes torrent_hashes(rust::Slice<const uint8_t> source, bool magnet);

class Engine {
public:
    explicit Engine(EngineOptions const& options);

    bool add(int64_t id, rust::Slice<const uint8_t> source, rust::Str destination,
             rust::Slice<const uint8_t> resume, rust::Slice<const uint32_t> skipped_files,
             AddOptions options);
    void fetch_metadata(rust::Str magnet, rust::Str destination);
    rust::Vec<TorrentFile> metadata() const;
    rust::Vec<TorrentFile> files(int64_t id) const;
    PieceMap pieces(int64_t id) const;
    rust::Vec<Peer> peers(int64_t id) const;
    rust::Vec<Tracker> trackers(int64_t id) const;
    void reannounce(int64_t id);
    void force_recheck(int64_t id);
    void check_until_ready(int64_t id);
    void edit_tracker(int64_t id, rust::Str old, rust::Str url, uint8_t tier);
    void set_file_priority(int64_t id, uint32_t index, uint8_t priority);
    void set_limits(int64_t id, int32_t download, int32_t upload);
    void set_network_ready(bool ready);
    std::unique_ptr<HttpRequest> request_feed(rust::Str url) const;
    rust::String vpn_status(rust::Str adapter_name, bool dns_configured);
    void suspend_for_network_restart();
    void clear_error(int64_t id);
    void remove(int64_t id, bool delete_files);
    void set_paused(int64_t id, bool paused, bool graceful);
    rust::Vec<Transfer> snapshot();
    rust::Vec<uint8_t> checkpoint(int64_t id);
    rust::Vec<ResumeData> checkpoints(bool flush);
    uint16_t listen_port() const noexcept;
    void connect_local(int64_t id, uint16_t port);

private:
    libtorrent::session session_;
    bool proxy_;
    std::map<int64_t, libtorrent::torrent_handle> torrents_;
};

std::unique_ptr<Engine> new_engine(EngineOptions const& options);
}
