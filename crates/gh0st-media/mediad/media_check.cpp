// Host checks of the still decoders over the corpus corpus.sh writes: every
// kind of file opens, tiles fill, views fit, orientations come out
// upright, and files that lie, end early or would take gigabytes are
// refused rather than crash. An animated WebP keeps a sane pace. `make
// check CORPUS=DIR` runs it.
#include "anim.h"
#include "decode.h"
#include "stills.h"

#include <chrono>
#include <fcntl.h>
#include <poll.h>
#include <unistd.h>

extern "C" {
#include <libavutil/log.h>
}
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <string>

using namespace mediad;

namespace {

int failed = 0;
std::string dir;

void expect(bool ok, const std::string &what)
{
    std::printf("%s %s\n", ok ? "ok  " : "FAIL", what.c_str());
    failed += !ok;
}

std::string at(const char *name)
{
    return dir + "/" + name;
}

double ms(std::chrono::steady_clock::time_point t0)
{
    return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
}

// Each file: what it should be, and that a tile and a view come out.
void opens(const char *name, Kind kind, bool hasPicture = true)
{
    Stills s;
    Info i;
    Fail f = s.probe(at(name), i);
    if (f != Fail::None) {
        expect(false, std::string(name) + ": " + failWord(f));
        return;
    }
    bool ok = i.kind == kind;
    Pic tile, view;
    auto t0 = std::chrono::steady_clock::now();
    Fail ft = s.thumb(at(name), 60, 45, tile);
    Fail fv = s.view(at(name), 320, 170, 1, 0.5f, 0.5f, view);
    double t = ms(t0);
    if (hasPicture)
        ok = ok && ft == Fail::None && tile.w == 60 && tile.h == 45 && fv == Fail::None &&
             view.w <= 320 && view.h <= 170 && (view.w == 320 || view.h == 170 ||
                                                (view.w == i.w && view.h == i.h));
    char what[200];
    snprintf(what, sizeof what, "%s: %s %dx%d %s/%s/%s, tile %s, view %dx%d %s, %.0f ms", name,
             kindWord(i.kind), i.w, i.h, i.format.c_str(), i.vcodec.c_str(), i.acodec.c_str(),
             failWord(ft), view.w, view.h, failWord(fv), t);
    expect(ok, what);
}

void refused(const char *name, Fail want)
{
    Stills s;
    Pic p;
    Fail f = s.view(at(name), 320, 170, 1, 0.5f, 0.5f, p);
    expect(f == want, std::string(name) + " is refused: " + failWord(f));
}

// The mean difference of two pictures of one size, 0 to 255.
double diff(const Pic &a, const Pic &b)
{
    if (a.w != b.w || a.h != b.h)
        return 255;
    double sum = 0;
    for (size_t i = 0; i < a.px.size(); i++) {
        int ga = (a.px[i] >> 5) & 63, gb = (b.px[i] >> 5) & 63;
        sum += std::abs(ga - gb) * 4;
    }
    return sum / double(a.px.size());
}

// A picture with no alpha scales straight to RGB565: its colour must come
// through as pack565 gives it, give or take swscale's dither.
void colours()
{
    struct Want {
        const char *name;
        int r, g, b;
    };
    for (Want w : {Want{"tall.png", 160, 90, 40}, Want{"pano.png", 40, 160, 90}}) {
        Stills s;
        Pic p;
        s.view(at(w.name), 320, 170, 1, 0.5f, 0.5f, p);
        uint16_t got = p.empty() ? 0 : p.px[size_t(p.h / 2) * p.w + p.w / 2];
        uint16_t want = gh0st::pack565(w.r, w.g, w.b);
        int dr = (got >> 11) - (want >> 11), dg = ((got >> 5) & 63) - ((want >> 5) & 63),
            db = (got & 31) - (want & 31);
        char what[100];
        snprintf(what, sizeof what, "%s keeps its colour: %04x for %04x", w.name, got, want);
        expect(std::abs(dr) <= 1 && std::abs(dg) <= 1 && std::abs(db) <= 1, what);
    }
}

// orient-N.jpg holds orient-1's pixels with tag N: shown upright, each is
// orient-1's picture turned as N says.
void orientations()
{
    Stills s;
    Pic up;
    s.view(at("orient-1.jpg"), 120, 120, 1, 0.5f, 0.5f, up);
    for (int o = 2; o <= 8; o++) {
        char name[32];
        snprintf(name, sizeof name, "orient-%d.jpg", o);
        Info i;
        s.probe(at(name), i);
        Pic p;
        s.view(at(name), 120, 120, 1, 0.5f, 0.5f, p);
        Pic want = gh0st::orient(up, o);
        bool sides = gh0st::swapsSides(o) ? i.w == 1200 && i.h == 1600 : i.w == 1600 && i.h == 1200;
        double d = diff(p, want);
        char what[100];
        snprintf(what, sizeof what, "%s is upright: %dx%d, off by %.1f", name, p.w, p.h, d);
        expect(sides && d < 12, what);
    }
}

void zoomed()
{
    Stills s;
    Pic whole, close, corner;
    s.view(at("pano.png"), 320, 170, 1, 0.5f, 0.5f, whole);
    s.view(at("pano.png"), 320, 170, 170.0f / 800 / (320.0f / 30000), 0.5f, 0.5f, close);
    expect(whole.w == 320 && whole.h < 20, "a panorama fits as a strip");
    expect(close.w == 320 && close.h == 170, "filled, it covers the screen");
    auto t0 = std::chrono::steady_clock::now();
    s.view(at("pano.png"), 320, 170, 40, 0.0f, 0.0f, corner);
    char what[80];
    snprintf(what, sizeof what, "a pan of the kept picture decodes nothing (%.0f ms)", ms(t0));
    expect(corner.w == 320 && corner.h == 170 && ms(t0) < 100, what);
    Pic j;
    s.view(at("pano.jpg"), 320, 170, 1, 0.5f, 0.5f, j);
    expect(j.w == 320 && j.h < 20, "a JPEG panorama fits as a strip");
}

// Frames of 0 ms show for 100 ms each, as a GIF's do: not as fast as
// they decode.
void zeroDurations()
{
    int fds[2];
    if (pipe(fds) != 0)
        return;
    fcntl(fds[0], F_SETFL, O_NONBLOCK);
    Anim a(fds[1]);
    std::string why;
    if (!a.open(at("fast.webp"), 320, 170, false, why)) {
        expect(false, "fast.webp plays: " + why);
        return;
    }
    int pictures = 0;
    std::vector<uint16_t> px;
    auto t0 = std::chrono::steady_clock::now();
    while (ms(t0) < 1000) {
        pollfd f{fds[0], POLLIN, 0};
        poll(&f, 1, 20);
        char buf[64];
        while (read(fds[0], buf, sizeof buf) > 0) {
        }
        int w, h;
        pictures += a.picture(px, w, h);
    }
    a.close();
    close(fds[0]);
    close(fds[1]);
    expect(pictures >= 8 && pictures <= 12,
           "frames of 0 ms show 100 ms each: " + std::to_string(pictures) + " in a second");
}

} // namespace

