#include "player.h"
#include "pulse.h"

#include <chrono>
#include <condition_variable>
#include <deque>
#include <mutex>
#include <thread>
#include <unistd.h>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/channel_layout.h>
#include <libavutil/imgutils.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>
}

namespace camd {

namespace {

// Read ahead at most this much picture; sound for the same second lies
// after it in the file, so the sound queue may hold more.
const size_t VIDEO_AHEAD = 90;
const size_t AUDIO_AHEAD = 400;
// A picture this late is dropped rather than shown.
const int64_t LATE_US = 70000;
// Packets sent with no picture out: the decoder has stopped.
const int STALL_PACKETS = 60;

int64_t nowUs()
{
    using namespace std::chrono;
    return duration_cast<microseconds>(steady_clock::now().time_since_epoch()).count();
}

struct Pkt {
    AVPacket *p;
    // Which seek it belongs to: one from before a seek is dropped.
    uint32_t serial;
};

} // namespace

struct Player::State {
    int wake;
    AVFormatContext *fmt = nullptr;
    AVCodecContext *vdec = nullptr, *adec = nullptr;
    int vi = -1, ai = -1;
    AVRational vtb{1, 1}, atb{1, 1};
    SwsContext *sws = nullptr;
    SwrContext *swr = nullptr;
    PaOut out;
    bool sound = false;
    int outW = 0, outH = 0;
    bool hw = false;

    std::mutex mu;
    std::condition_variable cv;
    std::deque<Pkt> vq, aq;
    bool eof = false, quit = false;
    uint32_t serial = 0;
    int64_t seekTo = -1;
    // The media clock: playing, media time = now - base - the sound's lag.
    int64_t base = 0, pausedAt = 0, lag = 0;
    bool paused = false;
    // Where a seek lands: earlier pictures and sound are decoded, not shown.
    int64_t skipUntil = 0;
    bool vDone = false, aDone = false;
    // Paused, a seek still shows the picture it lands on.
    bool showOne = false;

    std::vector<uint16_t> front;
    bool fresh = false;
    Status status;
    bool told = true;
    int64_t lastSecond = -1;

    std::thread demux, video, audio;

    int64_t mediaUs()
    {
        return paused ? pausedAt : nowUs() - base - lag;
    }

    void poke()
    {
        (void)!::write(wake, "p", 1);
    }

    // Under mu.
    void tell()
    {
        told = false;
        poke();
    }

    void readLoop()
    {
        AVPacket *p = av_packet_alloc();
        for (;;) {
            {
                std::unique_lock<std::mutex> l(mu);
                cv.wait(l, [&] {
                    return quit || seekTo >= 0 ||
                           (!eof && vq.size() < VIDEO_AHEAD && aq.size() < AUDIO_AHEAD);
                });
                if (quit)
                    break;
                if (seekTo >= 0) {
                    int64_t ts = seekTo * 1000;
                    av_seek_frame(fmt, -1, ts * AV_TIME_BASE / 1000000, AVSEEK_FLAG_BACKWARD);
                    for (auto &q : {&vq, &aq})
                        for (auto &k : *q)
                            av_packet_free(&k.p);
                    vq.clear();
                    aq.clear();
                    serial++;
                    skipUntil = ts;
                    base = nowUs() - ts - lag;
                    if (paused) {
                        pausedAt = ts;
                        showOne = true;
                    }
                    eof = false;
                    vDone = vi < 0;
                    aDone = ai < 0;
                    status.ended = false;
                    seekTo = -1;
                    cv.notify_all();
                    continue;
                }
            }
            int r = av_read_frame(fmt, p);
            std::lock_guard<std::mutex> l(mu);
            if (r < 0) {
                eof = true;
                cv.notify_all();
                continue;
            }
            if (p->stream_index == vi || p->stream_index == ai) {
                auto &q = p->stream_index == vi ? vq : aq;
                q.push_back({av_packet_clone(p), serial});
                cv.notify_all();
            }
            av_packet_unref(p);
        }
        av_packet_free(&p);
    }

