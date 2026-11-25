#include "anim.h"
#include "pix.h"
#include "stills.h"

#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <mutex>
#include <thread>
#include <unistd.h>

#include <webp/demux.h>

namespace mediad {

namespace {

// A frame shorter than this shows for SHORT_FRAME_MS, as FFmpeg does for
// a GIF. Frames of 0 ms would make the thread spin.
const int MIN_FRAME_MS = 20, SHORT_FRAME_MS = 100;

} // namespace

struct Anim::State {
    int wake;
    std::vector<uint8_t> file;
    WebPAnimDecoder *dec = nullptr;
    WebPAnimInfo info{};
    int outW = 0, outH = 0;

    std::mutex mu;
    std::condition_variable cv;
    bool quit = false, paused = false, restart = false;
    std::vector<uint16_t> front;
    bool fresh = false;
    gh0st::Player::Status status;
    bool told = true;
    std::thread thread;

    void poke() { (void)!::write(wake, "p", 1); }

    // The canvas, RGBA, fitted into outW x outH by a box filter.
    std::vector<uint16_t> fit(const uint8_t *rgba)
    {
        const int W = int(info.canvas_width), H = int(info.canvas_height);
        std::vector<uint16_t> px(size_t(outW) * outH);
        for (int y = 0; y < outH; y++) {
            int y0 = y * H / outH, y1 = std::max(y0 + 1, (y + 1) * H / outH);
            for (int x = 0; x < outW; x++) {
                int x0 = x * W / outW, x1 = std::max(x0 + 1, (x + 1) * W / outW);
                uint32_t r = 0, g = 0, b = 0, n = 0;
                for (int yy = y0; yy < y1; yy++)
                    for (int xx = x0; xx < x1; xx++) {
                        const uint8_t *q = rgba + (size_t(yy) * W + xx) * 4;
                        r += q[0] * q[3] / 255;
                        g += q[1] * q[3] / 255;
                        b += q[2] * q[3] / 255;
                        n++;
                    }
                px[size_t(y) * outW + x] = gh0st::pack565(int(r / n), int(g / n), int(b / n));
            }
        }
        return px;
    }

    void run()
    {
        using clock = std::chrono::steady_clock;
        auto start = clock::now();
        // Where the last frame ends: on the clock from start, and in the
        // file's own time.
        int lastEnd = 0, lastTs = 0;
        for (;;) {
            {
                std::unique_lock<std::mutex> l(mu);
                cv.wait(l, [&] { return quit || !paused || restart; });
                if (quit)
                    return;
                if (restart) {
                    WebPAnimDecoderReset(dec);
                    start = clock::now();
                    lastEnd = lastTs = 0;
                    restart = false;
                } else if (!WebPAnimDecoderHasMoreFrames(dec)) {
                    // Round again, as a GIF does, once the last frame has
                    // had its time.
                    WebPAnimDecoderReset(dec);
                    start += std::chrono::milliseconds(lastEnd);
                    lastEnd = lastTs = 0;
                }
            }
            uint8_t *buf = nullptr;
            int ts = 0;
            if (!WebPAnimDecoderGetNext(dec, &buf, &ts)) {
                std::lock_guard<std::mutex> l(mu);
                status.error = "can't decode this picture";
                told = false;
                poke();
                return;
            }
            std::vector<uint16_t> px = fit(buf);
            // A frame shows from where the last one ended. Late, after a
            // stall or a slow decode, the clock moves on instead: the frames
            // owed would show back to back.
            auto at = start + std::chrono::milliseconds(lastEnd);
            auto now = clock::now();
            if (at < now) {
                start += now - at;
                at = now;
            }
            lastEnd += ts - lastTs < MIN_FRAME_MS ? SHORT_FRAME_MS : ts - lastTs;
            lastTs = ts;
            std::unique_lock<std::mutex> l(mu);
            cv.wait_until(l, at, [&] { return quit || restart; });
            if (quit)
                return;
            front.swap(px);
            fresh = true;
            status.posMs = ts;
            poke();
            if (paused) {
                // Paused: hold this frame, and the clock with it.
                auto held = clock::now();
                cv.wait(l, [&] { return quit || !paused || restart; });
                start += clock::now() - held;
            }
        }
    }
};

Anim::Anim(int wakeFd) : wake_(wakeFd) {}

Anim::~Anim()
{
    close();
}

bool Anim::open(const std::string &path, int w, int h, bool paused, std::string &why)
{
    close();
    auto s = std::make_unique<State>();
    s->wake = wake_;
    FILE *f = fopen(path.c_str(), "rb");
    if (!f) {
        why = "can't read this file";
        return false;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    rewind(f);
    if (n > MAX_ANIM_BYTES) {
        fclose(f);
        why = "too big to play here";
        return false;
    }
    if (n > 0) {
        s->file.resize(size_t(n));
        s->file.resize(fread(s->file.data(), 1, s->file.size(), f));
    }
    fclose(f);
    WebPData data = {s->file.data(), s->file.size()};
    WebPAnimDecoderOptions o;
    WebPAnimDecoderOptionsInit(&o);
    o.color_mode = MODE_RGBA;
    s->dec = WebPAnimDecoderNew(&data, &o);
    if (!s->dec || !WebPAnimDecoderGetInfo(s->dec, &s->info) || !s->info.canvas_width ||
        !s->info.canvas_height) {
        if (s->dec)
            WebPAnimDecoderDelete(s->dec);
        why = "can't read this picture";
        return false;
    }
    const int W = int(s->info.canvas_width), H = int(s->info.canvas_height);
    double k = std::min({1.0, double(w) / W, double(h) / H});
    s->outW = std::max(1, int(W * k));
    s->outH = std::max(1, int(H * k));
    s->paused = paused;
    s->status.paused = paused;
    State *raw = s.get();
    s->thread = std::thread([raw] { raw->run(); });
    s_ = std::move(s);
    return true;
}

bool Anim::active() const
{
    return s_ != nullptr;
}

void Anim::pause(bool on)
{
    if (!s_)
        return;
    std::lock_guard<std::mutex> l(s_->mu);
    s_->paused = on;
    s_->status.paused = on;
    s_->told = false;
    s_->poke();
    s_->cv.notify_all();
}

void Anim::restart()
{
    if (!s_)
        return;
    std::lock_guard<std::mutex> l(s_->mu);
    s_->restart = true;
    s_->cv.notify_all();
}

void Anim::close()
{
    if (!s_)
        return;
    {
        std::lock_guard<std::mutex> l(s_->mu);
        s_->quit = true;
        s_->cv.notify_all();
    }
    if (s_->thread.joinable())
        s_->thread.join();
    WebPAnimDecoderDelete(s_->dec);
    s_.reset();
}

bool Anim::picture(std::vector<uint16_t> &px, int &w, int &h)
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

bool Anim::news(gh0st::Player::Status &out)
{
    if (!s_)
        return false;
    std::lock_guard<std::mutex> l(s_->mu);
    if (s_->told)
        return false;
    s_->told = true;
    out = s_->status;
    return true;
}

} // namespace mediad
