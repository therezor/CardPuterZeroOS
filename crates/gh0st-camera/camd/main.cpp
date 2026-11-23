// gh0st-camd: the camera helper gh0st-camera starts. It owns the sensor
// and the encoders; the app owns the screen. One poll loop: the app's link,
// a wake pipe the sensor and the JPEG worker write to, and signals.
#include "camd.h"
#include "link.h"
#include "rec.h"
#include "ring.h"
#include "still.h"

#include <cerrno>
#include <chrono>
#include <condition_variable>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <fcntl.h>
#include <mutex>
#include <poll.h>
#include <sys/stat.h>
#include <thread>
#include <unistd.h>
#ifdef __linux__
#include <sys/prctl.h>
#endif

namespace camd {

uint64_t nowMs()
{
    using namespace std::chrono;
    return duration_cast<milliseconds>(steady_clock::now().time_since_epoch()).count();
}

namespace {

const char *VERSION = "1";
const uint32_t PREVIEW_SLOTS = 3;
// Shots waiting to be encoded. Each holds a full picture in memory.
const size_t MAX_PENDING = 2;

int wakeW = -1;

void onSignal(int)
{
    int e = errno;
    (void)!::write(wakeW, "s", 1);
    errno = e;
}

// Encodes and saves stills on a thread of its own, so the preview comes
// back while a JPEG is written.
class Worker {
public:
    struct Job {
        Yuv420 img;
        ShotMeta meta;
        std::string path;
        int quality;
        uint64_t t0;
        // A video's poster: saved without a word to the app.
        bool quiet = false;
    };
    struct Done {
        std::string path;
        bool ok;
        std::string why;
        uint64_t ms;
        bool quiet;
    };

    explicit Worker(int wake) : wake_(wake) {}
    ~Worker() { finish(); }

    size_t pending()
    {
        std::lock_guard<std::mutex> l(mu_);
        return jobs_.size() + busy_;
    }

    void push(Job &&j)
    {
        {
            std::lock_guard<std::mutex> l(mu_);
            jobs_.push_back(std::move(j));
        }
        if (!thread_.joinable())
            thread_ = std::thread([this] { run(); });
        cv_.notify_one();
    }

    bool take(Done &d)
    {
        std::lock_guard<std::mutex> l(mu_);
        if (done_.empty())
            return false;
        d = std::move(done_.front());
        done_.pop_front();
        return true;
    }

    // Saves what is queued, then stops the thread.
    void finish()
    {
        {
            std::lock_guard<std::mutex> l(mu_);
            quit_ = true;
        }
        cv_.notify_one();
        if (thread_.joinable())
            thread_.join();
    }

private:
    void run()
    {
        for (;;) {
            Job j;
            {
                std::unique_lock<std::mutex> l(mu_);
                cv_.wait(l, [this] { return quit_ || !jobs_.empty(); });
                if (jobs_.empty())
                    return;
                j = std::move(jobs_.front());
                jobs_.pop_front();
                busy_ = 1;
            }
            Done d{j.path, false, "", 0, j.quiet};
            d.ok = saveJpeg(j.img, j.meta, j.quality, j.path, d.why);
            d.ms = nowMs() - j.t0;
            {
                std::lock_guard<std::mutex> l(mu_);
                done_.push_back(std::move(d));
                busy_ = 0;
            }
            (void)!::write(wake_, "j", 1);
        }
    }

    int wake_;
    std::mutex mu_;
    std::condition_variable cv_;
    std::deque<Job> jobs_;
    std::deque<Done> done_;
    size_t busy_ = 0;
    bool quit_ = false;
    std::thread thread_;
};

const char *faultWord(Fault f)
{
    switch (f) {
    case Fault::NoCamera:
        return "nocamera";
    case Fault::NoMemory:
        return "nomem";
    case Fault::Busy:
        return "busy";
    default:
        return "other";
    }
}

class Helper {
public:
    Helper(int linkFd, int wakeR, int wakeW)
        : link_(linkFd), wakeR_(wakeR), be_(makeBackend(wakeW)), worker_(wakeW), rec_(*be_)
    {
    }

