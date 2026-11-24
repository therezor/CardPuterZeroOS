#include "mic.h"
#include "camd.h"
#include "pulse.h"

#include <algorithm>
#include <chrono>
#include <cmath>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/channel_layout.h>
#include <libavutil/opt.h>
}

namespace camd {

namespace {

const int RATE = 48000;
// The board has one microphone; a plugged-in one is mono too, or folded
// to it by the server.
const int CHANNELS = 1;
const int BITRATE = 96000;
// The board's microphone comes in quiet (M5's Recorder normalises on
// playback). A slow gain brings speech up: at most +24 dB, down at once
// before a peak clips, up 3 dB a second.
const float GAIN_MAX = 16.0f, GAIN_START = 8.0f, PEAK_TARGET = 0.5f;
const float GAIN_UP_DB_S = 3.0f;

int64_t nowUs()
{
    using namespace std::chrono;
    return duration_cast<microseconds>(steady_clock::now().time_since_epoch()).count();
}

} // namespace

Mic::~Mic()
{
    close();
}

bool Mic::open(const std::string &source, AudioTrack &track, std::string &why)
{
    close();
    const AVCodec *codec = avcodec_find_encoder(AV_CODEC_ID_AAC);
    if (!codec) {
        why = "no AAC encoder";
        return false;
    }
    AVCodecContext *c = avcodec_alloc_context3(codec);
    c->sample_rate = RATE;
    c->sample_fmt = AV_SAMPLE_FMT_FLTP;
    av_channel_layout_default(&c->ch_layout, CHANNELS);
    c->bit_rate = BITRATE;
    c->profile = AV_PROFILE_AAC_LOW;
    c->time_base = AVRational{1, RATE};
    // The MP4 carries the config in its esds, not in each frame.
    c->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    if (avcodec_open2(c, codec, nullptr) < 0) {
        avcodec_free_context(&c);
        why = "the AAC encoder would not open";
        return false;
    }
    track.rate = RATE;
    track.channels = CHANNELS;
    track.asc.assign(c->extradata, c->extradata + c->extradata_size);
    enc_ = c;
    source_ = source;
    quit_ = false;
    failed_ = false;
    thread_ = std::thread([this] { run(); });
    return true;
}

void Mic::run()
{
    auto *c = static_cast<AVCodecContext *>(enc_);
    PaIn in;
    {
        std::string why;
        std::string dev = source_.empty() ? autoSource(sources()) : source_;
        // The one picked may be unplugged now: then the one Auto would take.
        bool ok = in.open(dev, RATE, CHANNELS, why) ||
                  (!source_.empty() && in.open(autoSource(sources()), RATE, CHANNELS, why));
        if (!ok) {
            std::lock_guard<std::mutex> l(mu_);
            why_ = why;
            failed_ = true;
            return;
        }
    }
    const int n = c->frame_size > 0 ? c->frame_size : 1024;
    std::vector<int16_t> pcm(size_t(n) * CHANNELS);
    AVFrame *f = av_frame_alloc();
    AVPacket *p = av_packet_alloc();
    f->format = c->sample_fmt;
    f->nb_samples = n;
    av_channel_layout_copy(&f->ch_layout, &c->ch_layout);
    f->sample_rate = RATE;
    av_frame_get_buffer(f, 0);
    // When the first sample was heard, on the sensor's clock.
    int64_t t0 = -1, samples = 0;
    float gain = GAIN_START;
    const float up = std::pow(10.0f, GAIN_UP_DB_S * n / RATE / 20.0f);
    auto drain = [&] {
        while (avcodec_receive_packet(c, p) == 0) {
            // pts counts input samples, less the encoder's priming.
            Packet out{std::vector<uint8_t>(p->data, p->data + p->size),
                       t0 + p->pts * 1000000 / RATE};
            av_packet_unref(p);
            std::lock_guard<std::mutex> l(mu_);
            q_.push_back(std::move(out));
        }
    };
    while (!quit_) {
        if (!in.read(pcm.data(), n)) {
            std::lock_guard<std::mutex> l(mu_);
            why_ = "the microphone stopped";
            failed_ = true;
            break;
        }
        if (t0 < 0)
            t0 = nowUs() - in.latencyUs() - int64_t(n) * 1000000 / RATE;
        av_frame_make_writable(f);
        auto *dst = reinterpret_cast<float *>(f->data[0]);
        float peak = 1e-6f;
        for (int i = 0; i < n; i++)
            peak = std::max(peak, std::fabs(pcm[size_t(i)] / 32768.0f));
        // Ramped across the block, so a change makes no click.
        float next = std::min({GAIN_MAX, PEAK_TARGET / peak, gain * up});
        for (int i = 0; i < n; i++) {
            float g = gain + (next - gain) * float(i) / float(n);
            dst[i] = std::clamp(pcm[size_t(i)] / 32768.0f * g, -1.0f, 1.0f);
        }
        gain = next;
        f->pts = samples;
        samples += n;
        if (avcodec_send_frame(c, f) == 0)
            drain();
    }
    avcodec_send_frame(c, nullptr);
    drain();
    av_packet_free(&p);
    av_frame_free(&f);
}

std::string Mic::why()
{
    std::lock_guard<std::mutex> l(mu_);
    return why_;
}

void Mic::take(std::deque<Packet> &out)
{
    std::lock_guard<std::mutex> l(mu_);
    while (!q_.empty()) {
        out.push_back(std::move(q_.front()));
        q_.pop_front();
    }
}

void Mic::close()
{
    quit_ = true;
    if (thread_.joinable())
        thread_.join();
    if (enc_) {
        auto *c = static_cast<AVCodecContext *>(enc_);
        avcodec_free_context(&c);
        enc_ = nullptr;
    }
}

} // namespace camd
