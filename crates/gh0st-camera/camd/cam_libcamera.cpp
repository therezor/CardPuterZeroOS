// The sensor through libcamera, and `--probe`. Requests complete on
// libcamera's thread; onDone only queues them and writes the wake fd, and
// everything else runs on the loop's thread.
#include "camd.h"
#include "rec.h"
#include "still.h"

#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <cstdio>
#include <cstring>
#include <deque>
#include <fcntl.h>
#include <fstream>
#include <map>
#include <mutex>
#include <optional>
#include <set>
#include <poll.h>
#include <sys/mman.h>
#include <unistd.h>

#include <libcamera/libcamera.h>

using namespace libcamera;

namespace camd {

namespace {

// The preview runs at 25 fps, every second panel refresh, and slows to 15
// in the dark rather than go noisy.
const int64_t PREVIEW_MIN_US = 40000, PREVIEW_MAX_US = 66666;
// The CardputerZero's IMX219 runs 1.2x slower than libcamera's model of it
// (measured: frames 40 ms apart when 33.3 ms is reported). Every time we
// ask for is divided by this and every time we read is multiplied, so a
// frame, an exposure and the flicker period are what they say.
// GH0ST_CAMD_CLOCK overrides it, for another board.
double sensorClock()
{
    static double c = [] {
        const char *e = getenv("GH0ST_CAMD_CLOCK");
        double v = e ? atof(e) : 0;
        return v > 0.5 && v < 2 ? v : 1.2;
    }();
    return c;
}

int64_t toCamera(int64_t realUs)
{
    return int64_t(realUs / sensorClock());
}

int64_t fromCamera(int64_t camUs)
{
    return int64_t(camUs * sensorClock());
}
// Video runs at a steady 30 fps.
const int VIDEO_FPS = 30;
const int64_t VIDEO_FRAME_US = 1000000 / VIDEO_FPS;
// Buffers in video mode: frames wait in the encoder while the preview runs.
const unsigned VIDEO_BUFS = 6;
// A photo may expose longer than the preview: up to 1/8 s, still sharp in
// a steady hand, and much less gain in a dim room.
const int64_t STILL_MIN_US = 33333, STILL_MAX_US = 125000;
const auto STILL_WAIT = std::chrono::seconds(5);
// Frames a photo may wait for exposure to settle at the longer limit.
const int STILL_SETTLE_FRAMES = 8;
const uint64_t STILL_SETTLE_MS = 1000;

Rectangle cropFor(const Rectangle &max, double aspect, float zoom)
{
    double w = max.width, h = max.height;
    if (w / h > aspect)
        w = h * aspect;
    else
        h = w / aspect;
    unsigned cw = unsigned(w / zoom) & ~1u, ch = unsigned(h / zoom) & ~1u;
    return Rectangle((max.x + int((max.width - cw) / 2)) & ~1,
                     (max.y + int((max.height - ch) / 2)) & ~1, cw, ch);
}

Fault faultOf(int err)
{
    return err == -ENOMEM ? Fault::NoMemory : err == -EBUSY ? Fault::Busy : Fault::Other;
}

long cmaFreeKb()
{
    std::ifstream f("/proc/meminfo");
    std::string k;
    long v;
    std::string unit;
    while (f >> k >> v >> unit)
        if (k == "CmaFree:")
            return v;
    return -1;
}

struct Lc : Backend {
    explicit Lc(int wake) : wake_(wake) {}
    ~Lc() override
    {
        close();
        if (cm_)
            cm_->stop();
    }

    Fault open() override
    {
        if (!cm_) {
            usePipelineConfig();
            cm_ = std::make_unique<CameraManager>();
            if (cm_->start() < 0) {
                cm_.reset();
                return Fault::Other;
            }
        }
        if (cam_)
            return Fault::None;
        auto cams = cm_->cameras();
        if (cams.empty())
            return Fault::NoCamera;
        cam_ = cams[0];
        if (cam_->acquire() < 0) {
            cam_.reset();
            return Fault::Busy;
        }
        cam_->requestCompleted.connect(this, &Lc::onDone);
        model_ = cam_->properties().get(properties::Model).value_or("camera");
        full_ = cam_->properties().get(properties::PixelArraySize).value_or(Size(0, 0));
        findModes();
        return Fault::None;
    }