    bool init(int ringFd) { return ring_.open(ringFd); }

    int run()
    {
        // Without PDEATHSIG, and on datagram links that never report a
        // hang-up, watch the parent instead.
#ifdef __linux__
        const int tick = -1;
#else
        const int tick = 1000;
#endif
        const pid_t parent = getppid();
        for (;;) {
            pollfd fds[3] = {{link_.fd(), POLLIN, 0}, {wakeR_, POLLIN, 0}, {rec_.fd(), POLLIN, 0}};
            int n = rec_.active() ? 3 : 2;
            if (poll(fds, n, tick) < 0 && errno != EINTR)
                break;
            if (getppid() != parent)
                break;
            if (fds[1].revents & POLLIN && !wake())
                break;
            if (n == 3 && fds[2].revents)
                encoded();
            std::vector<std::string> m;
            bool go = true;
            while (go && link_.recv(m))
                go = command(m);
            if (!go || link_.closed() || (fds[0].revents & (POLLHUP | POLLERR)))
                break;
        }
        shutdown();
        return 0;
    }

private:
    void say(std::vector<std::string> f) { link_.send(f); }

    void error(Fault f, const std::string &why = "")
    {
        if (f == Fault::Other)
            say({"error", "other", why});
        else
            say({"error", faultWord(f)});
    }

    // The picture between the app's rails, above its action bar
    // (preview_size in src/app.rs).
    void size(int &w, int &h) const
    {
        w = mode_ == Mode::Photo ? 226 : 302;
        h = 170;
    }

    // Drains the wake pipe. False on a signal: time to go.
    bool wake()
    {
        char buf[64];
        bool sig = false;
        ssize_t n;
        while ((n = ::read(wakeR_, buf, sizeof buf)) > 0)
            sig = sig || memchr(buf, 's', n);
        if (live_) {
            std::function<bool(const VideoFrame &)> video;
            if (rec_.active())
                video = [this](const VideoFrame &f) { return rec_.frame(f); };
            be_->frames(
                [this](const View &v) {
                    // A new recording's poster: the first preview frame,
                    // already small, saved off the loop.
                    if (poster_) {
                        poster_ = false;
                        Worker::Job j;
                        j.img = viewToYuv(v);
                        j.path = posterFor(rec_.path());
                        j.quality = 85;
                        j.t0 = nowMs();
                        j.quiet = true;
                        worker_.push(std::move(j));
                    }
                    int w, h;
                    size(w, h);
                    slot_ = (slot_ + 1) % PREVIEW_SLOTS;
                    uint32_t seq =
                        ring_.write(slot_, w, h, [&](uint16_t *px) { viewTo565(v, w, h, px); });
                    if (seq)
                        say({"frame", std::to_string(slot_), std::to_string(seq),
                             std::to_string(w), std::to_string(h)});
                },
                video);
        }
        Worker::Done d;
        while (worker_.take(d)) {
            if (d.quiet)
                continue;
            if (d.ok)
                say({"shot", "ok", d.path, std::to_string(d.ms)});
            else
                say({"shot", "err", d.path, d.why});
        }
        return !sig;
    }

    // The encoder made something: a second on disk is news for the app.
    void encoded()
    {
        if (rec_.service())
            say({"rec", std::to_string(rec_.ms()), std::to_string(rec_.bytes())});
        if (rec_.failed()) {
            std::string why = rec_.why();
            endRec();
            say({"error", "io", why});
        }
    }

    void startRec(const std::string &path)
    {
        if (rec_.active())
            return;
        std::string why;
        if (!live_ || mode_ == Mode::Photo) {
            say({"error", "io", "not in video mode"});
            return;
        }
        if (!rec_.start(path, why)) {
            say({"error", "io", why});
            return;
        }
        lostAtStart_ = be_->lostFrames();
        poster_ = true;
        say({"state", "rec"});
    }

