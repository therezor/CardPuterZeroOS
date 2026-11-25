// Host checks of the looks: a neutral look leaves a photo as it was, the
// preview's table agrees with the still's path, every id is found, and odd
// sizes stay inside the picture. `make check` builds and runs them.
#include "look.h"

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cmath>
#include <cstring>
#include <set>

using namespace camd;

namespace {

int failed = 0;

void expect(bool ok, const char *what)
{
    std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
    failed += !ok;
}

struct Rgb {
    int r, g, b;
};

void toYuv(Rgb c, int &y, int &u, int &v)
{
    y = clamp8(int(0.299 * c.r + 0.587 * c.g + 0.114 * c.b + 0.5));
    u = clamp8(int(-0.168736 * c.r - 0.331264 * c.g + 0.5 * c.b + 128.5));
    v = clamp8(int(0.5 * c.r - 0.418688 * c.g - 0.081312 * c.b + 128.5));
}

Rgb toRgb(int y, int u, int v)
{
    return {clamp8(int(y + 1.402 * (v - 128) + 0.5)),
            clamp8(int(y - 0.344136 * (u - 128) - 0.714136 * (v - 128) + 0.5)),
            clamp8(int(y + 1.772 * (u - 128) + 0.5))};
}

// One colour per 2x2 block, so the chroma loses nothing.
Yuv420 blocks(const std::vector<Rgb> &cols, int cw)
{
    int chh = int((cols.size() + cw - 1) / cw);
    Yuv420 img;
    img.resize(cw * 2, chh * 2);
    for (size_t i = 0; i < cols.size(); i++) {
        int cx = int(i % cw), cy = int(i / cw), y, u, v;
        toYuv(cols[i], y, u, v);
        for (int d = 0; d < 4; d++)
            img.y[size_t(2 * cy + d / 2) * img.w + 2 * cx + d % 2] = uint8_t(y);
        img.u[size_t(cy) * cw + cx] = uint8_t(u);
        img.v[size_t(cy) * cw + cx] = uint8_t(v);
    }
    return img;
}

std::vector<Rgb> colours()
{
    std::vector<Rgb> out;
    for (int r = 0; r < 256; r += 17)
        for (int g = 0; g < 256; g += 17)
            for (int b = 0; b < 256; b += 17)
                out.push_back({r, g, b});
    return out;
}

int diff(Rgb a, Rgb b)
{
    return std::max({std::abs(a.r - b.r), std::abs(a.g - b.g), std::abs(a.b - b.b)});
}

void neutral()
{
    Look k = lookFor("standard");
    expect(!k.on, "standard is off");
    k.on = true;
    auto cols = colours();
    Yuv420 img = blocks(cols, 64), was = img;
    lookYuv(img, k, 1);
    int worst = 0;
    for (size_t i = 0; i < img.y.size(); i++)
        worst = std::max(worst, std::abs(img.y[i] - was.y[i]));
    for (size_t i = 0; i < img.u.size(); i++)
        worst = std::max({worst, std::abs(img.u[i] - was.u[i]), std::abs(img.v[i] - was.v[i])});
    std::printf("     neutral through YUV: off by %d at most\n", worst);
    expect(worst <= 2, "a neutral look leaves the photo as it was");
}

void ids()
{
    std::set<std::string> seen;
    bool found = true, on = true;
    for (const Recipe &r : recipes()) {
        found = found && lookFor(r.id).id == r.id;
        on = on && (r.id == std::string("standard") || lookFor(r.id).on);
        seen.insert(r.id);
    }
    expect(found, "every id finds its look");
    expect(on, "every look but standard changes the photo");
    expect(seen.size() == recipes().size(), "ids are unique");
    bool grey = true;
    for (const Recipe &r : recipes())
        for (int row = 0; row < 3; row++)
            grey = grey && std::abs(r.twist[row * 3] + r.twist[row * 3 + 1] + r.twist[row * 3 + 2]) < 1e-6f;
    expect(grey, "a twist keeps greys grey");
    expect(lookFor("nonsense").id == "standard" && !lookFor("").on, "an unknown id is standard");
}

// The preview's table against the still's path, grain aside.
void agree()
{
    auto cols = colours();
    int worstAll = 0;
    for (const Recipe &r : recipes()) {
        Recipe still = r;
        still.grain = 0;
        Look k = build(still);
        std::vector<uint16_t> table;
        lookTable(k, table);
        // Colours on the 565 grid, so the table sees what the still sees.
        std::vector<Rgb> grid;
        for (Rgb c : cols) {
            uint16_t px = pack565(c.r, c.g, c.b);
            int r5 = px >> 11, g6 = (px >> 5) & 63, b5 = px & 31;
            grid.push_back({(r5 << 3) | (r5 >> 2), (g6 << 2) | (g6 >> 4), (b5 << 3) | (b5 >> 2)});
        }
        Yuv420 img = blocks(grid, 64);
        lookYuv(img, k, 1);
        int worst = 0;
        for (size_t i = 0; i < grid.size(); i++) {
            int cx = int(i % 64), cy = int(i / 64);
            // Only colours the camera can give: a YUV round trip keeps them.
            int y, u, v;
            toYuv(grid[i], y, u, v);
            if (diff(toRgb(y, u, v), grid[i]) > 2)
                continue;
            Rgb still8 = toRgb(img.y[size_t(2 * cy) * img.w + 2 * cx], img.u[size_t(cy) * 64 + cx],
                               img.v[size_t(cy) * 64 + cx]);
            uint16_t px = pack565(grid[i].r, grid[i].g, grid[i].b);
            uint16_t out = k.on ? table[px] : px;
            int r5 = out >> 11, g6 = (out >> 5) & 63, b5 = out & 31;
            Rgb pv{(r5 << 3) | (r5 >> 2), (g6 << 2) | (g6 >> 4), (b5 << 3) | (b5 >> 2)};
            worst = std::max(worst, diff(still8, pv));
        }
        std::printf("     %s: preview off the still by %d at most\n", r.id, worst);
        worstAll = std::max(worstAll, worst);
    }
    // 565 keeps 5 bits of red and blue: 4 either way, and the YUV's rounding.
    expect(worstAll <= 8, "the preview shows what the still gets");
}

void odd()
{
    bool ok = true;
    for (auto [w, h] : {std::pair{1, 1}, {3, 3}, {5, 2}, {2, 7}, {641, 129}}) {
        Yuv420 img;
        img.resize(w, h);
        size_t ys = img.y.size(), cs = img.u.size();
        for (const Recipe &r : recipes())
            lookYuv(img, build(r), 7);
        ok = ok && img.y.size() == ys && img.u.size() == cs && img.v.size() == cs;
    }
    expect(ok, "odd sizes stay inside the picture");
}

void grain()
{
    Yuv420 img;
    img.resize(256, 256);
    std::memset(img.y.data(), 128, img.y.size());
    Recipe r;
    r.grain = 2;
    Look k = build(r);
    lookYuv(img, k, 3);
    long sum = 0;
    std::set<int> values;
    for (uint8_t y : img.y) {
        sum += y;
        values.insert(y);
    }
    double mean = double(sum) / img.y.size();
    std::printf("     grain on grey: mean %.2f, %zu levels\n", mean, values.size());
    expect(std::abs(mean - 128) < 1 && values.size() > 8, "grain adds texture, not light");
    Yuv420 again;
    again.resize(256, 256);
    std::memset(again.y.data(), 128, again.y.size());
    lookYuv(again, k, 3);
    expect(again.y == img.y, "grain is the same on every thread");
}

} // namespace

int main()
{
    neutral();
    ids();
    agree();
    odd();
    grain();
    return failed ? 1 : 0;
}