    // Our buffer counts (vc4.yaml), from beside the binary or the package,
    // unless the caller chose a file.
    static void usePipelineConfig()
    {
        if (getenv("LIBCAMERA_RPI_CONFIG_FILE"))
            return;
        char exe[4096];
        ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 1);
        std::string here = n > 0 ? std::string(exe, n) : "";
        here = here.substr(0, here.find_last_of('/') + 1) + "vc4.yaml";
        for (const std::string &p : {here, std::string("/usr/share/gh0st-camera/vc4.yaml")})
            if (access(p.c_str(), R_OK) == 0) {
                setenv("LIBCAMERA_RPI_CONFIG_FILE", p.c_str(), 1);
                return;
            }
    }

    std::string sensor() const override { return model_; }
    int fullW() const override { return int(full_.width); }
    int fullH() const override { return int(full_.height); }

    Fault preview(Mode mode, int w, int h, const Controls &c) override
    {
        ctl_ = c;
        aspect_ = mode == Mode::Photo ? 4.0 / 3 : 16.0 / 9;
        Fault f;
        int64_t lo = PREVIEW_MIN_US, hi = PREVIEW_MAX_US;
        if (mode == Mode::Photo) {
            f = configure({{StreamRole::Viewfinder, formats::RGB565, Size(w, h), 3, std::nullopt}},
                          binned_);
        } else {
            // Video keeps both streams running, so a recording starts on
            // the next frame with the view unchanged: the full size for the
            // encoder, and the preview from the ISP's second output.
            Size size = mode == Mode::Video1080 ? Size(1920, 1080) : Size(1280, 720);
            f = configure({{StreamRole::VideoRecording, formats::YUV420, size, VIDEO_BUFS,
                            ColorSpace::Rec709},
                           {StreamRole::Viewfinder, formats::RGB565, Size(w, h), VIDEO_BUFS,
                            std::nullopt}},
                          mode == Mode::Video1080 ? hd_ : binned_);
            lo = hi = VIDEO_FRAME_US;
        }
        if (f != Fault::None)
            return f;
        ControlList cl = controlsFor(c, aspect_);
        if (cam_->controls().count(&controls::FrameDurationLimits))
            cl.set(controls::FrameDurationLimits,
                   Span<const int64_t, 2>({toCamera(lo), toCamera(hi)}));
        return startStream(cl);
    }

    uint32_t lostFrames() const override { return gaps_; }

    bool videoStream(int &w, int &h, int &stride) const override
    {
        if (vid_ < 0 || !cfg_)
            return false;
        const StreamConfiguration &sc = cfg_->at(vid_);
        w = int(sc.size.width);
        h = int(sc.size.height);
        stride = int(sc.stride);
        return true;
    }

    void release(void *token) override
    {
        auto *r = static_cast<Request *>(token);
        if (held_.erase(r))
            requeue(r);
    }

    void set(const Controls &c) override
    {
        ctl_ = c;
        dirty_ = true;
    }

