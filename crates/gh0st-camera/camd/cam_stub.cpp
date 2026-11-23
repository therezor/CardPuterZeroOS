// A sensor for hosts without libcamera: a moving test pattern at 25 fps
// from a thread, as libcamera's own thread delivers frames. It proves the
// wire, the ring and the files against the Rust app (`make stub`).
// GH0ST_CAMD_STUB=nocamera answers as a Lite does.
#include "camd.h"

#include <atomic>
#include <chrono>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <thread>
#include <unistd.h>

namespace camd {

namespace {

class Stub : public Backend {
public:
    explicit Stub(int wake) : wake_(wake) {}
    ~Stub() override { close(); }

    Fault open() override
    {
        const char *e = getenv("GH0ST_CAMD_STUB");
        return e && !strcmp(e, "nocamera") ? Fault::NoCamera : Fault::None;
    }
    std::string sensor() const override { return "stub"; }
    int fullW() const override { return 3280; }
    int fullH() const override { return 2464; }

    Fault preview(Mode, int w, int h, const Controls &) override
    {
        stop();
        w_ = w;
        h_ = h;
        px_.assign(size_t(w) * h, 0);
        run_ = true;
        thread_ = std::thread([this] { loop(); });
        return Fault::None;
    }

    void set(const Controls &) override {}

    Fault still(const Controls &c, Yuv420 &out, ShotMeta &meta, std::string &) override
    {
        stop();
        out.resize(1092, 820); // not a whole MCU wide or tall: the padding runs
        for (int y = 0; y < out.h; y++)
            for (int x = 0; x < out.w; x++)
                out.y[size_t(y) * out.w + x] = uint8_t((x + y) / 5);
        meta.exposure_us = 10000;
        meta.gain = 1.0f;
        meta.zoom = c.zoom;
        meta.ev = c.ev;
        return Fault::None;
    }

    void stop() override
    {
        run_ = false;
        if (thread_.joinable())
            thread_.join();
    }

    void close() override { stop(); }

    void frames(const std::function<void(const View &)> &sink,
                const std::function<bool(const VideoFrame &)> &) override
    {
        std::lock_guard<std::mutex> l(mu_);
        if (!fresh_)
            return;
        fresh_ = false;
        View v{View::RGB565, w_, h_, {reinterpret_cast<const uint8_t *>(px_.data())}, {w_ * 2}};
        sink(v);
    }

private:
    void loop()
    {
        for (int n = 0; run_; n++) {
            {
                std::lock_guard<std::mutex> l(mu_);
                for (int y = 0; y < h_; y++)
                    for (int x = 0; x < w_; x++)
                        px_[size_t(y) * w_ + x] = uint16_t(((x + n) & 31) << 11 | (y & 63) << 5);
                fresh_ = true;
            }
            (void)!::write(wake_, "f", 1);
            std::this_thread::sleep_for(std::chrono::milliseconds(40));
        }
    }

    int wake_;
    int w_ = 0, h_ = 0;
    std::vector<uint16_t> px_;
    bool fresh_ = false;
    std::atomic<bool> run_{false};
    std::mutex mu_;
    std::thread thread_;
};

} // namespace

std::unique_ptr<Backend> makeBackend(int wakeFd)
{
    return std::make_unique<Stub>(wakeFd);
}

int probe(const char *)
{
    fprintf(stderr, "gh0st-camd: --probe needs the libcamera build\n");
    return 2;
}

} // namespace camd
