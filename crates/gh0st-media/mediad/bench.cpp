// The device's numbers for docs/media.md: plays or shows a file as the
// helper does, on the 320x170 screen, and says how it went.
//
//   bench play FILE [SECONDS]   pictures a second, dropped, CPU, peak RSS
//   bench view FILE             probe and first view time, peak RSS
//
// Sound goes to the stub server: playing measures the picture.
#include "player.h"
#include "stills.h"

#include <chrono>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <fstream>
#include <malloc.h>
#include <poll.h>
#include <string>
#include <sys/resource.h>
#include <unistd.h>

using Clock = std::chrono::steady_clock;

static long statusKb(const char *key)
{
    std::ifstream f("/proc/self/status");
    std::string line;
    while (std::getline(f, line))
        if (line.rfind(key, 0) == 0)
            return std::stol(line.substr(strlen(key)));
    return 0;
}

static long peakKb()
{
    return statusKb("VmHWM:");
}

static double cpuSeconds()
{
    rusage u{};
    getrusage(RUSAGE_SELF, &u);
    return u.ru_utime.tv_sec + u.ru_stime.tv_sec + (u.ru_utime.tv_usec + u.ru_stime.tv_usec) / 1e6;
}

static double ms(Clock::time_point a, Clock::time_point b)
{
    return std::chrono::duration<double, std::milli>(b - a).count();
}

static int play(const char *path, int seconds)
{
    int wake[2];
    if (pipe2(wake, O_NONBLOCK | O_CLOEXEC) != 0)
        return 1;
    gh0st::Player p(wake[1]);
    std::string why;
    auto t0 = Clock::now();
    double c0 = cpuSeconds();
    gh0st::Player::Options o;
    o.silent = true;
    if (!p.open(path, 320, 170, o, why)) {
        printf("%s: %s\n", path, why.c_str());
        return 1;
    }
    std::vector<uint16_t> px;
    int w = 0, h = 0, shown = 0, dropped = 0, slow = 0;
    bool ended = false;
    double first = -1;
    while (ms(t0, Clock::now()) < seconds * 1000.0 && !ended) {
        pollfd f{wake[0], POLLIN, 0};
        poll(&f, 1, 100);
        char b[64];
        while (read(wake[0], b, sizeof b) > 0) {
        }
        while (p.picture(px, w, h)) {
            shown++;
            if (first < 0)
                first = ms(t0, Clock::now());
        }
        gh0st::Player::Status s;
        while (p.news(s)) {
            dropped = s.dropped;
            slow = s.slow;
            ended = s.ended;
            if (!s.error.empty()) {
                printf("%s: %s\n", path, s.error.c_str());
                return 1;
            }
        }
    }
    double wall = ms(t0, Clock::now()) / 1000.0;
    double cpu = cpuSeconds() - c0;
    printf("%s: %.1f pictures/s, %d dropped, slow %d, first %.0f ms, CPU %.0f%% of one core, "
           "peak %ld kB\n",
           path, shown / wall, dropped, slow, first, cpu / wall * 100, peakKb());
    p.close();
    return 0;
}

static int view(const char *path)
{
    mediad::Stills s;
    mediad::Info i;
    auto t0 = Clock::now();
    mediad::Fail f = s.probe(path, i);
    auto t1 = Clock::now();
    if (f != mediad::Fail::None) {
        printf("%s: %s\n", path, mediad::failWord(f));
        return 1;
    }
    mediad::Pic pic;
    f = s.view(path, 320, 170, 1, 0.5f, 0.5f, pic);
    auto t2 = Clock::now();
    mediad::Pic zoomed;
    s.view(path, 320, 170, 4, 0.5f, 0.5f, zoomed);
    auto t3 = Clock::now();
    long held = statusKb("VmRSS:");
    s.forget();
    malloc_trim(0);
    printf("%s: %dx%d, probe %.0f ms, view %.0f ms (%s), zoomed again %.0f ms, peak %ld kB, "
           "held %ld kB, after forget %ld kB\n",
           path, i.w, i.h, ms(t0, t1), ms(t1, t2), mediad::failWord(f), ms(t2, t3), peakKb(), held,
           statusKb("VmRSS:"));
    return 0;
}

int main(int argc, char **argv)
{
    if (argc >= 3 && !strcmp(argv[1], "play"))
        return play(argv[2], argc > 3 ? atoi(argv[3]) : 10);
    if (argc >= 3 && !strcmp(argv[1], "view"))
        return view(argv[2]);
    fprintf(stderr, "usage: bench play FILE [SECONDS] | bench view FILE\n");
    return 2;
}
