// Pictures as the helpers pass them around: YUV 4:2:0 planes, a view of
// someone else's pixels, and RGB565 packing. Shared by gh0st-camd and
// gh0st-mediad.
#pragma once

#include <cstdint>
#include <vector>

namespace gh0st {

// A full-range 4:2:0 picture with packed planes.
struct Yuv420 {
    int w = 0, h = 0;
    std::vector<uint8_t> y, u, v;
    int cw() const { return (w + 1) / 2; }
    int ch() const { return (h + 1) / 2; }
    void resize(int nw, int nh)
    {
        w = nw;
        h = nh;
        y.assign(size_t(w) * h, 0);
        u.assign(size_t(cw()) * ch(), 128);
        v.assign(size_t(cw()) * ch(), 128);
    }
};

// One frame as its source has it: RGB565 rows or YUV420 planes.
struct View {
    enum Fmt { RGB565, YUV420 } fmt;
    int w, h;
    const uint8_t *p[3];
    int stride[3];
    // YUV in 16 to 235 (video's BT.709) rather than 0 to 255 (sYCC).
    bool limited = false;
};

inline uint16_t pack565(int r, int g, int b)
{
    return uint16_t(((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3));
}

inline uint8_t clamp8(int v)
{
    return uint8_t(v < 0 ? 0 : v > 255 ? 255 : v);
}

// Full-range BT.601, as libcamera's sYCC.
inline uint16_t yuv565(int y, int u, int v)
{
    u -= 128;
    v -= 128;
    return pack565(clamp8(y + ((359 * v) >> 8)), clamp8(y - ((88 * u + 183 * v) >> 8)),
                   clamp8(y + ((454 * u) >> 8)));
}

// Limited-range BT.709, as the ISP gives video.
inline uint16_t yuv565Video(int y, int u, int v)
{
    y = (y - 16) * 298;
    u -= 128;
    v -= 128;
    return pack565(clamp8((y + 459 * v) >> 8), clamp8((y - 55 * u - 136 * v) >> 8),
                   clamp8((y + 541 * u) >> 8));
}

} // namespace gh0st