    // Ends a recording, if one runs: the file is whole after this.
    void endRec()
    {
        if (!rec_.active())
            return;
        rec_.stop();
        // Frames the encoder had no room for: rare, and worth knowing.
        uint32_t lost = be_->lostFrames() - lostAtStart_;
        if (rec_.dropped() || lost)
            fprintf(stderr, "gh0st-camd: %s: %d frames dropped at the encoder, %u lost before it\n",
                    rec_.path().c_str(), rec_.dropped(), lost);
        say({"recdone", rec_.path()});
        if (live_)
            say({"state", "preview"});
    }

    // Where a video's poster lives: out of the photos folder, in our cache.
    static std::string posterFor(const std::string &video)
    {
        const char *home = getenv("HOME");
        std::string dir = std::string(home ? home : "/tmp") + "/.cache/gh0st-camera";
        mkdir((dir.substr(0, dir.rfind('/'))).c_str(), 0700);
        mkdir(dir.c_str(), 0700);
        std::string name = video.substr(video.find_last_of('/') + 1);
        return dir + "/" + name.substr(0, name.rfind('.')) + ".jpg";
    }

    bool openSensor()
    {
        if (open_)
            return true;
        Fault f = be_->open();
        if (f != Fault::None) {
            error(f);
            return false;
        }
        open_ = true;
        return true;
    }

    void startPreview()
    {
        int w, h;
        size(w, h);
        Fault f = be_->preview(mode_, w, h, ctl_);
        if (f != Fault::None) {
            live_ = false;
            error(f, "preview");
            return;
        }
        live_ = true;
        say({"state", "preview"});
    }

    void stopAll()
    {
        endRec();
        live_ = false;
        if (open_) {
            be_->close();
            open_ = false;
        }
    }

    void shoot(const std::string &path)
    {
        uint64_t t0 = nowMs();
        if (!live_) {
            say({"shot", "err", path, "the camera is off"});
            return;
        }
        if (worker_.pending() >= MAX_PENDING) {
            say({"shot", "err", path, "busy"});
            return;
        }
        say({"state", "busy"});
        live_ = false;
        Worker::Job j;
        j.meta.zoom = ctl_.zoom;
        j.meta.ev = ctl_.ev;
        std::string why;
        Fault f = be_->still(ctl_, j.img, j.meta, why);
        if (f == Fault::None) {
            j.path = path;
            j.quality = ctl_.quality;
            j.t0 = t0;
            worker_.push(std::move(j));
        } else {
            say({"shot", "err", path, f == Fault::NoMemory ? "nomem" : why});
        }
        startPreview();
    }

    // False on quit.
    bool command(const std::vector<std::string> &m)
    {
        auto at = [&](size_t i) { return i < m.size() ? m[i] : std::string(); };
        const std::string verb = at(0);
        if (verb == "hello") {
            if (openSensor())
                say({"hello", VERSION, be_->sensor(), std::to_string(be_->fullW()),
                     std::to_string(be_->fullH())});
        } else if (verb == "mode") {
            Mode m2 = mode_;
            if (at(1) == "photo")
                m2 = Mode::Photo;
            else if (at(1) == "video")
                m2 = at(2) == "1080" ? Mode::Video1080 : Mode::Video720;
            if (m2 != mode_) {
                endRec();
                mode_ = m2;
                if (live_) {
                    be_->stop();
                    startPreview();
                }
            }
        } else if (verb == "start") {
            if (!live_ && openSensor())
                startPreview();
        } else if (verb == "stop") {
            stopAll();
            say({"state", "off"});
        } else if (verb == "quit") {
            return false;
        } else if (verb == "set") {
            float v = strtof(at(2).c_str(), nullptr);
            if (at(1) == "zoom")
                ctl_.zoom = std::min(4.0f, std::max(1.0f, v));
            else if (at(1) == "ev")
                ctl_.ev = std::min(2.0f, std::max(-2.0f, v));
            else if (at(1) == "flicker")
                ctl_.flicker = int(v) == 50 || int(v) == 60 ? int(v) : 0;
            else if (at(1) == "quality")
                ctl_.quality = std::min(100, std::max(50, int(v)));
            if (live_)
                be_->set(ctl_);
        } else if (verb == "shoot") {
            endRec();
            shoot(at(1));
        } else if (verb == "rec") {
            // Sound comes later: `mic` is taken and not yet used.
            if (at(1) == "start")
                startRec(at(2));
            else if (at(1) == "stop")
                endRec();
        } else if (verb == "thumb") {
            int w = atoi(at(2).c_str()), h = atoi(at(3).c_str());
            std::vector<uint16_t> px;
            // A video's picture is the poster taken from its first frame.
            std::string src = at(1);
            if (src.size() > 4 && src.compare(src.size() - 4, 4, ".mp4") == 0)
                src = posterFor(src);
            float zoom = m.size() > 4 ? strtof(at(4).c_str(), nullptr) : 1;
            float cx = m.size() > 5 ? strtof(at(5).c_str(), nullptr) : 0.5f;
            float cy = m.size() > 6 ? strtof(at(6).c_str(), nullptr) : 0.5f;
            if (w > 0 && h > 0 && size_t(w) * h <= ring_.slotPx() &&
                jpegTo565(src, w, h, zoom, cx, cy, px)) {
                uint32_t slot = ring_.slots() - 1;
                uint32_t seq = ring_.write(slot, w, h, [&](uint16_t *dst) {
                    memcpy(dst, px.data(), px.size() * 2);
                });
                say({"thumb", at(1), std::to_string(slot), std::to_string(seq), std::to_string(w),
                     std::to_string(h)});
            } else {
                // Every ask gets an answer, so the app can keep one in flight.
                say({"nothumb", at(1)});
            }
        }
        return true;
    }

