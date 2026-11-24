#include "rec.h"
#include "mic.h"
#include "mp4.h"
#include "venc.h"

#include <cstring>

namespace camd {

namespace {
const int FPS = 30;
} // namespace

struct Recorder::State {
    Backend &be;
    Venc venc;
    Mp4 mp4;
    Mic mic;
    std::deque<Mic::Packet> sound;
    bool micTold = false;
    std::string path;
    int frags = 0;
    int dropped = 0;
    explicit State(Backend &b) : be(b) {}
    void released(void *t) { be.release(t); }
    void unit(const uint8_t *d, size_t n, int64_t ts, bool key) { mp4.add(d, n, ts, key); }
    // Sound joins the fragment being built. It runs a little behind the
    // picture, and its track keeps its own time, so nothing is cut.
    void audio()
    {
        mic.take(sound);
        while (!sound.empty()) {
            mp4.addAudio(sound.front().data.data(), sound.front().data.size(),
                         sound.front().tsUs);
            sound.pop_front();
        }
    }
};

Recorder::Recorder(Backend &be) : s_(std::make_unique<State>(be)) {}

Recorder::~Recorder()
{
    stop();
}

bool Recorder::start(const std::string &path, bool mic, const std::string &source,
                     std::string &why, std::string &micWhy)
{
    int w, h, stride;
    if (!s_->be.videoStream(w, h, stride)) {
        why = "not in a video mode";
        return false;
    }
    // About 94 MB a minute at 1080p, as the app counts the room left.
    int bitrate = w >= 1920 ? 12000000 : 8000000;
    AudioTrack track;
    bool sound = mic && s_->mic.open(source, track, micWhy);
    if (!s_->venc.open(w, h, stride, FPS, bitrate, why)) {
        s_->mic.close();
        return false;
    }
    if (!s_->mp4.open(path, w, h, sound ? &track : nullptr, why)) {
        s_->venc.close([this](void *t) { s_->released(t); });
        s_->mic.close();
        return false;
    }
    s_->sound.clear();
    s_->micTold = false;
    s_->path = path;
    s_->frags = 0;
    s_->dropped = 0;
    return true;
}

bool Recorder::active() const
{
    return s_->venc.isOpen();
}

int Recorder::fd() const
{
    return s_->venc.fd();
}

bool Recorder::frame(const VideoFrame &f)
{
    if (!active())
        return false;
    if (s_->venc.push(f.fd, f.size, f.tsUs, f.token))
        return true;
    // Full: take back what the encoder finished since the loop last
    // looked, then try once more.
    service();
    if (s_->venc.push(f.fd, f.size, f.tsUs, f.token))
        return true;
    s_->dropped++;
    return false;
}

bool Recorder::service()
{
    if (!active())
        return false;
    s_->venc.service([this](void *t) { s_->released(t); },
                     [this](const uint8_t *d, size_t n, int64_t ts, bool k) { s_->unit(d, n, ts, k); });
    s_->audio();
    if (s_->mp4.fragments() != s_->frags) {
        s_->frags = s_->mp4.fragments();
        return true;
    }
    return false;
}

void Recorder::stop()
{
    if (!active())
        return;
    auto rel = [this](void *t) { s_->released(t); };
    auto unit = [this](const uint8_t *d, size_t n, int64_t ts, bool k) { s_->unit(d, n, ts, k); };
    s_->venc.drain(500, rel, unit);
    s_->venc.close(rel);
    s_->mic.close();
    s_->audio();
    s_->mp4.finish();
}

int64_t Recorder::ms() const
{
    return s_->mp4.durationUs() / 1000;
}

uint64_t Recorder::bytes() const
{
    return s_->mp4.bytes();
}

int Recorder::dropped() const
{
    return s_->dropped;
}

bool Recorder::failed()
{
    return s_->mp4.failed();
}

std::string Recorder::why()
{
    return s_->mp4.why();
}

bool Recorder::micLost(std::string &why)
{
    if (s_->micTold || !s_->mic.failed())
        return false;
    s_->micTold = true;
    why = s_->mic.why();
    return true;
}

const std::string &Recorder::path() const
{
    return s_->path;
}

} // namespace camd
