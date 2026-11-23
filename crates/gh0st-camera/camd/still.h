// Stills: JPEG with EXIF from a YUV420 picture, and small RGB565 pictures
// of a saved JPEG for the app.
#pragma once

#include "camd.h"

#include <string>
#include <vector>

namespace camd {

// Writes `img` as a JPEG to `path`: a .part file, synced, then renamed.
bool saveJpeg(const Yuv420 &img, const ShotMeta &meta, int quality, const std::string &path,
              std::string &why);

// A box-filtered copy of `img` at w x h.
Yuv420 shrink(const Yuv420 &img, int w, int h);

// The JPEG at `path` in exactly w x h RGB565, black around it: fitted at
// zoom 1, or `zoom` times closer around (cx, cy), each 0 to 1 across the
// picture, up to one picture pixel per screen pixel.
bool jpegTo565(const std::string &path, int w, int h, float zoom, float cx, float cy,
               std::vector<uint16_t> &out);

// A view's pixels as a full-range YUV420 picture: a video's poster.
Yuv420 viewToYuv(const View &v);

// Draws any preview view into w x h RGB565: a straight copy when the sizes
// match, nearest-neighbour otherwise.
void viewTo565(const View &v, int w, int h, uint16_t *out);

// EXIF in an APP1 payload ("Exif\0\0" and a TIFF), with `thumb` as its
// JPEG thumbnail.
std::vector<uint8_t> exifBlock(const Yuv420 &img, const ShotMeta &meta,
                               const std::vector<uint8_t> &thumb);

} // namespace camd
