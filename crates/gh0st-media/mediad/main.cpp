// gh0st-mediad: the decoder gh0st-media starts. It reads every file the
// app shows, so a file that crashes a decoder takes this process with it,
// never the app. One poll loop: the app's link, a wake pipe the player
// writes to, and signals. docs/media.md has the protocol.
#include "cache.h"
#include "link.h"
#include "ring.h"
#include "stills.h"

#include <cerrno>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <poll.h>
#include <sys/resource.h>
#include <unistd.h>
#ifdef __linux__
#include <sys/prctl.h>
#endif

extern "C" {
#include <libavutil/log.h>
}

namespace mediad {

namespace {

const char *VERSION = "1";
// Slots 0 to 2 take a playing video's pictures in turn; the last one
// answers asks, one at a time.
const uint32_t PLAY_SLOTS = 3;
// A still that takes longer than this is a hang: the helper ends, and the
// app starts it again and marks the file.
const unsigned STILL_SECONDS = 10;
// The tiles kept on the card.
const size_t CACHE_MOST = 32u << 20, CACHE_KEEP = 24u << 20;

int wakeW = -1;

void onSignal(int)
{
    int e = errno;
    (void)!::write(wakeW, "s", 1);
    errno = e;
}


class Helper {
public:
    Helper(int link, int wakeR) : link_(link), wakeR_(wakeR), cache_(ThumbCache::defaultDir()) {}

    bool init(int ringFd)
    {
        if (!ring_.open(ringFd) || ring_.slots() < PLAY_SLOTS + 1)
            return false;
        cache_.trim(CACHE_MOST, CACHE_KEEP);
        return true;
    }

    int run()
    {
        for (;;) {
            pollfd fds[2] = {{link_.fd(), POLLIN, 0}, {wakeR_, POLLIN, 0}};
            if (poll(fds, 2, -1) < 0 && errno != EINTR)
                return 1;
            if (fds[1].revents & POLLIN) {
                char buf[64];
                bool sig = false;
                ssize_t n;
                while ((n = read(wakeR_, buf, sizeof buf)) > 0)
                    sig = sig || memchr(buf, 's', size_t(n));
                if (sig)
                    return 0;
            }
            std::vector<std::string> m;
            while (link_.recv(m))
                if (!command(m))
                    return 0;
            if (link_.closed())
                return 0;
        }
    }

private:
    void say(const std::vector<std::string> &f) { link_.send(f); }

    void fail(const std::string &id, Fail f) { say({"fail", id, failWord(f)}); }

    // Hands a picture to the app in the ask slot.
    void answer(const char *verb, const std::string &id, const Pic &p)
    {
        if (p.empty() || size_t(p.w) * p.h > ring_.slotPx()) {
            fail(id, Fail::TooBig);
            return;
        }
        uint32_t slot = ring_.slots() - 1;
        uint32_t seq = ring_.write(slot, uint16_t(p.w), uint16_t(p.h), [&](uint16_t *dst) {
            memcpy(dst, p.px.data(), p.px.size() * 2);
        });
        say({verb, id, std::to_string(slot), std::to_string(seq), std::to_string(p.w),
             std::to_string(p.h)});
    }

    bool fits(int w, int h) const { return w > 0 && h > 0 && size_t(w) * h <= ring_.slotPx(); }

    void probe(const std::string &id, const std::string &path)
    {
        Info i;
        alarm(STILL_SECONDS);
        Fail f = stills_.probe(path, i);
        alarm(0);
        if (f != Fail::None)
            return fail(id, f);
        say({"info", id, kindWord(i.kind), std::to_string(i.w), std::to_string(i.h),
             std::to_string(i.durMs), std::to_string(i.audio), std::to_string(i.subs), i.format,
             i.vcodec, i.acodec});
    }

    void tags(const std::string &id, const std::string &path)
    {
        Tags t;
        alarm(STILL_SECONDS);
        Fail f = stills_.tags(path, t);
        alarm(0);
        if (f != Fail::None)
            return fail(id, f);
        say({"tags", id, t.title, t.artist, t.album, t.track, t.date});
    }

