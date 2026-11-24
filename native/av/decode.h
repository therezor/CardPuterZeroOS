// Decoding for the screen: a JPEG straight to RGB565 at the size it is
// shown, any frame scaled into a w x h box, and the box filter both use.
#pragma once

#include "pix.h"

#include <csetjmp>
#include <cstdio>
#include <string>
#include <vector>

#include <jpeglib.h>

namespace gh0st {

// libjpeg's errors end in a longjmp back to the caller, not an exit.
struct JpegErr {
    jpeg_error_mgr pub;
    jmp_buf jb;
};
void onJpegError(j_common_ptr c);

// Box filter of one plane (`ch` interleaved channels) into dw x dh.
void box(const uint8_t *src, int sw, int sh, int sstride, int ch, uint8_t *dst, int dw, int dh,
         int dstride);

// The JPEG at `path` in exactly w x h RGB565, black around it: fitted at
// zoom 1, or `zoom` times closer around (cx, cy), each 0 to 1 across the
// picture, up to one picture pixel per screen pixel.
bool jpegTo565(const std::string &path, int w, int h, float zoom, float cx, float cy,
               std::vector<uint16_t> &out);

// Draws any view into w x h RGB565: a straight copy when the sizes match,
// nearest-neighbour otherwise.
void viewTo565(const View &v, int w, int h, uint16_t *out);

// A JPEG's size and EXIF orientation (1 when it has none), from its
// headers. Exactly one of `f` and `mem` is set.
bool jpegHeader(FILE *f, const std::vector<uint8_t> *mem, int &w, int &h, int &orient);

// The rectangle (sx, sy, sw, sh) of a JPEG, in its own pixels, decoded at
// the scale that makes it ow x oh: rows above are skipped, columns cropped
// and the DCT scaled, so only what is shown is decoded. False past
// `maxPixels` or on a broken file.
bool jpegRegion(FILE *f, const std::vector<uint8_t> *mem, double sx, double sy, double sw,
                double sh, int ow, int oh, double maxPixels, Pic &out);

// `src` turned and mirrored as EXIF orientation `orient` says, so it shows
// upright.
Pic orient(const Pic &src, int orient);

// Where a rectangle of the upright picture (W x H as shown) lies in the
// stored one, for orientation `orient`.
void unorientRect(int orient, double W, double H, double &x, double &y, double &w, double &h);

} // namespace gh0st
