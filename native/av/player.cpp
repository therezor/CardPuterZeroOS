#include "player.h"
#include "decode.h"
#include "pulse.h"

#include <algorithm>
#include <cerrno>
#include <chrono>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <mutex>
#include <sys/stat.h>
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

// Speed and subtitles need libavfilter and libass, which the media helper
// loads when asked (lazy.h) and the camera helper, playing its own clips,
// does without.
#ifdef GH0ST_PLAYER_EXTRAS
extern "C" {
#include <libavfilter/avfilter.h>
#include <libavfilter/buffersink.h>
#include <libavfilter/buffersrc.h>
}
#include <ass/ass.h>
#include "lazy.h"
GH0ST_LAZY(gh0st::lazy::ASS, ass_free_track)
GH0ST_LAZY(gh0st::lazy::ASS, ass_library_done)
GH0ST_LAZY(gh0st::lazy::ASS, ass_library_init)
GH0ST_LAZY(gh0st::lazy::ASS, ass_new_track)
GH0ST_LAZY(gh0st::lazy::ASS, ass_process_chunk)
GH0ST_LAZY(gh0st::lazy::ASS, ass_process_codec_private)
GH0ST_LAZY(gh0st::lazy::ASS, ass_render_frame)
GH0ST_LAZY(gh0st::lazy::ASS, ass_renderer_done)
GH0ST_LAZY(gh0st::lazy::ASS, ass_renderer_init)
GH0ST_LAZY(gh0st::lazy::ASS, ass_set_font_scale)
GH0ST_LAZY(gh0st::lazy::ASS, ass_set_fonts)
GH0ST_LAZY(gh0st::lazy::ASS, ass_set_frame_size)
GH0ST_LAZY(gh0st::lazy::AVFILTER, av_buffersink_get_frame)
GH0ST_LAZY(gh0st::lazy::AVFILTER, av_buffersrc_add_frame_flags)
GH0ST_LAZY(gh0st::lazy::AVFILTER, avfilter_graph_alloc)
GH0ST_LAZY(gh0st::lazy::AVFILTER, avfilter_graph_config)
GH0ST_LAZY(gh0st::lazy::AVFILTER, avfilter_graph_free)
GH0ST_LAZY(gh0st::lazy::AVFILTER, avfilter_graph_parse_ptr)

#else
struct AVFilterGraph;
struct AVFilterContext;
struct ASS_Library;
struct ASS_Renderer;
struct ASS_Track;
#endif

namespace gh0st {

namespace {

// Read ahead at most this much picture; sound for the same second lies
// after it in the file, so the sound queue may hold more.
const size_t VIDEO_AHEAD = 90;
const size_t AUDIO_AHEAD = 400;
// A picture this late is dropped rather than shown.
const int64_t LATE_US = 70000;
// Packets sent with no picture out: the decoder has stopped.
const int STALL_PACKETS = 60;
// Keeping up is judged over this long: past a quarter of the pictures
// dropped, the decoder skips more work, a step at a time.
const int64_t SLOW_WINDOW_US = 2000000;
const int SLOWEST = 4;
// Subtitles drawn this much larger than libass would, so a line on a
// 170-row screen stays readable.
const double SUB_SCALE = 1.6;

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

// A subtitle that is a picture (DVD, DVB, PGS), on the video's canvas.
struct SubPic {
    int64_t startMs, endMs;
    int x, y, w, h;
    std::vector<uint32_t> argb;
};

inline uint16_t mix565(uint16_t d, int r, int g, int b, int a)
{
    int dr = (d >> 11) << 3, dg = ((d >> 5) & 63) << 2, db = (d & 31) << 3;
    return pack565(dr + (r - dr) * a / 255, dg + (g - dg) * a / 255, db + (b - db) * a / 255);
}

#ifdef GH0ST_PLAYER_EXTRAS
std::string sidecarSrt(const std::string &path)
{
    size_t dot = path.find_last_of('.');
    if (dot == std::string::npos || dot < path.find_last_of('/') + 1)
        return "";
    std::string srt = path.substr(0, dot) + ".srt";
    struct stat st;
    return stat(srt.c_str(), &st) == 0 ? srt : "";
}
#endif

} // namespace

struct Player::State {
    int wake;
    Options opt;
    std::string path;
    AVFormatContext *fmt = nullptr;
    AVCodecContext *vdec = nullptr, *adec = nullptr, *sdec = nullptr;
    int vi = -1, ai = -1, si = -1;
    AVRational vtb{1, 1}, atb{1, 1}, stb{1, 1};
    SwsContext *sws = nullptr;
    SwrContext *swr = nullptr;
    // atempo, while the speed is not 100.
    AVFilterGraph *graph = nullptr;
    AVFilterContext *asrc = nullptr, *asink = nullptr;
    int graphSpeed = 100;
    PaOut out;
    bool sound = false;
    // The picture as shown, and as decoded before it is turned.
    int outW = 0, outH = 0, convW = 0, convH = 0;
    bool hw = false;