    // Takes the next packet of `q`, or null at the end of the file. False:
    // quitting. `seeked`: a seek came since the last one, so the decoder
    // starts afresh.
    bool next(std::deque<Pkt> &q, Pkt &k, uint32_t &mine, bool &seeked)
    {
        std::unique_lock<std::mutex> l(mu);
        seeked = false;
        for (;;) {
            cv.wait(l, [&] { return quit || !q.empty() || eof || serial != mine; });
            if (quit)
                return false;
            if (serial != mine) {
                mine = serial;
                seeked = true;
                continue;
            }
            if (!q.empty()) {
                k = q.front();
                q.pop_front();
                cv.notify_all();
                if (k.serial != mine) {
                    av_packet_free(&k.p);
                    continue;
                }
                return true;
            }
            k = {nullptr, mine};
            return true;
        }
    }

    // Waits until `us` on the media clock. False: a seek or quit came.
    // Paused, it sleeps until something changes: every change notifies.
    bool waitFor(int64_t us, uint32_t mine)
    {
        std::unique_lock<std::mutex> l(mu);
        for (;;) {
            if (quit || serial != mine)
                return false;
            if (paused && showOne) {
                showOne = false;
                return true;
            }
            if (paused) {
                cv.wait(l);
                continue;
            }
            int64_t ahead = us - mediaUs();
            if (ahead <= 0)
                return true;
            cv.wait_for(l, std::chrono::microseconds(ahead));
        }
    }

    void show(AVFrame *f)
    {
        sws = sws_getCachedContext(sws, f->width, f->height, AVPixelFormat(f->format), outW, outH,
                                   AV_PIX_FMT_RGB565LE, SWS_BILINEAR, nullptr, nullptr, nullptr);
        if (!sws)
            return;
        // Ours are BT.709 in video range; the frame says what it is.
        int cs = f->colorspace == AVCOL_SPC_BT470BG || f->colorspace == AVCOL_SPC_SMPTE170M
                     ? SWS_CS_ITU601
                     : SWS_CS_ITU709;
        int full = f->color_range == AVCOL_RANGE_JPEG || f->format == AV_PIX_FMT_YUVJ420P;
        const int *coef = sws_getCoefficients(cs);
        sws_setColorspaceDetails(sws, coef, full, coef, 1, 0, 1 << 16, 1 << 16);
        std::vector<uint16_t> px(size_t(outW) * outH);
        uint8_t *dst[4] = {reinterpret_cast<uint8_t *>(px.data()), nullptr, nullptr, nullptr};
        int stride[4] = {outW * 2, 0, 0, 0};
        sws_scale(sws, f->data, f->linesize, 0, f->height, dst, stride);
        std::lock_guard<std::mutex> l(mu);
        front.swap(px);
        fresh = true;
        poke();
    }

    // One decoded picture: shown on time, dropped when late or before a
    // seek's mark.
    void picture(AVFrame *f, uint32_t mine)
    {
        int64_t pts = f->best_effort_timestamp;
        int64_t us = pts == AV_NOPTS_VALUE ? 0 : av_rescale_q(pts, vtb, AVRational{1, 1000000});
        {
            std::lock_guard<std::mutex> l(mu);
            if (us < skipUntil)
                return;
        }
        if (!waitFor(us, mine))
            return;
        {
            std::lock_guard<std::mutex> l(mu);
            if (!paused && mediaUs() - us > LATE_US)
                return;
        }
        show(f);
        std::lock_guard<std::mutex> l(mu);
        status.posMs = us / 1000;
        if (us / 1000000 != lastSecond) {
            lastSecond = us / 1000000;
            tell();
        }
    }

    // A seek: the decoder starts afresh. The VPU's decoder is opened again,
    // as not every build of it can flush.
    void resetVideo()
    {
        if (!hw) {
            avcodec_flush_buffers(vdec);
            return;
        }
        std::string why;
        avcodec_free_context(&vdec);
        if (!openDecoder(vi, vdec, why, true))
            giveUp(why);
    }

