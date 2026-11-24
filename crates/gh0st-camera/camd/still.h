// Stills: JPEG with EXIF from a YUV420 picture. Reading one back for the
// screen is decode.h's.
#pragma once

#include "camd.h"
#include "decode.h"

#include <string>
#include <vector>

namespace camd {

// Writes `img` as a JPEG to `path`: a .part file, synced, then renamed.
bool saveJpeg(const Yuv420 &img, const ShotMeta &meta, int quality, const std::string &path,
              std::string &why);

// A box-filtered copy of `img` at w x h.
Yuv420 shrink(const Yuv420 &img, int w, int h);

// A view's pixels as a full-range YUV420 picture: a video's poster.
Yuv420 viewToYuv(const View &v);

// EXIF in an APP1 payload ("Exif\0\0" and a TIFF), with `thumb` as its
// JPEG thumbnail.
std::vector<uint8_t> exifBlock(const Yuv420 &img, const ShotMeta &meta,
                               const std::vector<uint8_t> &thumb);

} // namespace camd