    Fault still(const Controls &c, Yuv420 &out, ShotMeta &meta, std::string &why) override
    {
        ctl_ = c;
        // Zoom crops the sensor: the photo is the crop's own pixels, not
        // scaled up to 8 MP.
        Size size(unsigned(full_.width / c.zoom) & ~1u, unsigned(full_.height / c.zoom) & ~1u);
        Fault f = configure({{StreamRole::StillCapture, formats::YUV420, size, 1, ColorSpace::Sycc}},
                            full_);
        if (f != Fault::None) {
            why = "still config";
            return f;
        }
        ControlList cl = controlsFor(c, 4.0 / 3);
        if (cam_->controls().count(&controls::draft::NoiseReductionMode))
            cl.set(controls::draft::NoiseReductionMode,
                   controls::draft::NoiseReductionModeHighQuality);
        if (cam_->controls().count(&controls::FrameDurationLimits))
            cl.set(controls::FrameDurationLimits,
                   Span<const int64_t, 2>({toCamera(STILL_MIN_US), toCamera(STILL_MAX_US)}));
        f = startStream(cl);
        if (f != Fault::None) {
            why = "still start";
            stopStream();
            return f;
        }
        // The exposure carries over from the preview. In good light the
        // first frame is settled; in the dark, AE now has room to expose
        // longer, so wait while it searches.
        Request *r = nullptr;
        uint64_t t0 = nowMs();
        for (int n = 0;; n++) {
            r = nullptr;
            {
                std::unique_lock<std::mutex> l(mu_);
                if (cv_.wait_for(l, STILL_WAIT, [this] { return !done_.empty(); })) {
                    r = done_.front();
                    done_.clear();
                }
            }
            if (!r || r->status() != Request::RequestComplete)
                break;
            auto ae = r->metadata().get(controls::AeState);
            if (!ae || *ae != controls::AeStateSearching || n + 1 >= STILL_SETTLE_FRAMES ||
                nowMs() - t0 > STILL_SETTLE_MS)
                break;
            r->reuse(Request::ReuseBuffers);
            cam_->queueRequest(r);
        }
        if (!r || r->status() != Request::RequestComplete || !copyStill(r, out)) {
            why = r ? "still failed" : "still timed out";
            stopStream();
            return Fault::Other;
        }
        const ControlList &md = r->metadata();
        meta.exposure_us = fromCamera(md.get(controls::ExposureTime).value_or(0));
        meta.gain = md.get(controls::AnalogueGain).value_or(1.0f) *
                    md.get(controls::DigitalGain).value_or(1.0f);
        meta.model = model_;
        stopStream();
        return Fault::None;
    }

    void stop() override { stopStream(); }

    void close() override
    {
        stopStream();
        if (cam_) {
            cam_->requestCompleted.disconnect(this);
            cam_->release();
            cam_.reset();
        }
    }

    void frames(const std::function<void(const View &)> &sink,
                const std::function<bool(const VideoFrame &)> &video) override
    {
        std::deque<Request *> got;
        {
            std::lock_guard<std::mutex> l(mu_);
            got.swap(done_);
        }
        if (got.empty() || !running_)
            return;
        Request *last = got.back();
        if (last->status() == Request::RequestComplete) {
            const ControlList &md = last->metadata();
            lastFrameUs_ = md.get(controls::FrameDuration).value_or(0);
            lastExposureUs_ = md.get(controls::ExposureTime).value_or(0);
            View v{};
            if (view(last, pv_, v))
                sink(v);
        }
        for (Request *r : got) {
            if (r->status() == Request::RequestComplete) {
                // Gaps in sensor time: frames lost before libcamera.
                int64_t ts = r->metadata().get(controls::SensorTimestamp).value_or(0);
                int64_t fd = r->metadata().get(controls::FrameDuration).value_or(0);
                // Frames lost in a gap: each frame is fd (camera time) apart.
                double period = fd * 1000.0 * sensorClock();
                if (lastTs_ && fd && ts - lastTs_ > period * 1.5)
                    gaps_ += uint32_t(std::lround((ts - lastTs_) / period)) - 1;
                if (lastTs_ && fd) {
                    sumDelta_ += ts - lastTs_;
                    sumDur_ += fd * 1000;
                }
                lastTs_ = ts;
                completed_++;
            }
            VideoFrame vf{};
            if (video && vid_ >= 0 && r->status() == Request::RequestComplete &&
                videoFrame(r, vf) && video(vf)) {
                held_.insert(r);
                continue;
            }
            requeue(r);
        }
    }

    // A request's full-size buffer, as the encoder takes it: one dmabuf,
    // its length, and when the sensor began the frame.
    bool videoFrame(Request *r, VideoFrame &vf)
    {
        FrameBuffer *b = r->findBuffer(streams_[vid_]);
        if (!b || !view(r, vid_, vf.view))
            return false;
        const auto &planes = b->planes();
        vf.fd = planes[0].fd.get();
        size_t end = 0;
        for (const auto &pl : planes)
            if (pl.fd.get() == vf.fd)
                end = std::max(end, size_t(pl.offset) + pl.length);
        vf.size = end;
        auto ts = r->metadata().get(controls::SensorTimestamp);
        vf.tsUs = int64_t(ts ? *ts : b->metadata().timestamp) / 1000;
        vf.token = r;
        return true;
    }

    // libcamera's thread.
    void onDone(Request *r)
    {
        if (r->status() == Request::RequestCancelled)
            return;
        {
            std::lock_guard<std::mutex> l(mu_);
            done_.push_back(r);
        }
        cv_.notify_all();
        (void)!::write(wake_, "f", 1);
    }