    // No picture for a while: the VPU once more in software, then an error.
    void stalled()
    {
        std::string why = "can't decode this video";
        if (hw) {
            fprintf(stderr, "gh0st-camd: the hardware decoder gave nothing, trying software\n");
            avcodec_free_context(&vdec);
            if (openDecoder(vi, vdec, why, false)) {
                std::lock_guard<std::mutex> l(mu);
                seekTo = status.posMs;
                cv.notify_all();
                return;
            }
        }
        giveUp(why);
    }

    void giveUp(const std::string &why)
    {
        std::lock_guard<std::mutex> l(mu);
        if (status.error.empty())
            status.error = why;
        tell();
    }

    void videoLoop()
    {
        AVFrame *f = av_frame_alloc();
        uint32_t mine = 0;
        bool seeked;
        Pkt k;
        // Packets sent since the last picture came out.
        int stall = 0;
        auto drain = [&] {
            while (vdec && avcodec_receive_frame(vdec, f) == 0) {
                stall = 0;
                picture(f, mine);
                av_frame_unref(f);
            }
        };
        while (next(vq, k, mine, seeked)) {
            if (seeked) {
                resetVideo();
                stall = 0;
            }
            if (!vdec) {
                av_packet_free(&k.p);
                continue;
            }
            if (!k.p) {
                // The end: what the decoder held comes out, then a seek or
                // quit is awaited.
                avcodec_send_packet(vdec, nullptr);
                drain();
                std::unique_lock<std::mutex> l(mu);
                vDone = true;
                end_();
                cv.wait(l, [&] { return quit || serial != mine; });
                continue;
            }
            // Full: take pictures out, then send the same packet again. A
            // bad packet is skipped; the next keyframe heals the picture.
            int r;
            for (int tries = 0; (r = avcodec_send_packet(vdec, k.p)) == AVERROR(EAGAIN) && tries < 50;
                 tries++)
                drain();
            av_packet_free(&k.p);
            stall++;
            drain();
            if (stall > STALL_PACKETS) {
                stall = 0;
                stalled();
            }
        }
        av_frame_free(&f);
    }

    void audioLoop()
    {
        AVFrame *f = av_frame_alloc();
        uint32_t mine = 0;
        bool seeked;
        Pkt k;
        std::vector<int16_t> pcm;
        int ch = adec->ch_layout.nb_channels > 1 ? 2 : 1;
        // Paused or played out, the stream closes so the codec can sleep.
        auto rest = [&](auto until) {
            std::unique_lock<std::mutex> l(mu);
            if (!until())
                return;
            l.unlock();
            out.close();
            l.lock();
            cv.wait(l, [&] { return !until(); });
        };
        while (next(aq, k, mine, seeked)) {
            if (seeked) {
                avcodec_flush_buffers(adec);
                // What the server still holds is from before the seek.
                if (out.isOpen())
                    out.flush();
            }
            if (!k.p) {
                {
                    std::lock_guard<std::mutex> l(mu);
                    aDone = true;
                    end_();
                }
                rest([&] { return !quit && serial == mine; });
                continue;
            }
            int r;
            for (int tries = 0; (r = avcodec_send_packet(adec, k.p)) == AVERROR(EAGAIN) && tries < 50;
                 tries++) {
                while (avcodec_receive_frame(adec, f) == 0)
                    av_frame_unref(f);
            }
            av_packet_free(&k.p);
            while (avcodec_receive_frame(adec, f) == 0) {
                int64_t us = av_rescale_q(f->best_effort_timestamp, atb, AVRational{1, 1000000});
                rest([&] { return paused && !quit && serial == mine; });
                bool drop, wait;
                {
                    std::lock_guard<std::mutex> l(mu);
                    drop = us < skipUntil || serial != mine || quit;
                    // Far behind the picture (a slow card): catch up.
                    drop = drop || mediaUs() - us > 200000;
                    wait = us - mediaUs() > 200000;
                }
                if (wait && !waitFor(us - 100000, mine))
                    drop = true;
                std::string why;
                if (!drop && !out.isOpen() && !out.open(adec->sample_rate, ch, why))
                    drop = true;
                if (!drop) {
                    pcm.resize(size_t(f->nb_samples) * ch + 64);
                    uint8_t *dst = reinterpret_cast<uint8_t *>(pcm.data());
                    int n = swr_convert(swr, &dst, f->nb_samples, f->extended_data, f->nb_samples);
                    if (n > 0)
                        out.write(pcm.data(), n);
                }
                av_frame_unref(f);
            }
        }
        av_frame_free(&f);
    }

