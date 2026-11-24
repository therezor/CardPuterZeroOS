// A host check of the player: plays a file against the silent stub
// server, then pauses, seeks while paused, plays on and runs to the end.
// A second file, whose picture won't decode, must end in an error.
//   make playcheck O=out && out/playcheck in.mp4 [broken.mp4]
#include "player.h"

#include <chrono>
#include <cstdio>
#include <fcntl.h>
#include <poll.h>
#include <thread>
#include <unistd.h>

using namespace camd;

namespace {

int wakeR = -1;
int pictures = 0;
Player::Status last;

// Runs the loop's side for `ms`: pictures counted, news kept.
void pump(Player &p, int ms)
{
    auto end = std::chrono::steady_clock::now() + std::chrono::milliseconds(ms);
    std::vector<uint16_t> px;
    while (std::chrono::steady_clock::now() < end) {
        pollfd f{wakeR, POLLIN, 0};
        poll(&f, 1, 20);
        char buf[64];
        while (read(wakeR, buf, sizeof buf) > 0) {
        }
        int w, h;
        if (p.picture(px, w, h))
            pictures++;
        Player::Status s;
        if (p.news(s))
            last = s;
    }
}

int fails = 0;
void check(bool ok, const char *what)
{
    printf("%s %s (pos %lld ms of %lld, %d pictures)\n", ok ? "ok  " : "FAIL", what,
           (long long)last.posMs, (long long)last.durMs, pictures);
    fails += !ok;
}

} // namespace

int main(int argc, char **argv)
{
    if (argc != 2 && argc != 3)
        return fprintf(stderr, "usage: playcheck in.mp4 [broken.mp4]\n"), 2;
    int fds[2];
    if (pipe(fds))
        return 1;
    wakeR = fds[0];
    fcntl(wakeR, F_SETFL, O_NONBLOCK);
    Player p(fds[1]);
    std::string why;
    if (!p.open(argv[1], 302, 170, why))
        return fprintf(stderr, "open: %s\n", why.c_str()), 1;
    pump(p, 2000);
    check(pictures > 50 && last.posMs >= 1000 && !last.paused, "plays at its pace");
    p.pause(true);
    pump(p, 300);
    int before = pictures;
    pump(p, 700);
    check(pictures == before && last.paused, "a pause holds the picture");
    p.seek(5000);
    pump(p, 500);
    check(pictures == before + 1 && last.posMs >= 5000 && last.paused, "a paused seek shows one picture");
    p.pause(false);
    pump(p, 1000);
    check(last.posMs > 5500 && !last.paused, "plays on from there");
    p.seek(last.durMs - 1000);
    pump(p, 2500);
    check(last.ended, "ends");
    p.pause(false);
    pump(p, 800);
    check(!last.ended && last.posMs < 1500, "plays again from the start");
    p.close();
    if (argc == 3) {
        last = Player::Status();
        if (!p.open(argv[2], 302, 170, why)) {
            check(true, "a broken file is refused at open");
        } else {
            pump(p, 6000);
            check(!last.error.empty(), "a picture that won't decode ends in an error");
            p.close();
        }
    }
    return fails ? 1 : 0;
}