    // The sensor's modes, from the raw stream's sizes: the full one for
    // stills, the 2x2 binned one for the preview and 720p, and the native
    // 1080p crop when there is one.
    void findModes()
    {
        modes_.clear();
        auto raw = cam_->generateConfiguration({StreamRole::Raw});
        if (raw && !raw->empty()) {
            const StreamFormats &f = raw->at(0).formats();
            for (const PixelFormat &pf : f.pixelformats())
                for (const Size &s : f.sizes(pf))
                    if (std::find(modes_.begin(), modes_.end(), s) == modes_.end())
                        modes_.push_back(s);
        }
        Size big = full_;
        for (const Size &s : modes_)
            if (uint64_t(s.width) * s.height > uint64_t(big.width) * big.height)
                big = s;
        full_ = big;
        binned_ = full_;
        hd_ = Size();
        for (const Size &s : modes_) {
            if (s.width == full_.width / 2 && s.height == full_.height / 2)
                binned_ = s;
            if (s.width == 1920 && s.height == 1080)
                hd_ = s;
        }
        if (hd_.isNull())
            hd_ = binned_;
    }

    struct Want {
        StreamRole role;
        PixelFormat fmt;
        Size size;
        unsigned bufs;
        std::optional<ColorSpace> cs;
    };

    // Streams as asked, on the sensor mode `sensor`. Each request carries
    // one buffer of each stream. The last stream is the preview; with two,
    // the first is the video.
    Fault configure(const std::vector<Want> &wants, Size sensor)
    {
        stopStream();
        std::vector<StreamRole> roles;
        for (const Want &w : wants)
            roles.push_back(w.role);
        cfg_ = cam_->generateConfiguration(roles);
        if (!cfg_ || cfg_->size() != wants.size())
            return Fault::Other;
        for (size_t i = 0; i < wants.size(); i++) {
            StreamConfiguration &sc = cfg_->at(i);
            sc.pixelFormat = wants[i].fmt;
            sc.size = wants[i].size;
            sc.bufferCount = wants[i].bufs;
            if (wants[i].cs)
                sc.colorSpace = wants[i].cs;
        }
        // The device tree says the sensor is mounted at 180 degrees, so
        // libcamera flips every frame; on the CardputerZero that turns the
        // picture upside down. Asking for 180 cancels the flip.
        cfg_->orientation = Orientation::Rotate180;
        if (!sensor.isNull()) {
            SensorConfiguration s;
            s.bitDepth = 10;
            s.outputSize = sensor;
            cfg_->sensorConfig = s;
        }
        adjusted_ = false;
        auto st = cfg_->validate();
        if (st == CameraConfiguration::Invalid && cfg_->sensorConfig) {
            cfg_->sensorConfig.reset();
            st = cfg_->validate();
        }
        // A preview format we can't draw: ask for planar YUV instead.
        StreamConfiguration &pv = cfg_->at(wants.size() - 1);
        if (st != CameraConfiguration::Invalid && pv.pixelFormat != formats::RGB565 &&
            pv.pixelFormat != formats::YUV420) {
            pv.pixelFormat = formats::YUV420;
            st = cfg_->validate();
        }
        if (st == CameraConfiguration::Invalid)
            return Fault::Other;
        adjusted_ = st == CameraConfiguration::Adjusted;
        if (int e = cam_->configure(cfg_.get()); e < 0)
            return faultOf(e);
        pv_ = int(wants.size()) - 1;
        vid_ = wants.size() > 1 ? 0 : -1;
        alloc_ = std::make_unique<FrameBufferAllocator>(cam_);
        size_t count = SIZE_MAX;
        for (size_t i = 0; i < wants.size(); i++) {
            Stream *st2 = cfg_->at(i).stream();
            streams_.push_back(st2);
            int n = alloc_->allocate(st2);
            if (n <= 0) {
                stopStream();
                return n == 0 ? Fault::NoMemory : faultOf(n);
            }
            count = std::min(count, alloc_->buffers(st2).size());
        }
        for (size_t k = 0; k < count; k++) {
            auto r = cam_->createRequest();
            if (!r) {
                stopStream();
                return Fault::Other;
            }
            for (Stream *st2 : streams_) {
                FrameBuffer *b = alloc_->buffers(st2)[k].get();
                if (!map(b)) {
                    stopStream();
                    return Fault::NoMemory;
                }
                if (r->addBuffer(st2, b) < 0) {
                    stopStream();
                    return Fault::Other;
                }
            }
            reqs_.push_back(std::move(r));
        }
        auto it = cam_->controls().find(&controls::ScalerCrop);
        cropMax_ = it != cam_->controls().end() ? it->second.max().get<Rectangle>()
                                                : Rectangle(full_);
        return Fault::None;
    }

