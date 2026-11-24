#include "pulse.h"

#include <pulse/error.h>
#include <pulse/pulseaudio.h>
#include <pulse/simple.h>

namespace camd {

namespace {

const char *APP = "Camera";

pa_sample_spec spec(int rate, int channels)
{
    pa_sample_spec s;
    s.format = PA_SAMPLE_S16LE;
    s.rate = uint32_t(rate);
    s.channels = uint8_t(channels);
    return s;
}

// About 100 ms buffered each way: the simple API's default is two seconds,
// which a pause or the picture would wait out.
pa_buffer_attr attr(const pa_sample_spec &s)
{
    pa_buffer_attr a;
    uint32_t chunk = uint32_t(pa_usec_to_bytes(100000, &s));
    a.maxlength = uint32_t(-1);
    a.tlength = chunk;
    a.prebuf = uint32_t(-1);
    a.minreq = uint32_t(-1);
    a.fragsize = uint32_t(pa_usec_to_bytes(20000, &s));
    return a;
}

void onSource(pa_context *, const pa_source_info *i, int eol, void *u)
{
    auto *out = static_cast<std::vector<Source> *>(u);
    if (eol || !i || i->monitor_of_sink != PA_INVALID_INDEX)
        return;
    out->push_back({i->name ? i->name : "", i->description ? i->description : ""});
}

} // namespace

std::vector<Source> sources()
{
    std::vector<Source> out;
    pa_mainloop *ml = pa_mainloop_new();
    if (!ml)
        return out;
    pa_context *c = pa_context_new(pa_mainloop_get_api(ml), APP);
    if (c && pa_context_connect(c, nullptr, PA_CONTEXT_NOFLAGS, nullptr) >= 0) {
        pa_operation *op = nullptr;
        for (int spins = 0; spins < 2000; spins++) {
            pa_context_state_t st = pa_context_get_state(c);
            if (!PA_CONTEXT_IS_GOOD(st))
                break;
            if (st == PA_CONTEXT_READY && !op)
                op = pa_context_get_source_info_list(c, onSource, &out);
            if (op && pa_operation_get_state(op) != PA_OPERATION_RUNNING)
                break;
            if (pa_mainloop_iterate(ml, 1, nullptr) < 0)
                break;
        }
        if (op)
            pa_operation_unref(op);
        pa_context_disconnect(c);
    }
    if (c)
        pa_context_unref(c);
    pa_mainloop_free(ml);
    return out;
}

std::string autoSource(const std::vector<Source> &all)
{
    for (auto &s : all)
        if (s.name.rfind("alsa_input.platform-", 0) != 0)
            return s.name;
    return "";
}

PaIn::~PaIn()
{
    close();
}

bool PaIn::open(const std::string &dev, int rate, int channels, std::string &why)
{
    close();
    pa_sample_spec s = spec(rate, channels);
    pa_buffer_attr a = attr(s);
    int err = 0;
    s_ = pa_simple_new(nullptr, APP, PA_STREAM_RECORD, dev.empty() ? nullptr : dev.c_str(),
                       "Video", &s, nullptr, &a, &err);
    if (!s_) {
        why = pa_strerror(err);
        return false;
    }
    bytesPerFrame_ = 2 * channels;
    rate_ = rate;
    return true;
}

bool PaIn::read(int16_t *buf, int frames)
{
    int err = 0;
    return s_ && pa_simple_read(static_cast<pa_simple *>(s_), buf, size_t(frames) * bytesPerFrame_,
                                &err) >= 0;
}

int64_t PaIn::latencyUs()
{
    int err = 0;
    pa_usec_t l = s_ ? pa_simple_get_latency(static_cast<pa_simple *>(s_), &err) : 0;
    return l == pa_usec_t(-1) ? 0 : int64_t(l);
}

void PaIn::close()
{
    if (s_)
        pa_simple_free(static_cast<pa_simple *>(s_));
    s_ = nullptr;
}

PaOut::~PaOut()
{
    close();
}

bool PaOut::open(int rate, int channels, std::string &why)
{
    close();
    pa_sample_spec s = spec(rate, channels);
    pa_buffer_attr a = attr(s);
    int err = 0;
    s_ = pa_simple_new(nullptr, APP, PA_STREAM_PLAYBACK, nullptr, "Video", &s, nullptr, &a, &err);
    if (!s_) {
        why = pa_strerror(err);
        return false;
    }
    bytesPerFrame_ = 2 * channels;
    rate_ = rate;
    return true;
}

bool PaOut::write(const int16_t *buf, int frames)
{
    int err = 0;
    return s_ && pa_simple_write(static_cast<pa_simple *>(s_), buf,
                                 size_t(frames) * bytesPerFrame_, &err) >= 0;
}

int64_t PaOut::latencyUs()
{
    int err = 0;
    pa_usec_t l = s_ ? pa_simple_get_latency(static_cast<pa_simple *>(s_), &err) : 0;
    return l == pa_usec_t(-1) ? 0 : int64_t(l);
}

void PaOut::flush()
{
    int err = 0;
    if (s_)
        pa_simple_flush(static_cast<pa_simple *>(s_), &err);
}

void PaOut::close()
{
    if (s_)
        pa_simple_free(static_cast<pa_simple *>(s_));
    s_ = nullptr;
}

} // namespace camd