int main(int argc, char **argv)
{
    if (argc != 2)
        return std::fprintf(stderr, "usage: media_check CORPUS_DIR\n"), 2;
    dir = argv[1];
    av_log_set_level(AV_LOG_QUIET);
    for (auto n : {"photo.jpg", "progressive.jpg", "photo-plain.jpg", "notreally.png",
                   "alpha.png", "deep.png", "photo.webp", "photo.bmp", "photo.tiff", "photo.tga",
                   "photo.ppm", "photo.pgm", "photo.jp2", "photo.jxl", "photo.svg", "icon.ico",
                   "photo.heic", "grid.heic", "photo.avif", "tall.png", "pano.png", "pano.jpg"})
        opens(n, Kind::Image);
    for (auto n : {"anim.gif", "anim.png", "anim.webp", "fast.webp"})
        opens(n, Kind::Anim);
    for (auto n : {"h264.mp4", "hevc.mp4", "vp9.webm", "vp8.webm", "av1.mkv", "mpeg4.avi",
                   "mpeg2.ts", "theora.ogv", "clip.mov", "silent.mp4", "tracks.mkv", "4k.mp4"})
        opens(n, Kind::Video);
    // Embedded covers, then the cover.jpg beside the rest.
    for (auto n : {"song.mp3", "song.m4a", "song.flac", "song.ogg", "song.opus", "song.wav",
                   "song.aiff", "lossless.m4a", "album/01.mp3"})
        opens(n, Kind::Audio);
    opens("nocover/song.wav", Kind::Audio, false);

    Stills s;
    Info i;
    s.probe(at("rotated.mp4"), i);
    expect(i.w == 240 && i.h == 320, "a video turned a quarter shows tall");
    s.probe(at("tracks.mkv"), i);
    expect(i.audio == 2 && i.subs == 1, "two sound tracks and a subtitle");
    s.probe(at("sidecar.mp4"), i);
    expect(i.subs == 1, "a .srt beside the file counts");
    Tags t;
    s.tags(at("song.flac"), t);
    expect(t.title == "Corpus" && t.artist == "gh0st" && t.album == "Tests", "tags: " + t.title);

    orientations();
    colours();
    zoomed();
    zeroDurations();
    refused("bomb.png", Fail::TooBig);
    refused("truncated.png", Fail::Corrupt);
    refused("text.jpg", Fail::Corrupt);
    refused("truncated.mp4", Fail::Corrupt);
    refused("missing.jpg", Fail::Io);
    refused("nocover/song.wav", Fail::Unsupported);
    refused("badcover/song.wav", Fail::Unsupported);
    // Before a tile or a play is asked: play open trusts the probe.
    expect(s.probe(at("8k.mp4"), i) == Fail::TooBig, "an 8K video is too big at its probe");
    return failed ? 1 : 0;
}