    bool map(const FrameBuffer *b)
    {
        std::array<uint8_t *, 3> p{};
        size_t i = 0;
        for (const FrameBuffer::Plane &pl : b->planes()) {
            int fd = pl.fd.get();
            auto it = maps_.find(fd);
            if (it == maps_.end()) {
                off_t len = lseek(fd, 0, SEEK_END);
                void *m = len > 0 ? mmap(nullptr, len, PROT_READ, MAP_SHARED, fd, 0) : MAP_FAILED;
                if (m == MAP_FAILED)
                    return false;
                it = maps_.emplace(fd, std::make_pair(static_cast<uint8_t *>(m), size_t(len))).first;
            }
            if (i < 3)
                p[i++] = it->second.first + pl.offset;
        }
        planes_[b] = p;
        return true;
    }

    // Where the planes of a request's buffer are. One dmabuf may hold all
    // three planes of YUV420.
    bool view(Request *r, int stream, View &v)
    {
        if (stream < 0 || size_t(stream) >= streams_.size())
            return false;
        const StreamConfiguration &sc = cfg_->at(stream);
        FrameBuffer *b = r->findBuffer(streams_[stream]);
        if (!b)
            return false;
        auto it = planes_.find(b);
        if (it == planes_.end())
            return false;
        auto p = it->second;
        int stride = int(sc.stride);
        v.w = int(sc.size.width);
        v.h = int(sc.size.height);
        if (sc.pixelFormat == formats::RGB565) {
            v.fmt = View::RGB565;
            v.p[0] = p[0];
            v.stride[0] = stride;
            return true;
        }
        v.fmt = View::YUV420;
        v.limited = sc.colorSpace && sc.colorSpace->range == ColorSpace::Range::Limited;
        if (b->planes().size() == 1) {
            p[1] = p[0] + size_t(stride) * v.h;
            p[2] = p[1] + size_t(stride / 2) * (v.h / 2);
        }
        for (int i = 0; i < 3; i++) {
            v.p[i] = p[i];
            v.stride[i] = i ? stride / 2 : stride;
        }
        return true;
    }

    bool copyStill(Request *r, Yuv420 &out)
    {
        View v{};
        if (!view(r, 0, v) || v.fmt != View::YUV420)
            return false;
        out.resize(v.w, v.h);
        for (int y = 0; y < v.h; y++)
            memcpy(&out.y[size_t(y) * v.w], v.p[0] + size_t(y) * v.stride[0], v.w);
        for (int y = 0; y < out.ch(); y++) {
            memcpy(&out.u[size_t(y) * out.cw()], v.p[1] + size_t(y) * v.stride[1], out.cw());
            memcpy(&out.v[size_t(y) * out.cw()], v.p[2] + size_t(y) * v.stride[2], out.cw());
        }
        return true;
    }

    ControlList controlsFor(const Controls &c, double aspect)
    {
        ControlList cl(controls::controls);
        const ControlInfoMap &info = cam_->controls();
        if (info.count(&controls::ScalerCrop) && cropMax_.width)
            cl.set(controls::ScalerCrop, cropFor(cropMax_, aspect, c.zoom));
        if (info.count(&controls::ExposureValue))
            cl.set(controls::ExposureValue, c.ev);
        if (c.flicker && info.count(&controls::AeFlickerMode)) {
            cl.set(controls::AeFlickerMode, controls::FlickerManual);
            // Lights flicker at twice the mains frequency.
            cl.set(controls::AeFlickerPeriod, int32_t(toCamera(c.flicker == 50 ? 10000 : 8333)));
        }
        return cl;
    }

