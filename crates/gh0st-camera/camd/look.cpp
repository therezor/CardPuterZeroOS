#include "look.h"

#include <algorithm>
#include <cmath>
#include <system_error>
#include <thread>

namespace camd {

namespace {

// Each film is a starting point by eye; docs/camera.md says what each is for.
std::vector<Recipe> makeRecipes()
{
    std::vector<Recipe> all;
    all.push_back(Recipe{});

    Recipe r;
    r.id = "slide";
    r.sat = 0.72f;
    r.contrast = 0.25f;
    r.shadows = -0.08f;
    r.highlights = -0.1f;
    r.warm = -0.05f;
    r.lowTint[1] = 0.01f;
    r.lowTint[2] = 0.02f;
    // Blues go deeper, reds go quiet. Each row sums to 0: greys stay grey.
    r.twist[0] = -0.06f;
    r.twist[1] = 0.06f;
    r.twist[7] = 0.1f;
    r.twist[8] = -0.1f;
    r.grain = 1;
    all.push_back(r);

    r = Recipe{};
    r.id = "neg";
    r.sat = 0.75f;
    r.contrast = 0.35f;
    r.shadows = -0.05f;
    r.highlights = -0.2f;
    r.lift = 0.025f;
    r.cap = 0.98f;
    r.warm = 0.05f;
    r.lowTint[0] = -0.01f;
    r.lowTint[1] = 0.025f;
    r.lowTint[2] = 0.035f;
    r.highTint[0] = 0.03f;
    r.highTint[1] = 0.01f;
    r.highTint[2] = -0.03f;
    // Greens lean teal, reds lean orange.
    r.twist[0] = -0.05f;
    r.twist[1] = 0.05f;
    r.twist[4] = -0.05f;
    r.twist[5] = 0.05f;
    r.grain = 1;
    all.push_back(r);

    r = Recipe{};
    r.id = "vivid";
    r.sat = 1.4f;
    r.contrast = 0.25f;
    r.warm = 0.05f;
    all.push_back(r);

    r = Recipe{};
    r.id = "soft";
    r.sat = 0.9f;
    r.contrast = -0.15f;
    r.shadows = 0.15f;
    r.highlights = -0.1f;
    r.lift = 0.02f;
    r.warm = 0.12f;
    r.highTint[0] = 0.015f;
    r.highTint[1] = 0.005f;
    r.highTint[2] = -0.01f;
    all.push_back(r);

    r = Recipe{};
    r.id = "cine";
    r.sat = 0.6f;
    r.contrast = -0.35f;
    r.shadows = 0.2f;
    r.highlights = -0.25f;
    r.lift = 0.05f;
    r.cap = 0.95f;
    r.warm = -0.05f;
    r.lowTint[0] = -0.01f;
    r.lowTint[1] = 0.01f;
    r.lowTint[2] = 0.02f;
    r.highTint[0] = 0.01f;
    r.highTint[2] = -0.01f;
    all.push_back(r);

    r = Recipe{};
    r.id = "amber";
    r.sat = 0.95f;
    r.contrast = 0.05f;
    r.shadows = 0.12f;
    r.highlights = -0.15f;
    r.lift = 0.03f;
    r.warm = 0.4f;
    r.lowTint[0] = 0.015f;
    r.lowTint[1] = 0.005f;
    r.lowTint[2] = -0.005f;
    r.highTint[0] = 0.05f;
    r.highTint[1] = 0.02f;
    r.highTint[2] = -0.05f;
    r.grain = 1;
    all.push_back(r);

    r = Recipe{};
    r.id = "mono";
    // Through a yellow filter: skies go darker.
    r.mono[0] = 0.35f;
    r.mono[1] = 0.55f;
    r.mono[2] = 0.10f;
    r.contrast = 0.3f;
    r.shadows = -0.05f;
    r.highlights = 0.05f;
    r.grain = 2;
    all.push_back(r);

    r = Recipe{};
    r.id = "faded";
    r.sat = 0.7f;
    r.contrast = -0.1f;
    r.shadows = 0.25f;
    r.highlights = -0.2f;
    r.lift = 0.09f;
    r.cap = 0.92f;
    r.warm = 0.05f;
    r.tint = 0.05f;
    r.lowTint[0] = 0.03f;
    r.lowTint[1] = -0.01f;
    r.lowTint[2] = 0.03f;
    r.highTint[1] = 0.02f;
    r.highTint[2] = -0.02f;
    r.grain = 2;
    all.push_back(r);
    return all;
}

void mul(const float a[9], const float b[9], float out[9])
{
    for (int i = 0; i < 3; i++)
        for (int j = 0; j < 3; j++)
            out[i * 3 + j] = a[i * 3] * b[j] + a[i * 3 + 1] * b[3 + j] + a[i * 3 + 2] * b[6 + j];
}

// Luma weights, BT.601, as the still's YUV.
const float LR = 0.299f, LG = 0.587f, LB = 0.114f;

float curveAt(const Recipe &r, int c, float x)
{
    auto unit = [](float v) { return std::min(1.0f, std::max(0.0f, v)); };
    float s = x * x * (3 - 2 * x);
    float y = unit(x + r.contrast * (s - x));
    y = unit(y + r.shadows * 2 * y * (1 - y) * (1 - y));
    y = unit(y + r.highlights * 2 * y * y * (1 - y));
    y = unit(y + r.lowTint[c] * (1 - y) * (1 - y) + r.highTint[c] * y * y);
    return unit(r.lift + y * (r.cap - r.lift));
}

// A hash of a row: where its noise starts, the same on every thread.
inline uint32_t rowSeed(uint32_t y, uint32_t seed)
{
    uint32_t h = (y * 0x85EBCA77u) ^ seed;
    h ^= h >> 16;
    h *= 0x7FEB352Du;
    h ^= h >> 15;
    h *= 0x846CA68Bu;
    h ^= h >> 16;
    return h | 1;
}

// Noise along a row: xorshift, no multiply.
struct Noise {
    uint32_t s;
    // Two bytes summed: about bell-shaped, -255 to 255.
    int next()
    {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        return int(s & 255) + int((s >> 8) & 255) - 255;
    }
};

// Rows of 2x2 blocks [c0, c1), with grain G. Raw pointers: a byte store
// may alias anything, so through the vectors every pixel would reload them.
template <int G>
void band(Yuv420 &img, const Look &k, uint32_t seed, int c0, int c1)
{
    const int w = img.w, h = img.h, cw = img.cw();
    uint8_t *const Y = img.y.data();
    uint8_t *const U = img.u.data();
    uint8_t *const V = img.v.data();
    const uint8_t *const cr = k.curve[0];
    const uint8_t *const cg = k.curve[1];
    const uint8_t *const cb = k.curve[2];
    // Grain is strongest in the midtones, as film's is.
    const int amp = G == 2 ? 22 : 10;
    // YUV to RGB and the matrix in one step: each channel is a multiple of
    // Y plus what the block's U and V add, so a pixel costs three products.
    int32_t m[9];
    for (int i = 0; i < 9; i++)
        m[i] = k.m[i];
    const int32_t s0 = m[0] + m[1] + m[2], s1 = m[3] + m[4] + m[5], s2 = m[6] + m[7] + m[8];
    for (int cy = c0; cy < c1; cy++) {
        const int y0 = 2 * cy;
        const bool two = y0 + 1 < h;
        Noise n0{rowSeed(y0, seed)}, n1{rowSeed(y0 + 1, seed)}, nb{rowSeed(cy, ~seed)};
        uint8_t *const row0 = Y + size_t(y0) * w;
        uint8_t *const row1 = two ? row0 + w : row0;
        for (int cx = 0; cx < cw; cx++) {
            const size_t ci = size_t(cy) * cw + cx;
            const int u = U[ci] - 128, v = V[ci] - 128;
            const int rv = (91881 * v + 32768) >> 16;
            const int guv = (22554 * u + 46802 * v + 32768) >> 16;
            const int bu = (116130 * u + 32768) >> 16;
            const int32_t k0 = m[0] * rv - m[1] * guv + m[2] * bu + 2048;
            const int32_t k1 = m[3] * rv - m[4] * guv + m[5] * bu + 2048;
            const int32_t k2 = m[6] * rv - m[7] * guv + m[8] * bu + 2048;
            const int block = G == 2 ? nb.next() : 0;
            int sr = 0, sg = 0, sb = 0;
            auto one = [&](uint8_t *p, Noise &n) __attribute__((always_inline)) {
                const int py = *p;
                const int r = cr[clamp8((s0 * py + k0) >> 12)];
                const int g = cg[clamp8((s1 * py + k1) >> 12)];
                const int b = cb[clamp8((s2 * py + k2) >> 12)];
                int ny = (19595 * r + 38470 * g + 7471 * b + 32768) >> 16;
                if (G) {
                    int nz = n.next();
                    if (G == 2)
                        nz = (3 * block + nz) >> 2;
                    ny += (nz * amp * ((ny * (255 - ny)) >> 6)) >> 16;
                }
                *p = clamp8(ny);
                sr += r;
                sg += g;
                sb += b;
            };
            const int x0 = 2 * cx;
            const bool wide = x0 + 1 < w;
            one(row0 + x0, n0);
            if (wide)
                one(row0 + x0 + 1, n0);
            if (two) {
                one(row1 + x0, n1);
                if (wide)
                    one(row1 + x0 + 1, n1);
            }
            int cu = -11058 * sr - 21710 * sg + 32768 * sb;
            int cv = 32768 * sr - 27439 * sg - 5329 * sb;
            if (wide && two) {
                cu >>= 2;
                cv >>= 2;
            } else {
                const int n = (1 + wide) * (1 + two);
                cu /= n;
                cv /= n;
            }
            U[ci] = clamp8(((cu + 32768) >> 16) + 128);
            V[ci] = clamp8(((cv + 32768) >> 16) + 128);
        }
    }
}

void bandFor(Yuv420 &img, const Look &k, uint32_t seed, int c0, int c1)
{
    if (k.grain == 2)
        band<2>(img, k, seed, c0, c1);
    else if (k.grain)
        band<1>(img, k, seed, c0, c1);
    else
        band<0>(img, k, seed, c0, c1);
}

} // namespace

const std::vector<Recipe> &recipes()
{
    static const std::vector<Recipe> all = makeRecipes();
    return all;
}

Look build(const Recipe &r)
{
    Look k;
    k.id = r.id;
    k.grain = r.grain;
    float wb[9] = {1 + 0.08f * r.warm, 0, 0, 0, 1 - 0.05f * r.tint, 0, 0, 0, 1 - 0.08f * r.warm};
    float tw[9];
    for (int i = 0; i < 9; i++)
        tw[i] = r.twist[i] + (i % 4 == 0 ? 1 : 0);
    float mix[9];
    float total = r.mono[0] + r.mono[1] + r.mono[2];
    if (total > 0) {
        for (int i = 0; i < 3; i++)
            for (int j = 0; j < 3; j++)
                mix[i * 3 + j] = r.mono[j] / total;
    } else {
        const float l[3] = {LR, LG, LB};
        for (int i = 0; i < 3; i++)
            for (int j = 0; j < 3; j++)
                mix[i * 3 + j] = (1 - r.sat) * l[j] + (i == j ? r.sat : 0);
    }
    float a[9], m[9];
    mul(tw, wb, a);
    mul(mix, a, m);
    for (int i = 0; i < 9; i++)
        k.m[i] = int32_t(std::lround(m[i] * 4096));
    bool flat = true;
    for (int c = 0; c < 3; c++)
        for (int i = 0; i < 256; i++) {
            k.curve[c][i] = uint8_t(std::lround(curveAt(r, c, i / 255.0f) * 255));
            flat = flat && k.curve[c][i] == i;
        }
    for (int i = 0; i < 9; i++)
        flat = flat && k.m[i] == (i % 4 == 0 ? 4096 : 0);
    k.on = !flat || k.grain;
    return k;
}

Look lookFor(const std::string &id)
{
    for (const Recipe &r : recipes())
        if (id == r.id)
            return build(r);
    return build(recipes()[0]);
}

void lookYuv(Yuv420 &img, const Look &k, uint32_t seed)
{
    if (!k.on || img.w <= 0 || img.h <= 0)
        return;
    const int rows = img.ch();
    // An 8 MP photo takes the cores in bands; the threads end with it.
    int n = rows >= 64 ? int(std::min(4u, std::max(1u, std::thread::hardware_concurrency()))) : 1;
    std::vector<std::thread> more;
    // Bands from `mine` on run here: all of them past a thread that
    // couldn't start.
    int mine = n;
    for (int i = 1; i < n && mine == n; i++) {
        try {
            more.emplace_back(bandFor, std::ref(img), std::cref(k), seed, rows * i / n,
                              rows * (i + 1) / n);
        } catch (const std::system_error &) {
            mine = i;
        }
    }
    bandFor(img, k, seed, 0, rows / n);
    for (int i = mine; i < n; i++)
        bandFor(img, k, seed, rows * i / n, rows * (i + 1) / n);
    for (auto &t : more)
        t.join();
}

void lookTable(const Look &k, std::vector<uint16_t> &table)
{
    if (!k.on) {
        std::vector<uint16_t>().swap(table);
        return;
    }
    table.resize(65536);
    for (uint32_t px = 0; px < 65536; px++) {
        int r5 = px >> 11, g6 = (px >> 5) & 63, b5 = px & 31;
        int r = (r5 << 3) | (r5 >> 2), g = (g6 << 2) | (g6 >> 4), b = (b5 << 3) | (b5 >> 2);
        lookRgb(k, r, g, b);
        table[px] = uint16_t(((r * 31 + 127) / 255) << 11 | ((g * 63 + 127) / 255) << 5 |
                             ((b * 31 + 127) / 255));
    }
}

} // namespace camd
