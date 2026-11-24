#include "mp4.h"

#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <unistd.h>

namespace camd {

namespace {

const uint32_t TIMESCALE = 90000;
const uint32_t FLAGS_KEY = 0x02000000;     // depends on nothing
const uint32_t FLAGS_DELTA = 0x01010000;   // depends on others, not a sync sample
const uint32_t AAC_FRAME = 1024;

// Boxes nest: begin() leaves room for the size, end() fills it in.
struct Boxes {
    std::vector<uint8_t> b;
    std::vector<size_t> open;

    void u8(uint32_t v) { b.push_back(uint8_t(v)); }
    void u16(uint32_t v)
    {
        u8(v >> 8);
        u8(v);
    }
    void u32(uint32_t v)
    {
        u16(v >> 16);
        u16(v);
    }
    void u64(uint64_t v)
    {
        u32(uint32_t(v >> 32));
        u32(uint32_t(v));
    }
    void zeros(size_t n) { b.insert(b.end(), n, 0); }
    void str(const char *s, size_t n) { b.insert(b.end(), s, s + n); }
    void bytes(const std::vector<uint8_t> &v) { b.insert(b.end(), v.begin(), v.end()); }
    void begin(const char *type)
    {
        open.push_back(b.size());
        u32(0);
        str(type, 4);
    }
    // A full box: version and flags after the type.
    void full(const char *type, uint32_t version, uint32_t flags)
    {
        begin(type);
        u32(version << 24 | flags);
    }
    void end()
    {
        size_t at = open.back();
        open.pop_back();
        uint32_t n = uint32_t(b.size() - at);
        b[at] = n >> 24;
        b[at + 1] = n >> 16;
        b[at + 2] = n >> 8;
        b[at + 3] = n;
    }
    void matrix()
    {
        for (uint32_t v : {0x00010000u, 0u, 0u, 0u, 0x00010000u, 0u, 0u, 0u, 0x40000000u})
            u32(v);
    }
};

// The NAL units of an Annex B stream: (start, length) pairs.
std::vector<std::pair<const uint8_t *, size_t>> nals(const uint8_t *p, size_t n)
{
    std::vector<std::pair<const uint8_t *, size_t>> out;
    size_t i = 0, start = SIZE_MAX;
    while (i + 3 <= n) {
        if (p[i] == 0 && p[i + 1] == 0 && p[i + 2] == 1) {
            if (start != SIZE_MAX) {
                size_t end = i;
                while (end > start && p[end - 1] == 0)
                    end--;
                out.emplace_back(p + start, end - start);
            }
            i += 3;
            start = i;
        } else {
            i++;
        }
    }
    if (start != SIZE_MAX && start < n)
        out.emplace_back(p + start, n - start);
    return out;
}

// The sound's trak in the moov.
void audioTrak(Boxes &x, const AudioTrack &audio_)
{
    x.begin("trak");
    x.full("tkhd", 0, 3);
    x.u32(0);
    x.u32(0);
    x.u32(2);
    x.u32(0);
    x.u32(0);
    x.zeros(8);
    x.u16(0);
    x.u16(1); // alternate group
    x.u16(0x0100);
    x.u16(0);
    x.matrix();
    x.u32(0);
    x.u32(0);
    x.end();

    x.begin("mdia");
    x.full("mdhd", 0, 0);
    x.u32(0);
    x.u32(0);
    x.u32(uint32_t(audio_.rate));
    x.u32(0);
    x.u16(0x55c4);
    x.u16(0);
    x.end();
    x.full("hdlr", 0, 0);
    x.u32(0);
    x.str("soun", 4);
    x.zeros(12);
    x.str("SoundHandler", 13);
    x.end();

    x.begin("minf");
    x.full("smhd", 0, 0);
    x.u32(0);
    x.end();
    x.begin("dinf");
    x.full("dref", 0, 0);
    x.u32(1);
    x.full("url ", 0, 1);
    x.end();
    x.end();
    x.end();

    x.begin("stbl");
    x.full("stsd", 0, 0);
    x.u32(1);
    x.begin("mp4a");
    x.zeros(6);
    x.u16(1);
    x.zeros(8);
    x.u16(uint32_t(audio_.channels));
    x.u16(16);
    x.u32(0);
    x.u32(uint32_t(audio_.rate) << 16);
    // ES_Descriptor > DecoderConfigDescriptor > DecoderSpecificInfo.
    uint32_t asc = uint32_t(audio_.asc.size());
    x.full("esds", 0, 0);
    x.u8(0x03);
    x.u8(3 + 2 + 13 + 2 + asc + 3);
    x.u16(0);
    x.u8(0);
    x.u8(0x04);
    x.u8(13 + 2 + asc);
    x.u8(0x40); // MPEG-4 audio
    x.u8(0x15); // audio stream
    x.u8(0);
    x.u16(0);
    x.u32(0);
    x.u32(0);
    x.u8(0x05);
    x.u8(asc);
    x.bytes(audio_.asc);
    x.u8(0x06);
    x.u8(1);
    x.u8(2);
    x.end(); // esds
    x.end(); // mp4a
    x.end(); // stsd
    for (const char *t : {"stts", "stsc", "stco"}) {
        x.full(t, 0, 0);
        x.u32(0);
        x.end();
    }
    x.full("stsz", 0, 0);
    x.u32(0);
    x.u32(0);
    x.end();
    x.end(); // stbl
    x.end(); // minf
    x.end(); // mdia
    x.end(); // trak
}

} // namespace

Mp4::~Mp4()
{
    finish();
}

bool Mp4::open(const std::string &path, int w, int h, const AudioTrack *audio, std::string &why)
{
    fd_ = ::open(path.c_str(), O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0644);
    if (fd_ < 0) {
        why = strerror(errno);
        return false;
    }
    w_ = w;
    h_ = h;
    done_ = bad_ = headed_ = false;
    frags_ = 0;
    written_ = 0;
    firstTs_ = lastTs_ = -1;
    samples_.clear();
    data_.clear();
    hasAudio_ = audio && !audio->asc.empty();
    if (hasAudio_)
        audio_ = *audio;
    aSizes_.clear();
    aData_.clear();
    aBase_ = -1;
    aDone_ = 0;
    thread_ = std::thread([this] { writer(); });
    return true;
}

uint64_t Mp4::tick(int64_t tsUs) const
{
    return uint64_t(tsUs - firstTs_) * TIMESCALE / 1000000;
}

void Mp4::add(const uint8_t *data, size_t len, int64_t tsUs, bool key)
{
    if (fd_ < 0)
        return;
    std::vector<std::pair<const uint8_t *, size_t>> units = nals(data, len);
    for (auto &u : units) {
        int type = u.first[0] & 0x1f;
        if (type == 7)
            sps_.assign(u.first, u.first + u.second);
        else if (type == 8)
            pps_.assign(u.first, u.first + u.second);
    }
    if (firstTs_ < 0) {
        if (!key || sps_.empty() || pps_.empty())
            return;
        firstTs_ = tsUs;
    }
    // A new second starts at each keyframe: the last one goes to disk.
    if (key && !samples_.empty())
        flush(tsUs);
    uint32_t size = 0;
    for (auto &u : units) {
        int type = u.first[0] & 0x1f;
        // Parameter sets live in avcC; access unit delimiters aren't needed.
        if (type == 7 || type == 8 || type == 9)
            continue;
        uint32_t n = uint32_t(u.second);
        uint8_t len4[4] = {uint8_t(n >> 24), uint8_t(n >> 16), uint8_t(n >> 8), uint8_t(n)};
        data_.insert(data_.end(), len4, len4 + 4);
        data_.insert(data_.end(), u.first, u.first + u.second);
        size += 4 + n;
    }
    if (size)
        samples_.push_back({size, tsUs, key});
    lastTs_ = tsUs;
}

void Mp4::addAudio(const uint8_t *data, size_t len, int64_t tsUs)
{
    if (fd_ < 0 || !hasAudio_ || firstTs_ < 0 || tsUs < firstTs_ || !len)
        return;
    if (aBase_ < 0)
        aBase_ = (tsUs - firstTs_) * audio_.rate / 1000000;
    aSizes_.push_back(uint32_t(len));
    aData_.insert(aData_.end(), data, data + len);
}

void Mp4::header()
{
    Boxes x;
    x.begin("ftyp");
    x.str("isom", 4);
    x.u32(0x200);
    x.str("isomiso6avc1mp41", 16);
    x.end();

    x.begin("moov");
    x.full("mvhd", 0, 0);
    x.u32(0);
    x.u32(0);
    x.u32(1000);
    x.u32(0);
    x.u32(0x00010000);
    x.u16(0x0100);
    x.zeros(10);
    x.matrix();
    x.zeros(24);
    x.u32(hasAudio_ ? 3 : 2);
    x.end();

    x.begin("trak");
    x.full("tkhd", 0, 3);
    x.u32(0);
    x.u32(0);
    x.u32(1);
    x.u32(0);
    x.u32(0);
    x.zeros(8);
    x.u16(0);
    x.u16(0);
    x.u16(0);
    x.u16(0);
    x.matrix();
    x.u32(uint32_t(w_) << 16);
    x.u32(uint32_t(h_) << 16);
    x.end();

    x.begin("mdia");
    x.full("mdhd", 0, 0);
    x.u32(0);
    x.u32(0);
    x.u32(TIMESCALE);
    x.u32(0);
    x.u16(0x55c4); // "und"
    x.u16(0);
    x.end();
    x.full("hdlr", 0, 0);
    x.u32(0);
    x.str("vide", 4);
    x.zeros(12);
    x.str("VideoHandler", 13);
    x.end();

    x.begin("minf");
    x.full("vmhd", 0, 1);
    x.zeros(8);
    x.end();
    x.begin("dinf");
    x.full("dref", 0, 0);
    x.u32(1);
    x.full("url ", 0, 1);
    x.end();
    x.end();
    x.end();

    x.begin("stbl");
    x.full("stsd", 0, 0);
    x.u32(1);
    x.begin("avc1");
    x.zeros(6);
    x.u16(1);
    x.zeros(16);
    x.u16(w_);
    x.u16(h_);
    x.u32(0x00480000);
    x.u32(0x00480000);
    x.u32(0);
    x.u16(1);
    x.zeros(32);
    x.u16(0x18);
    x.u16(0xffff);
    x.begin("avcC");
    x.u8(1);
    x.u8(sps_[1]);
    x.u8(sps_[2]);
    x.u8(sps_[3]);
    x.u8(0xff); // four-byte lengths
    x.u8(0xe1); // one SPS
    x.u16(uint32_t(sps_.size()));
    x.bytes(sps_);
    x.u8(1);
    x.u16(uint32_t(pps_.size()));
    x.bytes(pps_);
    if (sps_[1] == 100 || sps_[1] == 110 || sps_[1] == 122 || sps_[1] == 144) {
        x.u8(0xfc | 1); // 4:2:0
        x.u8(0xf8);     // 8-bit luma
        x.u8(0xf8);     // 8-bit chroma
        x.u8(0);
    }
    x.end();
    // BT.709, limited range: what the ISP gives for video.
    x.begin("colr");
    x.str("nclx", 4);
    x.u16(1);
    x.u16(1);
    x.u16(1);
    x.u8(0);
    x.end();
    x.end(); // avc1
    x.end(); // stsd
    for (const char *t : {"stts", "stsc", "stco"}) {
        x.full(t, 0, 0);
        x.u32(0);
        x.end();
    }
    x.full("stsz", 0, 0);
    x.u32(0);
    x.u32(0);
    x.end();
    x.end(); // stbl
    x.end(); // minf
    x.end(); // mdia
    x.end(); // trak
    if (hasAudio_)
        audioTrak(x, audio_);

    x.begin("mvex");
    x.full("trex", 0, 0);
    x.u32(1);
    x.u32(1);
    x.u32(0);
    x.u32(0);
    x.u32(0);
    x.end();
    if (hasAudio_) {
        x.full("trex", 0, 0);
        x.u32(2);
        x.u32(1);
        x.u32(AAC_FRAME);
        x.u32(0);
        x.u32(FLAGS_KEY);
        x.end();
    }
    x.end();
    x.end(); // moov
    queue(std::move(x.b));
    headed_ = true;
}

// One fragment: the second of video since the last keyframe, and the
// sound that came in that time. Sound arrives a little late, so it is not
// cut at the keyframe: what is queued goes, and the track stays whole.
void Mp4::flush(int64_t nextTs)
{
    if (samples_.empty() && aSizes_.empty())
        return;
    if (!headed_)
        header();
    Boxes x;
    x.begin("moof");
    x.full("mfhd", 0, 0);
    x.u32(++seq_);
    x.end();
    size_t vOffsetAt = 0, aOffsetAt = 0;
    if (!samples_.empty()) {
        x.begin("traf");
        x.full("tfhd", 0, 0x020000); // offsets count from this moof
        x.u32(1);
        x.end();
        x.full("tfdt", 1, 0);
        x.u64(tick(samples_[0].ts));
        x.end();
        x.full("trun", 0, 0x000701); // data offset; each sample's duration, size, flags
        x.u32(uint32_t(samples_.size()));
        vOffsetAt = x.b.size();
        x.u32(0);
        for (size_t i = 0; i < samples_.size(); i++) {
            uint32_t dur = lastDur_;
            if (i + 1 < samples_.size())
                dur = uint32_t(tick(samples_[i + 1].ts) - tick(samples_[i].ts));
            else if (nextTs > samples_[i].ts)
                dur = uint32_t(tick(nextTs) - tick(samples_[i].ts));
            lastDur_ = dur;
            x.u32(dur);
            x.u32(samples_[i].size);
            x.u32(samples_[i].key ? FLAGS_KEY : FLAGS_DELTA);
        }
        x.end(); // trun
        x.end(); // traf
    }
    if (!aSizes_.empty()) {
        x.begin("traf");
        // Offsets from this moof, and every frame 1024 samples and a sync sample.
        x.full("tfhd", 0, 0x020000 | 0x08 | 0x20);
        x.u32(2);
        x.u32(AAC_FRAME);
        x.u32(FLAGS_KEY);
        x.end();
        x.full("tfdt", 1, 0);
        x.u64(uint64_t(aBase_) + aDone_ * AAC_FRAME);
        x.end();
        x.full("trun", 0, 0x000201); // data offset; each sample's size
        x.u32(uint32_t(aSizes_.size()));
        aOffsetAt = x.b.size();
        x.u32(0);
        for (uint32_t s : aSizes_)
            x.u32(s);
        x.end(); // trun
        x.end(); // traf
    }
    x.end(); // moof
    auto put = [&](size_t at, uint32_t v) {
        x.b[at] = v >> 24;
        x.b[at + 1] = v >> 16;
        x.b[at + 2] = v >> 8;
        x.b[at + 3] = v;
    };
    uint32_t off = uint32_t(x.b.size() + 8);
    if (vOffsetAt)
        put(vOffsetAt, off);
    if (aOffsetAt)
        put(aOffsetAt, off + uint32_t(data_.size()));
    x.u32(uint32_t(data_.size() + aData_.size() + 8));
    x.str("mdat", 4);
    x.bytes(data_);
    x.bytes(aData_);
    queue(std::move(x.b));
    if (!samples_.empty())
        frags_++;
    aDone_ += aSizes_.size();
    samples_.clear();
    data_.clear();
    aSizes_.clear();
    aData_.clear();
}

void Mp4::queue(std::vector<uint8_t> &&b)
{
    written_ += b.size();
    {
        std::lock_guard<std::mutex> l(mu_);
        q_.push_back(std::move(b));
    }
    cv_.notify_one();
}

// Writes on a thread of its own: a card can stall for a while, and the
// preview must not.
void Mp4::writer()
{
    for (;;) {
        std::vector<uint8_t> b;
        {
            std::unique_lock<std::mutex> l(mu_);
            cv_.wait(l, [this] { return done_ || !q_.empty(); });
            if (q_.empty())
                return;
            b = std::move(q_.front());
            q_.pop_front();
        }
        size_t at = 0;
        while (at < b.size()) {
            ssize_t n = ::write(fd_, b.data() + at, b.size() - at);
            if (n < 0 && errno == EINTR)
                continue;
            if (n <= 0) {
                std::lock_guard<std::mutex> l(mu_);
                if (!bad_)
                    why_ = strerror(errno);
                bad_ = true;
                break;
            }
            at += size_t(n);
        }
        // Each second on the card before the next: a cut keeps it.
#ifdef __APPLE__
        fsync(fd_);
#else
        fdatasync(fd_);
#endif
    }
}

bool Mp4::failed()
{
    std::lock_guard<std::mutex> l(mu_);
    return bad_;
}

std::string Mp4::why()
{
    std::lock_guard<std::mutex> l(mu_);
    return why_;
}

void Mp4::finish()
{
    if (fd_ < 0)
        return;
    flush(-1);
    {
        std::lock_guard<std::mutex> l(mu_);
        done_ = true;
    }
    cv_.notify_one();
    if (thread_.joinable())
        thread_.join();
    fsync(fd_);
    ::close(fd_);
    fd_ = -1;
}

} // namespace camd