    // Subtitles. libass is not thread-safe: the read thread adds events,
    // the picture thread draws them, both under assMu.
    std::mutex assMu;
    ASS_Library *assLib = nullptr;
    ASS_Renderer *assR = nullptr;
    ASS_Track *assT = nullptr;
    std::vector<SubPic> subPics;
    int subW = 0, subH = 0;

    std::mutex mu;
    std::condition_variable cv;
    std::deque<Pkt> vq, aq;
    bool eof = false, quit = false;
    uint32_t serial = 0;
    int64_t seekTo = -1;
    // The media clock: playing, media time = (now - base) at the speed,
    // less the sound's lag.
    int64_t base = 0, pausedAt = 0, lag = 0;
    int speed = 100;
    bool paused = false;
    // Where a seek lands: earlier pictures and sound are decoded, not shown.
    int64_t skipUntil = 0;
    bool vDone = false, aDone = false;
    // Paused, a seek still shows the picture it lands on.
    bool showOne = false;
    // A to B, in ms; off while b <= a.
    int64_t repA = -1, repB = -1;
    int64_t frameUs = 33333;
    // Keeping up: pictures shown and dropped this window.
    int shown = 0, late = 0;
    int64_t windowAt = 0;
    bool wantLowres = false;

    std::vector<uint16_t> front;
    bool fresh = false;
    Status status;
    bool told = true;
    int64_t lastSecond = -1;

    std::thread demux, video, audio;

    int64_t mediaUs() { return paused ? pausedAt : (nowUs() - base) * speed / 100 - lag; }

    // Under mu: the clock reads `m` now.
    void rebase(int64_t m) { base = nowUs() - (m + lag) * 100 / speed; }

    void poke() { (void)!::write(wake, "p", 1); }

    // Under mu.
    void tell()
    {
        told = false;
        poke();
    }

    // Under mu: past B, back to A.
    void checkRepeat(int64_t us)
    {
        if (repB > repA && repA >= 0 && us / 1000 >= repB && seekTo < 0) {
            seekTo = repA;
            cv.notify_all();
        }
    }

