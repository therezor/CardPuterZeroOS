// Film looks for photos. One function, a colour matrix and then three
// curves, makes the still and the preview, so the viewfinder shows the
// photo you get. Grain is the still's alone. docs/camera.md, Looks.
#pragma once

#include "pix.h"

#include <cstdint>
#include <string>
#include <vector>

namespace camd {

using namespace gh0st;

struct Look {
    std::string id = "standard";
    // Off, the picture is left as it is.
    bool on = false;
    // RGB to RGB, 4096 is 1.
    int32_t m[9] = {4096, 0, 0, 0, 4096, 0, 0, 0, 4096};
    uint8_t curve[3][256];
    // Grain on the still: 0 none, 1 fine, 2 strong.
    int grain = 0;
};

// What makes a look, from neutral (every field as it starts) to a film.
struct Recipe {
    const char *id = "standard";
    // 1 is the camera's colour; 0 is grey.
    float sat = 1;
    // An S curve: more is harder, less is flatter.
    float contrast = 0;
    // More lifts the shadows, less deepens them.
    float shadows = 0;
    // More brightens the highlights, less holds them back.
    float highlights = 0;
    // Where black and white land, 0 to 1.
    float lift = 0, cap = 1;
    // White balance: more warm is amber, more tint is magenta.
    float warm = 0, tint = 0;
    // Added to each channel in the shadows and in the highlights.
    float lowTint[3] = {0, 0, 0};
    float highTint[3] = {0, 0, 0};
    // Black and white through these channel weights, when any is set.
    float mono[3] = {0, 0, 0};
    // Added to the identity before saturation: colours bend. Each row sums
    // to 0, so greys stay grey.
    float twist[9] = {0, 0, 0, 0, 0, 0, 0, 0, 0};
    int grain = 0;
};

// Every look, `standard` first.
const std::vector<Recipe> &recipes();
Look build(const Recipe &r);
// The look an id names; an unknown id is `standard`.
Look lookFor(const std::string &id);

// One pixel, 8-bit sRGB in and out.
inline void lookRgb(const Look &k, int &r, int &g, int &b)
{
    int r2 = (k.m[0] * r + k.m[1] * g + k.m[2] * b + 2048) >> 12;
    int g2 = (k.m[3] * r + k.m[4] * g + k.m[5] * b + 2048) >> 12;
    int b2 = (k.m[6] * r + k.m[7] * g + k.m[8] * b + 2048) >> 12;
    r = k.curve[0][clamp8(r2)];
    g = k.curve[1][clamp8(g2)];
    b = k.curve[2][clamp8(b2)];
}

// The still, in place: full-range BT.601 YUV420, as the ISP gives it.
// `seed` varies the grain from shot to shot.
void lookYuv(Yuv420 &img, const Look &k, uint32_t seed);
// RGB565 to RGB565 for the preview: 65,536 entries.
void lookTable(const Look &k, std::vector<uint16_t> &table);

} // namespace camd