    // Under mu: both streams played out.
    void end_()
    {
        if (vDone && aDone && !status.ended) {
            status.ended = true;
            paused = true;
            pausedAt = status.posMs * 1000;
            tell();
        }
    }

    bool openDecoder(int idx, AVCodecContext *&ctx, std::string &why, bool allowHw)
    {
        AVStream *st = fmt->streams[idx];
        const AVCodec *codec = nullptr;
        // The VPU decodes H.264 up to 1080p; software is the fallback.
#ifdef __linux__
        if (allowHw && st->codecpar->codec_id == AV_CODEC_ID_H264)
            codec = avcodec_find_decoder_by_name("h264_v4l2m2m");
#else
        (void)allowHw;
#endif
        for (int tries = 0; tries < 2; tries++) {
            if (!codec)
                codec = avcodec_find_decoder(st->codecpar->codec_id);
            if (!codec)
                break;
            ctx = avcodec_alloc_context3(codec);
            avcodec_parameters_to_context(ctx, st->codecpar);
            ctx->pkt_timebase = st->time_base;
            if (st->codecpar->codec_type == AVMEDIA_TYPE_VIDEO && !(codec->capabilities & AV_CODEC_CAP_HARDWARE)) {
                ctx->thread_count = 4;
                ctx->thread_type = FF_THREAD_FRAME | FF_THREAD_SLICE;
            }
            if (avcodec_open2(ctx, codec, nullptr) == 0) {
                if (st->codecpar->codec_type == AVMEDIA_TYPE_VIDEO)
                    hw = codec->capabilities & AV_CODEC_CAP_HARDWARE;
                return true;
            }
            avcodec_free_context(&ctx);
            codec = nullptr;
        }
        why = "can't decode this video";
        return false;
    }