    void shutdown()
    {
        stopAll();
        worker_.finish();
        Worker::Done d;
        while (worker_.take(d))
            if (d.ok && !d.quiet)
                say({"shot", "ok", d.path, std::to_string(d.ms)});
    }

    Link link_;
    Ring ring_;
    int wakeR_;
    std::unique_ptr<Backend> be_;
    Worker worker_;
    Recorder rec_;
    Mode mode_ = Mode::Photo;
    Controls ctl_;
    bool open_ = false, live_ = false, poster_ = false;
    uint32_t slot_ = 0;
    uint32_t lostAtStart_ = 0;
};

int envFd(const char *name)
{
    const char *v = getenv(name);
    int n = v ? atoi(v) : -1;
    unsetenv(name);
    if (n < 3 || fcntl(n, F_GETFD) < 0)
        return -1;
    fcntl(n, F_SETFD, FD_CLOEXEC);
    return n;
}

} // namespace
} // namespace camd

int main(int argc, char **argv)
{
    using namespace camd;
    // libcamera is chatty by default; our logs stay off unless asked for.
    setenv("LIBCAMERA_LOG_LEVELS", "*:ERROR", 0);
    if (argc >= 2 && !strcmp(argv[1], "--probe"))
        return probe(argc >= 3 ? argv[2] : nullptr);

    int link = envFd("GH0ST_CAMD_FD"), ring = envFd("GH0ST_CAMD_RING");
    if (link < 0 || ring < 0) {
        fprintf(stderr, "gh0st-camd: run by gh0st-camera, or with --probe\n");
        return 2;
    }
    fcntl(link, F_SETFL, fcntl(link, F_GETFL) | O_NONBLOCK);
#ifdef __linux__
    prctl(PR_SET_PDEATHSIG, SIGTERM);
#endif
    if (getppid() == 1)
        return 0;

    int wake[2];
    if (pipe(wake) != 0)
        return 1;
    for (int fd : wake) {
        fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK);
        fcntl(fd, F_SETFD, FD_CLOEXEC);
    }
    wakeW = wake[1];
    struct sigaction sa = {};
    sa.sa_handler = onSignal;
    sigaction(SIGTERM, &sa, nullptr);
    sigaction(SIGINT, &sa, nullptr);
    sigaction(SIGHUP, &sa, nullptr);
    signal(SIGPIPE, SIG_IGN);

    Helper h(link, wake[0], wake[1]);
    if (!h.init(ring)) {
        fprintf(stderr, "gh0st-camd: GH0ST_CAMD_RING is not a frame ring\n");
        return 2;
    }
    return h.run();
}
