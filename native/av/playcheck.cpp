// A host check of the player: plays a file against the silent stub
// server, then pauses, seeks while paused, plays on and runs to the end.
// A second file, whose picture won't decode, must end in an error.
//   make playcheck O=out && out/playcheck in.mp4 [broken.mp4]
// With --more and the media corpus (crates/gh0st-media/mediad/corpus.sh),
// it also plays sound alone, a turned video, a loop, A to B and a step,
// and in the media helper's build speed and subtitles:
//   out/playcheck --more CORPUS_DIR
#include "player.h"

#include <chrono>
#include <string>
#include <vector>
#include <cstdio>
#include <fcntl.h>
#include <poll.h>
#include <thread>
#include <unistd.h>

using namespace gh0st;

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

std::vector<uint16_t> shot;
int shotW = 0, shotH = 0;

// As pump, keeping the newest picture.
void watch(Player &p, int ms)
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
        if (p.picture(px, w, h)) {
            pictures++;
            shot = px;
            shotW = w;
            shotH = h;
        }
        Player::Status s;
        if (p.news(s))
            last = s;
    }
}

#ifdef GH0ST_PLAYER_EXTRAS
// Near-white pixels in the lower third: where subtitles sit.
int bright()
{
    int n = 0;
    for (int y = shotH * 2 / 3; y < shotH; y++)
        for (int x = 0; x < shotW; x++) {
            uint16_t v = shot[size_t(y) * shotW + x];
            n += (v >> 11) > 28 && ((v >> 5) & 63) > 58 && (v & 31) > 28;
        }
    return n;
}
#endif

int more(const std::string &dir, int wakeW)
{
    Player p(wakeW);
    std::string why;
    auto open = [&](const char *name, Player::Options o) {
        last = Player::Status();
        pictures = 0;
        bool ok = p.open(dir + "/" + name, 320, 170, o, why);
        if (!ok)
            printf("open %s: %s\n", name, why.c_str());
        return ok;
    };
    Player::Options plain;
    if (open("song.mp3", plain)) {
        watch(p, 1500);
        check(!p.hasPicture() && pictures == 0 && last.posMs >= 1000, "sound alone keeps time");
        watch(p, 2500);
        check(last.ended, "sound alone ends");
    }
    Player::Options turned;
    turned.orient = 6;
    if (open("rotated.mp4", turned)) {
        watch(p, 500);
        check(shotW == 126 && shotH == 170, "a video turned a quarter shows tall");
    }
    Player::Options loop;
    loop.loop = true;
    loop.silent = true;
    if (open("anim.gif", loop)) {
        watch(p, 2500);
        check(!last.ended && pictures > 24, "a moving picture goes round");
    }
    if (open("long.mp4", plain)) {
        p.repeat(1000, 2000);
        watch(p, 3500);
        check(last.posMs >= 1000 && last.posMs <= 2100 && !last.ended, "A to B plays over and over");
        p.repeat(0, 0);
        p.pause(true);
        watch(p, 300);
        int before = pictures;
        int64_t at = last.posMs;
        p.step();
        watch(p, 400);
        check(pictures == before + 1 && last.posMs > at && last.posMs < at + 100,
              "a step shows the next picture");
    }
#ifdef GH0ST_PLAYER_EXTRAS
    if (open("long.mp4", plain)) {
        watch(p, 300);
        p.speed(200);
        int64_t at = last.posMs;
        watch(p, 1500);
        check(last.posMs - at > 2400 && last.posMs - at < 3600, "twice the speed");
    }
    Player::Options none;
    if (open("tracks.mkv", none)) {
        watch(p, 700);
        int without = bright();
        Player::Options subs;
        subs.subs = 0;
        open("tracks.mkv", subs);
        watch(p, 700);
        int with = bright();
        printf("     bright pixels: %d without, %d with\n", without, with);
        check(with > without + 20, "subtitles in the file show");
    }
    Player::Options side;
    side.subs = 0;
    if (open("sidecar.mp4", side)) {
        watch(p, 700);
        int with = bright();
        check(with > 20, "a .srt beside the file shows");
    }
#endif
    p.close();
    return fails ? 1 : 0;
}

} // namespace

int main(int argc, char **argv)
{
    if (argc != 2 && argc != 3)
        return fprintf(stderr, "usage: playcheck in.mp4 [broken.mp4] | --more CORPUS_DIR\n"), 2;
    int fds[2];
    if (pipe(fds))
        return 1;
    wakeR = fds[0];
    fcntl(wakeR, F_SETFL, O_NONBLOCK);
    if (std::string(argv[1]) == "--more")
        return more(argv[2], fds[1]);
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