    void freeAll()
    {
        {
            std::lock_guard<std::mutex> l(mu);
            quit = true;
            cv.notify_all();
        }
        for (auto *t : {&demux, &video, &audio})
            if (t->joinable())
                t->join();
        for (auto &q : {&vq, &aq})
            for (auto &k : *q)
                av_packet_free(&k.p);
        vq.clear();
        aq.clear();
        out.close();
        avcodec_free_context(&vdec);
        avcodec_free_context(&adec);
        sws_freeContext(sws);
        sws = nullptr;
        swr_free(&swr);
        avformat_close_input(&fmt);
    }
};

Player::Player(int wakeFd) : wake_(wakeFd) {}

Player::~Player()
{
    close();
}

bool Player::open(const std::string &path, int w, int h, std::string &why)
{
    close();
    s_ = std::make_unique<State>();
    State &s = *s_;
    s.wake = wake_;
    if (avformat_open_input(&s.fmt, path.c_str(), nullptr, nullptr) < 0 ||
        avformat_find_stream_info(s.fmt, nullptr) < 0) {
        why = "can't read this video";
        close();
        return false;
    }
    s.vi = av_find_best_stream(s.fmt, AVMEDIA_TYPE_VIDEO, -1, -1, nullptr, 0);
    s.ai = av_find_best_stream(s.fmt, AVMEDIA_TYPE_AUDIO, -1, -1, nullptr, 0);
    if (s.vi < 0 || !s.openDecoder(s.vi, s.vdec, why, true)) {
        why = s.vi < 0 ? "no picture in this file" : why;
        close();
        return false;
    }
    s.vtb = s.fmt->streams[s.vi]->time_base;
    // Fitted, with even sides, as large as the screen allows.
    int vw = s.vdec->width, vh = s.vdec->height;
    if (vw <= 0 || vh <= 0) {
        why = "can't read this video";
        close();
        return false;
    }
    if (int64_t(vw) * h > int64_t(vh) * w) {
        s.outW = w;
        s.outH = std::max(2, int(int64_t(vh) * w / vw) & ~1);
    } else {
        s.outH = h;
        s.outW = std::max(2, int(int64_t(vw) * h / vh) & ~1);
    }
    std::string none;
    if (s.ai >= 0 && s.openDecoder(s.ai, s.adec, none, false)) {
        s.atb = s.fmt->streams[s.ai]->time_base;
        int ch = s.adec->ch_layout.nb_channels > 1 ? 2 : 1;
        AVChannelLayout layout;
        av_channel_layout_default(&layout, ch);
        if (swr_alloc_set_opts2(&s.swr, &layout, AV_SAMPLE_FMT_S16, s.adec->sample_rate,
                                &s.adec->ch_layout, s.adec->sample_fmt, s.adec->sample_rate, 0,
                                nullptr) == 0 &&
            swr_init(s.swr) == 0 && s.out.open(s.adec->sample_rate, ch, none)) {
            s.sound = true;
        } else {
            // No sound server: the picture plays on, silent.
            fprintf(stderr, "gh0st-camd: %s: playing without sound: %s\n", path.c_str(),
                    none.c_str());
            avcodec_free_context(&s.adec);
            swr_free(&s.swr);
            s.ai = -1;
        }
    }
    if (s.fmt->duration > 0)
        s.status.durMs = s.fmt->duration / (AV_TIME_BASE / 1000);
    s.vDone = false;
    s.aDone = s.ai < 0;
    s.lag = s.sound ? 100000 : 0;
    s.base = nowUs() - s.lag;
    s.demux = std::thread([&s] { s.readLoop(); });
    s.video = std::thread([&s] { s.videoLoop(); });
    if (s.ai >= 0)
        s.audio = std::thread([&s] { s.audioLoop(); });
    fprintf(stderr, "gh0st-camd: playing %s, %dx%d to %dx%d, %s decoder, %s\n", path.c_str(), vw, vh,
            s.outW, s.outH, s.hw ? "hardware" : "software", s.sound ? "with sound" : "silent");
    return true;
}

bool Player::active() const
{
    return s_ != nullptr;
}

void Player::pause(bool on)
{
    if (!s_)
        return;
    State &s = *s_;
    int64_t at;
    {
        std::lock_guard<std::mutex> l(s.mu);
        if (on == s.paused && !s.status.ended)
            return;
        if (on) {
            s.pausedAt = s.mediaUs();
            s.paused = true;
            s.status.paused = true;
            s.cv.notify_all();
            s.tell();
            return;
        }
        // Play again from the picture on screen: a seek there puts the
        // sound back in step, as the server played on a little.
        at = s.status.ended ? 0 : s.status.posMs;
        s.paused = false;
        s.status.paused = false;
        s.tell();
    }
    seek(at);
}

void Player::seek(int64_t ms)
{
    if (!s_)
        return;
    State &s = *s_;
    std::lock_guard<std::mutex> l(s.mu);
    if (s.status.durMs > 0)
        ms = std::min(ms, s.status.durMs);
    s.seekTo = std::max<int64_t>(0, ms);
    s.status.posMs = s.seekTo;
    s.lastSecond = -1;
    s.cv.notify_all();
    s.tell();
}

void Player::close()
{
    if (!s_)
        return;
    s_->freeAll();
    s_.reset();
}

bool Player::picture(std::vector<uint16_t> &px, int &w, int &h)
{
    if (!s_)
        return false;
    std::lock_guard<std::mutex> l(s_->mu);
    if (!s_->fresh)
        return false;
    s_->fresh = false;
    px.swap(s_->front);
    w = s_->outW;
    h = s_->outH;
    return true;
}

bool Player::news(Status &out)
{
    if (!s_)
        return false;
    std::lock_guard<std::mutex> l(s_->mu);
    if (s_->told)
        return false;
    s_->told = true;
    out = s_->status;
    out.paused = s_->paused;
    return true;
}

} // namespace camd
