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

} // namespace gh0st