    void thumb(const std::string &id, int w, int h, const std::string &path)
    {
        if (!fits(w, h))
            return fail(id, Fail::TooBig);
        Pic p;
        bool bad = false;
        if (cache_.get(path, w, h, p, bad))
            return answer("thumb", id, p);
        if (bad)
            return fail(id, Fail::Corrupt);
        alarm(STILL_SECONDS);
        Fail f = stills_.thumb(path, w, h, p);
        alarm(0);
        if (f == Fail::None) {
            cache_.put(path, w, h, &p);
            return answer("thumb", id, p);
        }
        // A file that can't be read now may be readable later: only one
        // that is broken is remembered as bad.
        if (f == Fail::Corrupt || f == Fail::Unsupported || f == Fail::TooBig)
            cache_.put(path, w, h, nullptr);
        fail(id, f);
    }

    void view(const std::string &id, int w, int h, float zoom, float cx, float cy,
              const std::string &path)
    {
        if (!fits(w, h))
            return fail(id, Fail::TooBig);
        Pic p;
        alarm(STILL_SECONDS);
        Fail f = stills_.view(path, w, h, zoom, cx, cy, p);
        alarm(0);
        if (f != Fail::None)
            return fail(id, f);
        answer("view", id, p);
    }

    // False on quit.
    bool command(const std::vector<std::string> &m)
    {
        auto at = [&](size_t i) { return i < m.size() ? m[i] : std::string(); };
        // The path is always the last field.
        const std::string &path = m.back();
        const std::string verb = at(0);
        if (verb == "hello") {
            say({"hello", VERSION});
        } else if (verb == "quit") {
            return false;
        } else if (verb == "probe" && m.size() >= 3) {
            probe(at(1), path);
        } else if (verb == "tags" && m.size() >= 3) {
            tags(at(1), path);
        } else if (verb == "thumb" && m.size() >= 5) {
            thumb(at(1), atoi(at(2).c_str()), atoi(at(3).c_str()), path);
        } else if (verb == "view" && m.size() >= 8) {
            view(at(1), atoi(at(2).c_str()), atoi(at(3).c_str()), strtof(at(4).c_str(), nullptr),
                 strtof(at(5).c_str(), nullptr), strtof(at(6).c_str(), nullptr), path);
        } else if (verb == "forget") {
            stills_.forget();
        }
        return true;
    }

    gh0st::Link link_;
    gh0st::Ring ring_;
    int wakeR_;
    Stills stills_;
    ThumbCache cache_;
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
} // namespace mediad

int main()
{
    using namespace mediad;
    // The app's log: what fails, not every probe's advice.
    av_log_set_level(AV_LOG_ERROR);
    int link = envFd("GH0ST_MEDIAD_FD"), ring = envFd("GH0ST_MEDIAD_RING");
    if (link < 0 || ring < 0) {
        fprintf(stderr, "gh0st-mediad: run by gh0st-media\n");
        return 2;
    }
    fcntl(link, F_SETFL, fcntl(link, F_GETFL) | O_NONBLOCK);
#ifdef __linux__
    prctl(PR_SET_PDEATHSIG, SIGTERM);
#endif
    if (getppid() == 1)
        return 0;
    // A decoder that runs away stops here, not in the kernel's OOM killer.
    // GH0ST_MEDIAD_AS_MB sets it; 0 turns it off.
    const char *as = getenv("GH0ST_MEDIAD_AS_MB");
    long mb = as ? atol(as) : 1536;
    if (mb > 0) {
        rlimit r = {rlim_t(mb) << 20, rlim_t(mb) << 20};
        setrlimit(RLIMIT_AS, &r);
    }

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

    Helper h(link, wake[0]);
    if (!h.init(ring)) {
        fprintf(stderr, "gh0st-mediad: GH0ST_MEDIAD_RING is not a frame ring\n");
        return 2;
    }
    return h.run();
}