    Fault startStream(ControlList &cl)
    {
        int e = cam_->start(&cl);
        // CMA lends its pages to the page cache and takes them back on
        // demand; a first try can lose that race.
        if (e == -ENOMEM)
            e = cam_->start(&cl);
        if (e < 0)
            return faultOf(e);
        running_ = true;
        for (auto &r : reqs_)
            cam_->queueRequest(r.get());
        return Fault::None;
    }

    void requeue(Request *r)
    {
        if (!running_)
            return;
        r->reuse(Request::ReuseBuffers);
        if (dirty_) {
            r->controls().merge(controlsFor(ctl_, aspect_));
            dirty_ = false;
        }
        cam_->queueRequest(r);
    }

    void stopStream()
    {
        if (running_) {
            cam_->stop();
            running_ = false;
        }
        {
            std::lock_guard<std::mutex> l(mu_);
            done_.clear();
        }
        held_.clear();
        lastTs_ = 0;
        reqs_.clear();
        planes_.clear();
        for (auto &m : maps_)
            munmap(m.second.first, m.second.second);
        maps_.clear();
        if (alloc_) {
            for (Stream *st : streams_)
                alloc_->free(st);
            alloc_.reset();
        }
        cfg_.reset();
        streams_.clear();
        pv_ = 0;
        vid_ = -1;
    }

    int wake_;
    std::unique_ptr<CameraManager> cm_;
    std::shared_ptr<Camera> cam_;
    std::unique_ptr<CameraConfiguration> cfg_;
    std::unique_ptr<FrameBufferAllocator> alloc_;
    std::vector<Stream *> streams_;
    // Which stream the preview and the video are; -1 for none.
    int pv_ = 0, vid_ = -1;
    // Requests the encoder holds; they go back on release().
    std::set<Request *> held_;
    // The newest frame's timing, for --probe.
    int64_t lastFrameUs_ = 0;
    uint32_t gaps_ = 0, completed_ = 0;
    int64_t lastTs_ = 0, sumDelta_ = 0, sumDur_ = 0;
    int32_t lastExposureUs_ = 0;
    std::vector<std::unique_ptr<Request>> reqs_;
    std::map<int, std::pair<uint8_t *, size_t>> maps_;
    std::map<const FrameBuffer *, std::array<uint8_t *, 3>> planes_;
    bool running_ = false, dirty_ = false, adjusted_ = false;
    std::mutex mu_;
    std::condition_variable cv_;
    std::deque<Request *> done_;
    Controls ctl_;
    double aspect_ = 4.0 / 3;
    Rectangle cropMax_;
    std::string model_;
    Size full_, binned_, hd_;
    std::vector<Size> modes_;
};

const char *faultName(Fault f)
{
    switch (f) {
    case Fault::None:
        return "ok";
    case Fault::NoCamera:
        return "no camera";
    case Fault::NoMemory:
        return "out of CMA";
    case Fault::Busy:
        return "busy";
    default:
        return "failed";
    }
}

// Waits for wakes for `ms` and counts the frames that came.
int countFrames(Lc &lc, int wakeR, int ms, uint64_t &firstMs)
{
    uint64_t t0 = nowMs(), end = t0 + ms;
    int n = 0;
    firstMs = 0;
    while (nowMs() < end) {
        pollfd p{wakeR, POLLIN, 0};
        poll(&p, 1, int(end - nowMs()));
        char buf[64];
        while (::read(wakeR, buf, sizeof buf) > 0) {
        }
        lc.frames(
            [&](const View &) {
                if (!n++)
                    firstMs = nowMs() - t0;
            },
            nullptr);
    }
    return n;
}

void describe(Lc &lc, const char *what)
{
    std::string streams;
    for (size_t i = 0; i < lc.cfg_->size(); i++)
        streams += (i ? " + " : "") + lc.cfg_->at(i).toString() + " stride " +
                   std::to_string(lc.cfg_->at(i).stride);
    printf("%s: %s%s, sensor %s, crop max %s\n", what, streams.c_str(),
           lc.adjusted_ ? " (adjusted)" : "",
           lc.cfg_->sensorConfig ? lc.cfg_->sensorConfig->outputSize.toString().c_str() : "auto",
           lc.cropMax_.toString().c_str());
}

// The gallery's pictures of a full photo (DIR/probe-1x.jpg): a tile from
// its EXIF, the full view, and each zoom to one photo pixel per screen
// pixel. Needs no sensor.
void gallery(const char *dir)
{
    std::string shot = std::string(dir ? dir : ".") + "/probe-1x.jpg";
    std::vector<uint16_t> px;
    struct {
        const char *what;
        int w, h;
        float zoom;
    } views[] = {{"tile", 53, 40, 1},       {"full", 320, 170, 1}, {"zoom 2", 320, 170, 2},
                 {"zoom 4", 320, 170, 4},   {"zoom 8", 320, 170, 8},
                 {"zoom 1:1", 320, 170, 15}};
    for (auto &v : views) {
        uint64_t t0 = nowMs();
        bool ok = jpegTo565(shot, v.w, v.h, v.zoom, 0.7f, 0.6f, px);
        printf("gallery %s: %s in %llu ms\n", v.what, ok ? "ok" : "failed",
               (unsigned long long)(nowMs() - t0));
    }
}

} // namespace