    // Subtitle packets decode as they are read: they are few and small.
    void subtitle(AVCodecContext *dec, AVRational tb, AVPacket *p)
    {
#ifndef GH0ST_PLAYER_EXTRAS
        (void)dec, (void)tb, (void)p;
#else
        AVSubtitle sub;
        int got = 0;
        if (avcodec_decode_subtitle2(dec, &sub, &got, p) < 0 || !got)
            return;
        int64_t start = p->pts == AV_NOPTS_VALUE ? 0 : av_rescale_q(p->pts, tb, AVRational{1, 1000});
        int64_t dur = p->duration > 0 ? av_rescale_q(p->duration, tb, AVRational{1, 1000})
                                       : int64_t(sub.end_display_time) - sub.start_display_time;
        start += sub.start_display_time;
        if (dur <= 0 || dur > 60000)
            dur = 5000;
        std::lock_guard<std::mutex> l(assMu);
        for (unsigned i = 0; i < sub.num_rects; i++) {
            AVSubtitleRect *r = sub.rects[i];
            if (r->type == SUBTITLE_ASS && r->ass && assT) {
                lz_ass_process_chunk(assT, r->ass, int(strlen(r->ass)), start, dur);
            } else if (r->type == SUBTITLE_BITMAP && r->w > 0 && r->h > 0 && r->data[0] &&
                       r->data[1]) {
                SubPic sp{start, start + dur, r->x, r->y, r->w, r->h, {}};
                sp.argb.resize(size_t(r->w) * r->h);
                const uint32_t *pal = reinterpret_cast<const uint32_t *>(r->data[1]);
                for (int y = 0; y < r->h; y++)
                    for (int x = 0; x < r->w; x++)
                        sp.argb[size_t(y) * r->w + x] = pal[r->data[0][size_t(y) * r->linesize[0] + x]];
                // Only the newest few are kept: they replace each other.
                if (subPics.size() > 8)
                    subPics.erase(subPics.begin());
                subPics.push_back(std::move(sp));
            }
        }
        avsubtitle_free(&sub);
#endif
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
                    rebase(ts);
                    if (paused) {
                        pausedAt = ts;
                        showOne = vi >= 0;
                    }
                    eof = false;
                    vDone = vi < 0;
                    aDone = ai < 0;
                    status.ended = false;
                    status.posMs = seekTo;
                    seekTo = -1;
                    cv.notify_all();
                    continue;
                }
            }
            int r = av_read_frame(fmt, p);
            if (r >= 0 && p->stream_index == si && sdec) {
                subtitle(sdec, stb, p);
                av_packet_unref(p);
                continue;
            }
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
            cv.wait_for(l, std::chrono::microseconds(ahead * 100 / speed));
        }
    }

    void drawSubs(std::vector<uint16_t> &px, int64_t ms)
    {
        std::lock_guard<std::mutex> l(assMu);
#ifdef GH0ST_PLAYER_EXTRAS
        if (assR && assT) {
            for (ASS_Image *im = lz_ass_render_frame(assR, assT, ms, nullptr); im; im = im->next) {
                int r = im->color >> 24, g = (im->color >> 16) & 255, b = (im->color >> 8) & 255;
                int op = 255 - int(im->color & 255);
                for (int y = 0; y < im->h; y++) {
                    int dy = im->dst_y + y;
                    if (dy < 0 || dy >= outH)
                        continue;
                    const uint8_t *row = im->bitmap + size_t(y) * im->stride;
                    for (int x = 0; x < im->w; x++) {
                        int dx = im->dst_x + x;
                        int a = row[x] * op / 255;
                        if (dx < 0 || dx >= outW || !a)
                            continue;
                        uint16_t &d = px[size_t(dy) * outW + dx];
                        d = mix565(d, r, g, b, a);
                    }
                }
            }
        }
#endif
        if (subPics.empty() || subW <= 0 || subH <= 0)
            return;
        // A picture subtitle sits on the video's own canvas: scaled with it.
        for (auto &sp : subPics) {
            if (ms < sp.startMs || ms >= sp.endMs)
                continue;
            int x0 = sp.x * outW / subW, y0 = sp.y * outH / subH;
            int w = std::max(1, sp.w * outW / subW), h = std::max(1, sp.h * outH / subH);
            for (int y = 0; y < h; y++) {
                int dy = y0 + y;
                if (dy < 0 || dy >= outH)
                    continue;
                for (int x = 0; x < w; x++) {
                    int dx = x0 + x;
                    if (dx < 0 || dx >= outW)
                        continue;
                    uint32_t c = sp.argb[size_t(y * sp.h / h) * sp.w + x * sp.w / w];
                    int a = c >> 24;
                    if (a)
                        px[size_t(dy) * outW + dx] =
                            mix565(px[size_t(dy) * outW + dx], (c >> 16) & 255, (c >> 8) & 255,
                                   c & 255, a);
                }
            }
        }
    }

    void show(AVFrame *f, int64_t ms)
    {
        std::vector<uint16_t> px(size_t(convW) * convH);
        if (f->format == AV_PIX_FMT_YUV420P || f->format == AV_PIX_FMT_YUVJ420P) {
            // The VPU's pictures sit in uncached memory: swscale reading
            // all of a 1080p one runs at half speed. The preview's path
            // copies only the rows it shows, then converts from cache.
            View v{View::YUV420, f->width, f->height,
                   {f->data[0], f->data[1], f->data[2]},
                   {f->linesize[0], f->linesize[1], f->linesize[2]},
                   f->color_range != AVCOL_RANGE_JPEG && f->format != AV_PIX_FMT_YUVJ420P};
            viewTo565(v, convW, convH, px.data());
        } else {
            sws = sws_getCachedContext(sws, f->width, f->height, AVPixelFormat(f->format), convW,
                                       convH, AV_PIX_FMT_RGB565LE, SWS_BILINEAR, nullptr, nullptr,
                                       nullptr);
            if (!sws)
                return;
            int cs = f->colorspace == AVCOL_SPC_BT470BG || f->colorspace == AVCOL_SPC_SMPTE170M
                         ? SWS_CS_ITU601
                         : SWS_CS_ITU709;
            int full = f->color_range == AVCOL_RANGE_JPEG || f->format == AV_PIX_FMT_YUVJ420P;
            const int *coef = sws_getCoefficients(cs);
            sws_setColorspaceDetails(sws, coef, full, coef, 1, 0, 1 << 16, 1 << 16);
            uint8_t *dst[4] = {reinterpret_cast<uint8_t *>(px.data()), nullptr, nullptr, nullptr};
            int stride[4] = {convW * 2, 0, 0, 0};
            sws_scale(sws, f->data, f->linesize, 0, f->height, dst, stride);
        }
        if (opt.orient > 1) {
            Pic p{convW, convH, std::move(px)};
            px = orient(p, opt.orient).px;
        }
        drawSubs(px, ms);
        put(px);
    }

    void put(std::vector<uint16_t> &px)
    {
        std::lock_guard<std::mutex> l(mu);
        front.swap(px);
        fresh = true;
        poke();
    }

    // Under mu: past a quarter dropped this window, a step slower.
    void judge()
    {
        int64_t now = nowUs();
        if (!windowAt)
            windowAt = now;
        if (now - windowAt < SLOW_WINDOW_US)
            return;
        bool behind = late * 4 > shown + late && late > 2;
        windowAt = now;
        shown = late = 0;
        if (!behind || status.slow >= SLOWEST || hw)
            return;
        status.slow++;
        switch (status.slow) {
        case 1:
            vdec->skip_loop_filter = AVDISCARD_NONREF;
            break;
        case 2:
            vdec->skip_frame = AVDISCARD_NONREF;
            break;
        case 3:
            if (vdec->codec->max_lowres > 0)
                wantLowres = true;
            else
                vdec->skip_idct = AVDISCARD_NONREF;
            break;
        default:
            vdec->skip_frame = AVDISCARD_NONKEY;
            break;
        }
        fprintf(stderr, "%s: behind, skipping more (step %d)\n", program_invocation_short_name,
                status.slow);
        tell();
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
            if (!paused && mediaUs() - us > LATE_US) {
                late++;
                status.dropped++;
                judge();
                return;
            }
            shown++;
            judge();
        }
        show(f, us / 1000);
        std::lock_guard<std::mutex> l(mu);
        status.posMs = us / 1000;
        checkRepeat(us);
        if (us / 1000000 != lastSecond) {
            lastSecond = us / 1000000;
            tell();
        }
    }

    // A seek: the decoder starts afresh. The VPU's decoder is opened again,
    // as not every build of it can flush.
    void resetVideo()
    {
        if (!hw && !wantLowres) {
            avcodec_flush_buffers(vdec);
            return;
        }
        std::string why;
        avcodec_free_context(&vdec);
        if (!openDecoder(vi, vdec, why, !wantLowres, wantLowres))
            giveUp(why);
    }

    // No picture for a while: the VPU once more in software, then an error.
    void stalled()
    {
        std::string why = "can't decode this video";
        if (hw) {
            fprintf(stderr, "%s: the hardware decoder gave nothing, trying software\n",
                    program_invocation_short_name);
            avcodec_free_context(&vdec);
            if (openDecoder(vi, vdec, why, false, false)) {
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
            bool lowres = false;
            {
                std::lock_guard<std::mutex> l(mu);
                lowres = wantLowres && !seeked;
                if (lowres) {
                    // Opened again at a lower resolution, from the last key
                    // frame before here.
                    seekTo = status.posMs;
                    cv.notify_all();
                }
            }
            if (lowres) {
                av_packet_free(&k.p);
                continue;
            }
            if (seeked) {
                resetVideo();
                wantLowres = false;
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

    // The tempo filter for `sp`, between the decoder and the server.
    bool tempo(int sp, int ch)
    {
#ifndef GH0ST_PLAYER_EXTRAS
        (void)ch;
        graphSpeed = sp;
        return sp == 100;
#else
        if (graph)
            lz_avfilter_graph_free(&graph);
        asrc = asink = nullptr;
        graphSpeed = sp;
        if (sp == 100)
            return true;
        if (!gh0st::lazy::open(gh0st::lazy::AVFILTER))
            return false;
        char layout[64] = "mono";
        av_channel_layout_describe(&adec->ch_layout, layout, sizeof layout);
        char desc[512];
        snprintf(desc, sizeof desc,
                 "abuffer=time_base=1/%d:sample_rate=%d:sample_fmt=%s:channel_layout=%s,"
                 "atempo=%.3f,aformat=sample_fmts=s16:channel_layouts=%s:sample_rates=%d,"
                 "abuffersink",
                 adec->sample_rate, adec->sample_rate, av_get_sample_fmt_name(adec->sample_fmt),
                 layout, sp / 100.0, ch > 1 ? "stereo" : "mono", adec->sample_rate);
        graph = lz_avfilter_graph_alloc();
        if (!graph || lz_avfilter_graph_parse_ptr(graph, desc, nullptr, nullptr, nullptr) < 0 ||
            lz_avfilter_graph_config(graph, nullptr) < 0) {
            lz_avfilter_graph_free(&graph);
            return false;
        }
        for (unsigned i = 0; i < graph->nb_filters; i++) {
            AVFilterContext *c = graph->filters[i];
            if (!strcmp(c->filter->name, "abuffer"))
                asrc = c;
            else if (!strcmp(c->filter->name, "abuffersink"))
                asink = c;
        }
        return asrc && asink;
#endif
    }

    void audioLoop()
    {
        AVFrame *f = av_frame_alloc(), *g = av_frame_alloc();
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
                // A new tempo filter: the old one holds sound from before.
                graphSpeed = 0;
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
                int sp;
                {
                    std::lock_guard<std::mutex> l(mu);
                    drop = us < skipUntil || serial != mine || quit;
                    // Far behind the picture (a slow card): catch up.
                    drop = drop || mediaUs() - us > 200000;
                    wait = us - mediaUs() > 200000;
                    sp = speed;
                }
                if (wait && !waitFor(us - 100000, mine))
                    drop = true;
                std::string why;
                if (!drop && !out.isOpen() && !out.open(adec->sample_rate, ch, why))
                    drop = true;
                if (!drop && sp != graphSpeed && !tempo(sp, ch))
                    drop = true;
#ifdef GH0ST_PLAYER_EXTRAS
                if (!drop && graph) {
                    if (lz_av_buffersrc_add_frame_flags(asrc, f, AV_BUFFERSRC_FLAG_KEEP_REF) >= 0)
                        while (lz_av_buffersink_get_frame(asink, g) >= 0) {
                            out.write(reinterpret_cast<const int16_t *>(g->data[0]), g->nb_samples);
                            av_frame_unref(g);
                        }
                    av_frame_unref(f);
                    continue;
                }
#endif
                if (!drop) {
                    pcm.resize(size_t(f->nb_samples) * ch + 64);
                    uint8_t *dst = reinterpret_cast<uint8_t *>(pcm.data());
                    int n = swr_convert(swr, &dst, f->nb_samples, f->extended_data, f->nb_samples);
                    if (n > 0)
                        out.write(pcm.data(), n);
                }
                if (!drop && vi < 0) {
                    // Sound alone keeps the time.
                    std::lock_guard<std::mutex> l(mu);
                    status.posMs = us / 1000;
                    checkRepeat(us);
                    if (us / 1000000 != lastSecond) {
                        lastSecond = us / 1000000;
                        tell();
                    }
                }
                av_frame_unref(f);
            }
        }
#ifdef GH0ST_PLAYER_EXTRAS
        if (graph)
            lz_avfilter_graph_free(&graph);
#endif
        av_frame_free(&f);
        av_frame_free(&g);
    }

    // Under mu: both streams played out.
    void end_()
    {
        if (!vDone || !aDone || status.ended)
            return;
        if (opt.loop && seekTo < 0) {
            seekTo = 0;
            cv.notify_all();
            return;
        }
        status.ended = true;
        paused = true;
        pausedAt = status.posMs * 1000;
        tell();
    }

    bool openDecoder(int idx, AVCodecContext *&ctx, std::string &why, bool allowHw, bool lowres)
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
                // Each frame thread holds pictures of its own: a small video
                // keeps up with fewer, and costs less memory.
                int64_t px = int64_t(st->codecpar->width) * st->codecpar->height;
                ctx->thread_count = px <= 640 * 480 ? 2 : px <= 1280 * 720 ? 3 : 4;
                ctx->thread_type = FF_THREAD_FRAME | FF_THREAD_SLICE;
                if (lowres)
                    ctx->lowres = std::min(1, int(codec->max_lowres));
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

    // The n-th stream of a type, cover pictures left out.
    int nth(AVMediaType type, int n)
    {
        for (unsigned i = 0; i < fmt->nb_streams; i++) {
            AVStream *st = fmt->streams[i];
            if (st->codecpar->codec_type != type || (st->disposition & AV_DISPOSITION_ATTACHED_PIC))
                continue;
            if (n-- == 0)
                return int(i);
        }
        return -1;
    }

    void openSubs(int w, int h)
    {
#ifndef GH0ST_PLAYER_EXTRAS
        (void)w, (void)h;
#else
        int embedded = 0;
        for (unsigned i = 0; i < fmt->nb_streams; i++)
            embedded += fmt->streams[i]->codecpar->codec_type == AVMEDIA_TYPE_SUBTITLE;
        if (opt.subs < 0 || !gh0st::lazy::open(gh0st::lazy::ASS))
            return;
        assLib = lz_ass_library_init();
        if (!assLib)
            return;
        assR = lz_ass_renderer_init(assLib);
        assT = lz_ass_new_track(assLib);
        if (!assR || !assT)
            return;
        lz_ass_set_frame_size(assR, w, h);
        lz_ass_set_font_scale(assR, SUB_SCALE);
        lz_ass_set_fonts(assR, nullptr, "sans-serif", ASS_FONTPROVIDER_AUTODETECT, nullptr, 1);
        subW = vdec ? vdec->width : w;
        subH = vdec ? vdec->height : h;
        if (opt.subs < embedded) {
            si = nth(AVMEDIA_TYPE_SUBTITLE, opt.subs);
            AVStream *st = fmt->streams[si];
            stb = st->time_base;
            const AVCodec *c = avcodec_find_decoder(st->codecpar->codec_id);
            if (!c || !(sdec = avcodec_alloc_context3(c)))
                return;
            avcodec_parameters_to_context(sdec, st->codecpar);
            sdec->pkt_timebase = st->time_base;
            if (avcodec_open2(sdec, c, nullptr) < 0) {
                avcodec_free_context(&sdec);
                si = -1;
                return;
            }
            if (sdec->subtitle_header)
                lz_ass_process_codec_private(assT, reinterpret_cast<char *>(sdec->subtitle_header),
                                          sdec->subtitle_header_size);
            if (sdec->width > 0 && sdec->height > 0) {
                subW = sdec->width;
                subH = sdec->height;
            }
            return;
        }
        // A .srt beside the file: read whole now, it is small.
        std::string srt = sidecarSrt(path);
        AVFormatContext *sf = nullptr;
        if (srt.empty() || avformat_open_input(&sf, srt.c_str(), nullptr, nullptr) < 0)
            return;
        if (avformat_find_stream_info(sf, nullptr) >= 0 && sf->nb_streams > 0) {
            AVStream *st = sf->streams[0];
            const AVCodec *c = avcodec_find_decoder(st->codecpar->codec_id);
            AVCodecContext *d = c ? avcodec_alloc_context3(c) : nullptr;
            if (d && avcodec_parameters_to_context(d, st->codecpar) >= 0 &&
                avcodec_open2(d, c, nullptr) >= 0) {
                if (d->subtitle_header)
                    lz_ass_process_codec_private(assT, reinterpret_cast<char *>(d->subtitle_header),
                                              d->subtitle_header_size);
                AVPacket *p = av_packet_alloc();
                while (av_read_frame(sf, p) >= 0) {
                    subtitle(d, st->time_base, p);
                    av_packet_unref(p);
                }
                av_packet_free(&p);
            }
            avcodec_free_context(&d);
        }
        avformat_close_input(&sf);
#endif
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
        avcodec_free_context(&sdec);
        sws_freeContext(sws);
        sws = nullptr;
        swr_free(&swr);
        avformat_close_input(&fmt);
#ifdef GH0ST_PLAYER_EXTRAS
        if (graph)
            lz_avfilter_graph_free(&graph);
        if (assT)
            lz_ass_free_track(assT);
        if (assR)
            lz_ass_renderer_done(assR);
        if (assLib)
            lz_ass_library_done(assLib);
#endif
        assT = nullptr;
        assR = nullptr;
        assLib = nullptr;
    }
};

Player::Player(int wakeFd) : wake_(wakeFd) {}

Player::~Player()
{
    close();
}

bool Player::open(const std::string &path, int w, int h, std::string &why)
{
    return open(path, w, h, Options(), why);
}

bool Player::open(const std::string &path, int w, int h, const Options &o, std::string &why)
{
    close();
    s_ = std::make_unique<State>();
    State &s = *s_;
    s.wake = wake_;
    s.opt = o;
    s.path = path;
    if (avformat_open_input(&s.fmt, path.c_str(), nullptr, nullptr) < 0 ||
        avformat_find_stream_info(s.fmt, nullptr) < 0) {
        why = "can't read this file";
        close();
        return false;
    }
    s.vi = av_find_best_stream(s.fmt, AVMEDIA_TYPE_VIDEO, -1, -1, nullptr, 0);
    if (s.vi >= 0 && (s.fmt->streams[s.vi]->disposition & AV_DISPOSITION_ATTACHED_PIC))
        s.vi = -1;
    s.ai = o.silent ? -1
           : o.audio >= 0 ? s.nth(AVMEDIA_TYPE_AUDIO, o.audio)
                          : av_find_best_stream(s.fmt, AVMEDIA_TYPE_AUDIO, -1, s.vi, nullptr, 0);
    if (s.vi < 0 && s.ai < 0) {
        why = "nothing to play in this file";
        close();
        return false;
    }
    int vw = 0, vh = 0;
    if (s.vi >= 0) {
        if (!s.openDecoder(s.vi, s.vdec, why, true, false)) {
            close();
            return false;
        }
        AVStream *st = s.fmt->streams[s.vi];
        s.vtb = st->time_base;
        AVRational fr = av_guess_frame_rate(s.fmt, st, nullptr);
        if (fr.num > 0 && fr.den > 0)
            s.frameUs = av_rescale(1000000, fr.den, fr.num);
        // Fitted, with even sides, as large as the screen allows, after
        // the picture is turned.
        vw = s.vdec->width;
        vh = s.vdec->height;
        if (vw <= 0 || vh <= 0) {
            why = "can't read this video";
            close();
            return false;
        }
        bool swap = swapsSides(o.orient);
        int dw = swap ? vh : vw, dh = swap ? vw : vh;
        if (int64_t(dw) * h > int64_t(dh) * w) {
            s.outW = w;
            s.outH = std::max(2, int(int64_t(dh) * w / dw) & ~1);
        } else {
            s.outH = h;
            s.outW = std::max(2, int(int64_t(dw) * h / dh) & ~1);
        }
        s.convW = swap ? s.outH : s.outW;
        s.convH = swap ? s.outW : s.outH;
    }
    std::string none;
    if (s.ai >= 0 && s.openDecoder(s.ai, s.adec, none, false, false)) {
        s.atb = s.fmt->streams[s.ai]->time_base;
        int ch = s.adec->ch_layout.nb_channels > 1 ? 2 : 1;
        AVChannelLayout layout;
        av_channel_layout_default(&layout, ch);
        if (swr_alloc_set_opts2(&s.swr, &layout, AV_SAMPLE_FMT_S16, s.adec->sample_rate,
                                &s.adec->ch_layout, s.adec->sample_fmt, s.adec->sample_rate, 0,
                                nullptr) == 0 &&
            swr_init(s.swr) == 0 && (o.paused || s.out.open(s.adec->sample_rate, ch, none))) {
            s.sound = true;
        } else {
            // No sound server: the picture plays on, silent.
            fprintf(stderr, "%s: %s: playing without sound: %s\n", program_invocation_short_name,
                    path.c_str(), none.c_str());
            avcodec_free_context(&s.adec);
            swr_free(&s.swr);
            s.ai = -1;
        }
    } else {
        s.ai = -1;
    }
    if (s.vi < 0 && s.ai < 0) {
        why = "can't play this sound";
        close();
        return false;
    }
    if (s.vi >= 0)
        s.openSubs(s.outW, s.outH);
    if (s.fmt->duration > 0)
        s.status.durMs = s.fmt->duration / (AV_TIME_BASE / 1000);
    s.vDone = s.vi < 0;
    s.aDone = s.ai < 0;
    s.lag = s.sound ? 100000 : 0;
    s.rebase(0);
    if (o.paused) {
        s.paused = true;
        s.pausedAt = o.startMs * 1000;
        s.showOne = s.vi >= 0;
        s.status.paused = true;
    }
    if (o.startMs > 0)
        s.seekTo = o.startMs;
    s.demux = std::thread([&s] { s.readLoop(); });
    if (s.vi >= 0)
        s.video = std::thread([&s] { s.videoLoop(); });
    if (s.ai >= 0)
        s.audio = std::thread([&s] { s.audioLoop(); });
    fprintf(stderr, "%s: playing %s, %dx%d to %dx%d, %s decoder, %s\n",
            program_invocation_short_name, path.c_str(), vw, vh, s.outW, s.outH,
            s.vi < 0 ? "no picture," : s.hw ? "hardware" : "software",
            s.sound ? "with sound" : "silent");
    return true;
}

bool Player::active() const
{
    return s_ != nullptr;
}

bool Player::hasPicture() const
{
    return s_ && s_->vi >= 0;
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

void Player::speed(int percent)
{
#ifndef GH0ST_PLAYER_EXTRAS
    percent = 100;
#endif
    if (!s_)
        return;
    State &s = *s_;
    std::lock_guard<std::mutex> l(s.mu);
    percent = std::clamp(percent, 50, 200);
    int64_t m = s.mediaUs();
    s.speed = percent;
    if (!s.paused)
        s.rebase(m);
    s.cv.notify_all();
}

void Player::repeat(int64_t a, int64_t b)
{
    if (!s_)
        return;
    std::lock_guard<std::mutex> l(s_->mu);
    s_->repA = b > a ? a : -1;
    s_->repB = b > a ? b : -1;
}

void Player::step()
{
    if (!s_)
        return;
    int64_t at;
    {
        std::lock_guard<std::mutex> l(s_->mu);
        if (!s_->paused || s_->vi < 0)
            return;
        at = s_->status.posMs + std::max<int64_t>(1, s_->frameUs / 1000);
    }
    seek(at);
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

int64_t Player::position() const
{
    if (!s_)
        return 0;
    std::lock_guard<std::mutex> l(s_->mu);
    return s_->status.posMs;
}

} // namespace gh0st
