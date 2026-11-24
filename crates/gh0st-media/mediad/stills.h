// What a file is, and pictures of it for the screen: a view at any zoom, a
// tile, a video's frame, a song's cover. Every decoder the media app uses
// runs here, in the helper, never in the app.
#pragma once

#include "pix.h"

#include <cstdint>
#include <string>

namespace mediad {

using gh0st::Pic;

enum class Kind { Image, Anim, Video, Audio };
const char *kindWord(Kind k);

// Why a file gave no picture: the word the app gets.
enum class Fail { None, Unsupported, TooBig, Corrupt, Io, NoMem };
const char *failWord(Fail f);

// How a still is decoded.
enum class Codec { Jpeg, Heif, Raw, WebpAnim, Av };

struct Info {
    Kind kind = Kind::Image;
    Codec codec = Codec::Av;
    // As shown: after its orientation or rotation. A song's are its cover's.
    int w = 0, h = 0;
    // EXIF orientation, 1 upright. A video's rotation comes as one too.
    int orient = 1;
    int64_t durMs = 0;
    // Sound and subtitle tracks, a .srt beside the file counted.
    int audio = 0, subs = 0;
    std::string format, vcodec, acodec;
};

struct Tags {
    std::string title, artist, album, track, date;
};

// A picture other than JPEG past this is refused before it is decoded.
constexpr double MAX_PIXELS = 24e6;
// A JPEG decodes at the size it is shown, so it may be far larger.
constexpr double MAX_JPEG_PIXELS = 256e6;
// What is kept of a picture between views: pan and zoom cost no decode.
constexpr double BASE_PIXELS = 8e6;

class Stills {
public:
    Fail probe(const std::string &path, Info &out);
    Fail tags(const std::string &path, Tags &out);
    // The file as a w x h screen shows it at `zoom` (1: fitted whole)
    // around (cx, cy), 0 to 1 across the picture: at most w x h, smaller
    // when the picture is.
    Fail view(const std::string &path, int w, int h, float zoom, float cx, float cy, Pic &out);
    // A tile: the picture's middle, cut to fill exactly w x h.
    Fail thumb(const std::string &path, int w, int h, Pic &out);
    // Drops the kept picture.
    void forget();

private:
    // probe, kept for the last file asked about until it changes.
    Fail info(const std::string &path, Info &out);
    Fail base(const std::string &path, const Info &info);
    Fail render(const std::string &path, const Info &info, int w, int h, float zoom, float cx,
                float cy, bool fill, Pic &out);

    // The last picture decoded whole, upright, at most BASE_PIXELS, and
    // the file it came from.
    std::string basePath_;
    int64_t baseStamp_ = 0;
    Pic base_;
    std::string infoPath_;
    int64_t infoStamp_ = 0;
    Info info_;
    Fail infoFail_ = Fail::None;
};

// Where a picture's rectangle and its size on screen come from: the part
// of a W x H picture a w x h screen shows at `zoom` around (cx, cy). `fill`
// covers the screen and cuts the picture's edges, and may enlarge it.
struct Frame {
    double x, y, w, h;
    int ow, oh;
};
Frame frameFor(double W, double H, int w, int h, float zoom, float cx, float cy, bool fill);

// The rectangle (x, y, w, h) of `src` box-filtered into ow x oh.
Pic sample(const Pic &src, double x, double y, double w, double h, int ow, int oh);

} // namespace mediad