std::unique_ptr<Backend> makeBackend(int wakeFd)
{
    return std::make_unique<Lc>(wakeFd);
}

int probe(const char *dir)
{
    int wake[2];
    if (pipe(wake) != 0)
        return 1;
    for (int fd : wake)
        fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK);
    Lc lc(wake[1]);
    printf("libcamera: %s\n", CameraManager::version().c_str());
    Lc::usePipelineConfig();
    printf("pipeline config: %s\n", getenv("LIBCAMERA_RPI_CONFIG_FILE") ?: "libcamera's default");
    printf("cma free idle: %ld kB\n", cmaFreeKb());
    Fault f = lc.open();
    if (f != Fault::None) {
        printf("camera: %s\n", faultName(f));
        gallery(dir);
        return 1;
    }
    const ControlList &props = lc.cam_->properties();
    printf("camera: %s, %s\n", lc.model_.c_str(), lc.cam_->id().c_str());
    printf("pixel array: %s\n", lc.full_.toString().c_str());
    printf("rotation: %d\n", props.get(properties::Rotation).value_or(-1));
    printf("location: %d\n", props.get(properties::Location).value_or(-1));
    printf("modes:");
    for (const Size &s : lc.modes_)
        printf(" %s", s.toString().c_str());
    printf("\nchosen: still %s, preview %s, 1080p %s\n", lc.full_.toString().c_str(),
           lc.binned_.toString().c_str(), lc.hd_.toString().c_str());

    struct {
        const char *name;
        Mode mode;
        int w, h;
    } previews[] = {{"photo preview", Mode::Photo, 226, 170},
                    {"1080p preview", Mode::Video1080, 302, 170},
                    {"720p preview", Mode::Video720, 302, 170}};
    Controls c;
    for (auto &p : previews) {
        uint64_t t0 = nowMs();
        f = lc.preview(p.mode, p.w, p.h, c);
        if (f != Fault::None) {
            printf("%s: %s\n", p.name, faultName(f));
            continue;
        }
        uint64_t started = nowMs() - t0, first;
        describe(lc, p.name);
        int n = countFrames(lc, wake[0], 3000, first);
        printf("%s: start %llu ms, first frame %llu ms, %.1f fps, cma free %ld kB\n", p.name,
               (unsigned long long)started, (unsigned long long)first, n / 3.0, cmaFreeKb());
        lc.stop();
    }

    // Five seconds of each video size: what the encoder keeps up with.
    // GH0ST_PROBE_FLICKER=50 adds the app's anti-flicker setting.
    if (const char *fl = getenv("GH0ST_PROBE_FLICKER"))
        c.flicker = atoi(fl);
    for (auto &p : {previews[1], previews[2]}) {
        f = lc.preview(p.mode, p.w, p.h, c);
        if (f != Fault::None) {
            printf("%s rec: %s\n", p.name, faultName(f));
            continue;
        }
        lc.completed_ = lc.gaps_ = 0;
        lc.lastTs_ = lc.sumDelta_ = lc.sumDur_ = 0;
        Recorder rec(lc);
        std::string why, micWhy, path = std::string(dir ? dir : ".") + "/probe-" +
                                        (p.mode == Mode::Video1080 ? "1080" : "720") + ".mp4";
        // With sound, as the app records: ffprobe the file for both tracks.
        if (!rec.start(path, true, "", why, micWhy)) {
            printf("%s rec: %s\n", p.name, why.c_str());
            lc.stop();
            continue;
        }
        if (!micWhy.empty())
            printf("%s rec: no sound: %s\n", p.name, micWhy.c_str());
        int shown = 0;
        uint64_t convMs = 0;
        std::vector<uint16_t> px(size_t(p.w) * p.h);
        uint64_t end = nowMs() + 5000;
        while (nowMs() < end) {
            pollfd fds[2] = {{wake[0], POLLIN, 0}, {rec.fd(), POLLIN, 0}};
            poll(fds, 2, int(end - nowMs()));
            char buf[64];
            while (::read(wake[0], buf, sizeof buf) > 0) {
            }
            lc.frames(
                [&](const View &v) {
                    // The app path's work: every preview frame to RGB565.
                    uint64_t t = nowMs();
                    viewTo565(v, p.w, p.h, px.data());
                    convMs += nowMs() - t;
                    shown++;
                },
                [&](const VideoFrame &vf) { return rec.frame(vf); });
            if (fds[1].revents)
                rec.service();
        }
        long cma = cmaFreeKb();
        printf("%s rec: %u frames, %u lost, preview to RGB565 %.2f ms a frame, real frame %.1f us, "
               "sensor clock %.3f (set %.3f)\n",
               p.name, lc.completed_, lc.gaps_, shown ? double(convMs) / shown : 0.0,
               lc.completed_ > 1 ? lc.sumDelta_ / 1000.0 / (lc.completed_ - 1) : 0.0,
               lc.sumDur_ ? double(lc.sumDelta_) / lc.sumDur_ : 0.0, sensorClock());
        lc.completed_ = lc.gaps_ = 0;
        lc.lastTs_ = 0;
        rec.stop();
        printf("%s rec: %lld ms, %llu bytes (%.1f Mb/s), %d dropped, preview %.1f fps, cma free %ld kB "
               "-> %s\n",
               p.name, (long long)rec.ms(), (unsigned long long)rec.bytes(),
               rec.ms() ? rec.bytes() * 8.0 / rec.ms() / 1000 : 0.0, rec.dropped(), shown / 5.0, cma,
               path.c_str());
        lc.stop();
    }

    for (float zoom : {1.0f, 2.0f}) {
        c.zoom = zoom;
        if (lc.preview(Mode::Photo, 226, 170, c) == Fault::None) {
            uint64_t first;
            countFrames(lc, wake[0], 1500, first);
        }
        Yuv420 img;
        ShotMeta meta;
        std::string why;
        uint64_t t0 = nowMs();
        long cma = cmaFreeKb();
        f = lc.still(c, img, meta, why);
        uint64_t took = nowMs() - t0;
        if (f != Fault::None) {
            printf("still %gx: %s (%s)\n", zoom, faultName(f), why.c_str());
            continue;
        }
        printf("still %gx: %dx%d in %llu ms, real exposure %lld us, gain %.2f, cma free before %ld kB\n",
               zoom, img.w, img.h, (unsigned long long)took, (long long)meta.exposure_us,
               meta.gain, cma);
        t0 = nowMs();
        f = lc.preview(Mode::Photo, 226, 170, c);
        uint64_t first = 0;
        int back = f == Fault::None ? countFrames(lc, wake[0], 1000, first) : 0;
        printf("preview after the still: %s, first frame %llu ms after %llu ms to start\n",
               back ? "ok" : faultName(f), (unsigned long long)first,
               (unsigned long long)(nowMs() - t0 - 1000));
        lc.stop();
        std::string path = std::string(dir ? dir : ".") + "/probe-" +
                           std::to_string(int(zoom)) + "x.jpg";
        for (int q : {85, 95}) {
            t0 = nowMs();
            bool ok = saveJpeg(img, meta, q, path, why);
            FILE *fp = fopen(path.c_str(), "rb");
            long size = -1;
            if (fp) {
                fseek(fp, 0, SEEK_END);
                size = ftell(fp);
                fclose(fp);
            }
            printf("jpeg q%d: %s in %llu ms, %ld bytes -> %s\n", q, ok ? "ok" : why.c_str(),
                   (unsigned long long)(nowMs() - t0), size, path.c_str());
        }
    }
    gallery(dir);
    lc.close();
    return 0;
}

} // namespace camd
