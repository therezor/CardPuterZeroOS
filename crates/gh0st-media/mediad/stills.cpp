#include "stills.h"
#include "decode.h"

#include <algorithm>
#include <cerrno>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <memory>
#include <sys/stat.h>
#include <vector>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/display.h>
#include <libavutil/opt.h>
#include <libswscale/swscale.h>
}
#include <libheif/heif.h>
#include <libraw/libraw.h>
#include <webp/decode.h>
#include <webp/demux.h>

namespace mediad {

const char *kindWord(Kind k)
{
    switch (k) {
    case Kind::Image: return "image";
    case Kind::Anim: return "anim";
    case Kind::Video: return "video";
    case Kind::Audio: return "audio";
    }
    return "image";
}

const char *failWord(Fail f)
{
    switch (f) {
    case Fail::None: return "ok";
    case Fail::Unsupported: return "unsupported";
    case Fail::TooBig: return "toobig";
    case Fail::Corrupt: return "corrupt";
    case Fail::Io: return "io";
    case Fail::NoMem: return "nomem";
    }
    return "corrupt";
}

namespace {

Fail fromErrno(int e)
{
    return e == ENOMEM ? Fail::NoMem : Fail::Io;
}

Fail fromAv(int r)
{
    if (r == AVERROR(ENOMEM))
        return Fail::NoMem;
    if (r == AVERROR(ENOENT) || r == AVERROR(EACCES) || r == AVERROR(EIO) || r == AVERROR(EISDIR))
        return Fail::Io;
    return Fail::Corrupt;
}

// Changes when the file does: its time and size.
int64_t stampOf(const std::string &path)
{
    struct stat st;
    if (stat(path.c_str(), &st) != 0)
        return 0;
    return int64_t(st.st_mtim.tv_sec) * 1000000007 + st.st_mtim.tv_nsec + st.st_size * 31;
}

bool head(const std::string &path, uint8_t *buf, size_t n, size_t &got, int &err)
{
    FILE *f = fopen(path.c_str(), "rb");
    if (!f) {
        err = errno;
        return false;
    }
    got = fread(buf, 1, n, f);
    fclose(f);
    return true;
}

bool isJpeg(const uint8_t *h, size_t n)
{
    return n >= 3 && h[0] == 0xff && h[1] == 0xd8 && h[2] == 0xff;
}

bool isHeif(const uint8_t *h, size_t n)
{
    if (n < 12 || memcmp(h + 4, "ftyp", 4) != 0)
        return false;
    static const char *brands[] = {"heic", "heix", "hevc", "hevx", "heim", "heis",
                                   "mif1", "msf1", "avif", "avis"};
    for (auto b : brands)
        if (memcmp(h + 8, b, 4) == 0)
            return true;
    return false;
}

bool isWebp(const uint8_t *h, size_t n)
{
    return n >= 12 && memcmp(h, "RIFF", 4) == 0 && memcmp(h + 8, "WEBP", 4) == 0;
}

// The size a PNG, GIF or BMP claims, read before any decoder is trusted
// with it: a 40000x40000 PNG packs into kilobytes.
bool claimedSize(const uint8_t *h, size_t n, int64_t &w, int64_t &hh)
{
    auto be32 = [&](size_t o) { return int64_t(h[o]) << 24 | h[o + 1] << 16 | h[o + 2] << 8 | h[o + 3]; };
    auto le16 = [&](size_t o) { return int64_t(h[o]) | int64_t(h[o + 1]) << 8; };
    auto le32 = [&](size_t o) { return le16(o) | le16(o + 2) << 16; };
    if (n >= 24 && memcmp(h, "\x89PNG\r\n\x1a\n", 8) == 0 && memcmp(h + 12, "IHDR", 4) == 0) {
        w = be32(16);
        hh = be32(20);
        return true;
    }
    if (n >= 10 && (memcmp(h, "GIF87a", 6) == 0 || memcmp(h, "GIF89a", 6) == 0)) {
        w = le16(6);
        hh = le16(8);
        return true;
    }
    if (n >= 26 && h[0] == 'B' && h[1] == 'M') {
        w = le32(18);
        hh = std::abs(int32_t(le32(22)));
        return true;
    }
    if (isWebp(h, n)) {
        int ww = 0, wh = 0;
        if (WebPGetInfo(h, n, &ww, &wh)) {
            w = ww;
            hh = wh;
            return true;
        }
    }
    return false;
}

bool rawName(const std::string &path)
{
    static const char *exts[] = {"dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2",
                                 "raf", "orf", "rw2", "pef", "srw", "3fr", "erf", "kdc",
                                 "mos", "mrw", "x3f", "iiq", "rwl"};
    size_t dot = path.find_last_of('.');
    if (dot == std::string::npos)
        return false;
    std::string e = path.substr(dot + 1);
    for (auto &c : e)
        c = char(tolower(c));
    for (auto x : exts)
        if (e == x)
            return true;
    return false;
}

// LibRaw's flip: 3 is half round, 5 a quarter left, 6 a quarter right.
int orientOfFlip(int flip)
{
    switch (flip) {
    case 3: return 3;
    case 5: return 8;
    case 6: return 6;
    default: return 1;
    }
}

// A stream's display matrix as an EXIF orientation.
int orientOfStream(const AVStream *st)
{
    const AVPacketSideData *sd =
        av_packet_side_data_get(st->codecpar->coded_side_data, st->codecpar->nb_coded_side_data,
                                AV_PKT_DATA_DISPLAYMATRIX);
    if (!sd || sd->size < 9 * 4)
        return 1;
    double a = av_display_rotation_get(reinterpret_cast<const int32_t *>(sd->data));
    if (std::isnan(a))
        return 1;
    int deg = (int(std::lround(-a)) % 360 + 360) % 360;
    switch (deg) {
    case 90: return 6;
    case 180: return 3;
    case 270: return 8;
    default: return 1;
    }
}

// A picture of any pixel format, kept at most BASE_PIXELS, over black where
// it is clear, then turned upright.
Fail packBase(const uint8_t *const data[4], const int ls[4], AVPixelFormat fmt, int w, int h,
              int orient, Pic &out)
{
    if (w <= 0 || h <= 0)
        return Fail::Corrupt;
    double k = std::min(1.0, std::sqrt(BASE_PIXELS / (double(w) * h)));
    int bw = std::max(1, int(w * k)), bh = std::max(1, int(h * k));
    SwsContext *sws = sws_getContext(w, h, fmt, bw, bh, AV_PIX_FMT_RGBA,
                                     k < 1 ? SWS_AREA : SWS_POINT, nullptr, nullptr, nullptr);
    if (!sws)
        return Fail::Unsupported;
    std::vector<uint8_t> rgba;
    try {
        rgba.resize(size_t(bw) * bh * 4);
    } catch (const std::bad_alloc &) {
        sws_freeContext(sws);
        return Fail::NoMem;
    }
    uint8_t *dst[4] = {rgba.data(), nullptr, nullptr, nullptr};
    int dls[4] = {bw * 4, 0, 0, 0};
    sws_scale(sws, data, ls, 0, h, dst, dls);
    sws_freeContext(sws);
    Pic p;
    p.w = bw;
    p.h = bh;
    p.px.resize(size_t(bw) * bh);
    for (size_t i = 0; i < p.px.size(); i++) {
        const uint8_t *q = &rgba[i * 4];
        int a = q[3];
        p.px[i] = gh0st::pack565(q[0] * a / 255, q[1] * a / 255, q[2] * a / 255);
    }
    out = gh0st::orient(p, orient);
    return Fail::None;
}

// libavformat with each stream's decoder held to MAX_PIXELS, so a file
// that claims to be huge is refused while it is probed.
struct Av {
    AVFormatContext *fmt = nullptr;
    ~Av() { avformat_close_input(&fmt); }

    int open(const std::string &path)
    {
        int r = avformat_open_input(&fmt, path.c_str(), nullptr, nullptr);
        if (r < 0)
            return r;
        std::vector<AVDictionary *> opts(fmt->nb_streams, nullptr);
        for (auto &o : opts)
            av_dict_set_int(&o, "max_pixels", int64_t(MAX_PIXELS), 0);
        r = avformat_find_stream_info(fmt, opts.data());
        for (auto &o : opts)
            av_dict_free(&o);
        return r;
    }
};

// The first picture of `st` from `at` ms on, or its attached cover.
Fail decodeFrame(AVFormatContext *fmt, AVStream *st, int64_t atMs, AVFrame *f)
{
    const AVCodec *c = avcodec_find_decoder(st->codecpar->codec_id);
    if (!c)
        return Fail::Unsupported;
    AVCodecContext *d = avcodec_alloc_context3(c);
    if (!d)
        return Fail::NoMem;
    avcodec_parameters_to_context(d, st->codecpar);
    d->pkt_timebase = st->time_base;
    av_opt_set_int(d, "max_pixels", int64_t(MAX_PIXELS), 0);
    d->thread_count = 4;
    int r = avcodec_open2(d, c, nullptr);
    Fail fail = Fail::Corrupt;
    AVPacket *p = av_packet_alloc();
    if (r >= 0 && (st->disposition & AV_DISPOSITION_ATTACHED_PIC)) {
        if (avcodec_send_packet(d, &st->attached_pic) >= 0) {
            avcodec_send_packet(d, nullptr);
            if (avcodec_receive_frame(d, f) == 0)
                fail = Fail::None;
        }
    } else if (r >= 0) {
        if (atMs > 0)
            av_seek_frame(fmt, -1, atMs * (AV_TIME_BASE / 1000), AVSEEK_FLAG_BACKWARD);
        // A frame within this many packets, or the file is no use.
        for (int n = 0; n < 600 && fail != Fail::None; n++) {
            int rr = av_read_frame(fmt, p);
            if (rr < 0) {
                avcodec_send_packet(d, nullptr);
                if (avcodec_receive_frame(d, f) == 0)
                    fail = Fail::None;
                break;
            }
            if (p->stream_index == st->index && avcodec_send_packet(d, p) >= 0 &&
                avcodec_receive_frame(d, f) == 0)
                fail = Fail::None;
            av_packet_unref(p);
        }
    } else if (r == AVERROR(ENOMEM)) {
        fail = Fail::NoMem;
    } else if (r == AVERROR(EINVAL)) {
        fail = Fail::TooBig;
    }
    av_packet_free(&p);
    avcodec_free_context(&d);
    return fail;
}

int64_t durationMs(const AVFormatContext *fmt)
{
    return fmt->duration > 0 ? fmt->duration / (AV_TIME_BASE / 1000) : 0;
}

// The cover beside a song: cover.jpg, folder.jpg, front.jpg and the like.
std::string folderCover(const std::string &path)
{
    std::string dir = path.substr(0, path.find_last_of('/') + 1);
    static const char *names[] = {"cover.jpg", "cover.png", "folder.jpg", "folder.png",
                                  "front.jpg", "front.png", "Cover.jpg", "Folder.jpg",
                                  "AlbumArt.jpg"};
    struct stat st;
    for (auto n : names)
        if (stat((dir + n).c_str(), &st) == 0 && S_ISREG(st.st_mode))
            return dir + n;
    return "";
}

std::string clean(const char *s)
{
    std::string out;
    for (; s && *s && out.size() < 120; s++)
        out += (*s == '\t' || *s == '\n' || *s == '\r') ? ' ' : *s;
    return out;
}

bool sidecarSrt(const std::string &path)
{
    size_t dot = path.find_last_of('.');
    if (dot == std::string::npos || dot < path.find_last_of('/'))
        return false;
    struct stat st;
    return stat((path.substr(0, dot) + ".srt").c_str(), &st) == 0;
}

// libheif decodes HEVC through libavcodec when its plugin is there.
heif_error heifDecode(heif_image_handle *h, heif_image **img, bool alpha)
{
    heif_decoding_options *o = heif_decoding_options_alloc();
    heif_chroma chroma = alpha ? heif_chroma_interleaved_RGBA : heif_chroma_interleaved_RGB;
    o->decoder_id = "ffmpeg";
    heif_error e = heif_decode_image(h, img, heif_colorspace_RGB, chroma, o);
    if (e.code != heif_error_Ok) {
        o->decoder_id = nullptr;
        e = heif_decode_image(h, img, heif_colorspace_RGB, chroma, o);
    }
    heif_decoding_options_free(o);
    return e;
}

std::vector<uint8_t> slurp(const std::string &path, int &err)
{
    std::vector<uint8_t> b;
    FILE *f = fopen(path.c_str(), "rb");
    if (!f) {
        err = errno;
        return b;
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    rewind(f);
    if (n > 0 && n < (256L << 20)) {
        b.resize(size_t(n));
        b.resize(fread(b.data(), 1, b.size(), f));
    }
    fclose(f);
    return b;
}

// A RAW's embedded preview, when it is a JPEG.
bool rawPreview(const std::string &path, std::vector<uint8_t> &jpg, int &orient)
{
    libraw_data_t *lr = libraw_init(0);
    if (!lr)
        return false;
    bool ok = false;
    if (libraw_open_file(lr, path.c_str()) == LIBRAW_SUCCESS &&
        libraw_unpack_thumb(lr) == LIBRAW_SUCCESS) {
        int e = 0;
        libraw_processed_image_t *t = libraw_dcraw_make_mem_thumb(lr, &e);
        if (t && t->type == LIBRAW_IMAGE_JPEG && t->data_size > 0) {
            jpg.assign(t->data, t->data + t->data_size);
            orient = orientOfFlip(lr->sizes.flip);
            ok = true;
        }
        if (t)
            libraw_dcraw_clear_mem(t);
    }
    libraw_close(lr);
    return ok;
}

} // namespace

Frame frameFor(double W, double H, int w, int h, float zoom, float cx, float cy, bool fill)
{
    Frame f;
    double scale = fill ? std::max(w / W, h / H) : std::min(w / W, h / H) * std::max(1.0f, zoom);
    if (!fill)
        scale = std::min(scale, 1.0);
    f.w = std::min(W, w / scale);
    f.h = std::min(H, h / scale);
    f.x = std::clamp(double(cx) * W - f.w / 2, 0.0, W - f.w);
    f.y = std::clamp(double(cy) * H - f.h / 2, 0.0, H - f.h);
    f.ow = std::clamp(int(std::lround(f.w * scale)), 1, w);
    f.oh = std::clamp(int(std::lround(f.h * scale)), 1, h);
    return f;
}

Pic sample(const Pic &src, double x, double y, double w, double h, int ow, int oh)
{
    Pic out;
    out.w = ow;
    out.h = oh;
    out.px.resize(size_t(ow) * oh);
    for (int oy = 0; oy < oh; oy++) {
        int y0 = std::clamp(int(y + oy * h / oh), 0, src.h - 1);
        int y1 = std::clamp(int(y + (oy + 1) * h / oh), y0 + 1, src.h);
        for (int ox = 0; ox < ow; ox++) {
            int x0 = std::clamp(int(x + ox * w / ow), 0, src.w - 1);
            int x1 = std::clamp(int(x + (ox + 1) * w / ow), x0 + 1, src.w);
            uint32_t r = 0, g = 0, b = 0, n = 0;
            for (int yy = y0; yy < y1; yy++) {
                const uint16_t *row = &src.px[size_t(yy) * src.w];
                for (int xx = x0; xx < x1; xx++) {
                    uint16_t p = row[xx];
                    r += p >> 11;
                    g += (p >> 5) & 63;
                    b += p & 31;
                    n++;
                }
            }
            out.px[size_t(oy) * ow + ox] = uint16_t((r / n) << 11 | (g / n) << 5 | (b / n));
        }
    }
    return out;
}

Fail Stills::probe(const std::string &path, Info &out)
{
    out = Info();
    uint8_t h[64];
    size_t n = 0;
    int err = 0;
    if (!head(path, h, sizeof h, n, err))
        return fromErrno(err);
    if (isJpeg(h, n)) {
        FILE *f = fopen(path.c_str(), "rb");
        if (!f)
            return fromErrno(errno);
        bool ok = gh0st::jpegHeader(f, nullptr, out.w, out.h, out.orient);
        fclose(f);
        if (!ok)
            return Fail::Corrupt;
        if (gh0st::swapsSides(out.orient))
            std::swap(out.w, out.h);
        out.codec = Codec::Jpeg;
        out.format = "jpeg";
        out.vcodec = "jpeg";
        return double(out.w) * out.h > MAX_JPEG_PIXELS ? Fail::TooBig : Fail::None;
    }
    if (isHeif(h, n)) {
        heif_context *ctx = heif_context_alloc();
        heif_image_handle *hh = nullptr;
        Fail fail = Fail::Corrupt;
        if (heif_context_read_from_file(ctx, path.c_str(), nullptr).code == heif_error_Ok &&
            heif_context_get_primary_image_handle(ctx, &hh).code == heif_error_Ok) {
            out.w = heif_image_handle_get_width(hh);
            out.h = heif_image_handle_get_height(hh);
            out.codec = Codec::Heif;
            bool avif = memcmp(h + 8, "avi", 3) == 0;
            out.format = avif ? "avif" : "heif";
            out.vcodec = avif ? "av1" : "hevc";
            fail = double(out.w) * out.h > MAX_PIXELS ? Fail::TooBig : Fail::None;
            heif_image_handle_release(hh);
        }
        heif_context_free(ctx);
        return fail;
    }
    if (isWebp(h, n)) {
        WebPBitstreamFeatures ft;
        if (WebPGetFeatures(h, n, &ft) == VP8_STATUS_OK && ft.has_animation) {
            out.kind = Kind::Anim;
            out.codec = Codec::WebpAnim;
            out.w = ft.width;
            out.h = ft.height;
            out.format = "webp";
            out.vcodec = "webp";
            return double(out.w) * out.h > MAX_PIXELS ? Fail::TooBig : Fail::None;
        }
    }
    if (rawName(path)) {
        libraw_data_t *lr = libraw_init(0);
        Fail fail = Fail::Unsupported;
        if (lr && libraw_open_file(lr, path.c_str()) == LIBRAW_SUCCESS) {
            out.orient = orientOfFlip(lr->sizes.flip);
            out.w = lr->sizes.width;
            out.h = lr->sizes.height;
            if (gh0st::swapsSides(out.orient))
                std::swap(out.w, out.h);
            out.codec = Codec::Raw;
            out.format = "raw";
            out.vcodec = clean(lr->idata.make) + " " + clean(lr->idata.model);
            fail = Fail::None;
        }
        if (lr)
            libraw_close(lr);
        if (fail == Fail::None)
            return fail;
    }
    int64_t cw = 0, ch = 0;
    if (claimedSize(h, n, cw, ch) && double(cw) * ch > MAX_PIXELS)
        return Fail::TooBig;
    Av av;
    int r = av.open(path);
    if (r < 0)
        return fromAv(r);
    AVFormatContext *fmt = av.fmt;
    out.format = fmt->iformat->name;
    out.durMs = durationMs(fmt);
    int vi = -1, cover = -1;
    for (unsigned i = 0; i < fmt->nb_streams; i++) {
        AVStream *st = fmt->streams[i];
        switch (st->codecpar->codec_type) {
        case AVMEDIA_TYPE_AUDIO:
            if (!out.audio++)
                out.acodec = avcodec_get_name(st->codecpar->codec_id);
            break;
        case AVMEDIA_TYPE_SUBTITLE:
            out.subs++;
            break;
        case AVMEDIA_TYPE_VIDEO:
            if (st->disposition & AV_DISPOSITION_ATTACHED_PIC)
                cover = int(i);
            break;
        default:
            break;
        }
    }
    vi = av_find_best_stream(fmt, AVMEDIA_TYPE_VIDEO, -1, -1, nullptr, 0);
    if (vi >= 0 && (fmt->streams[vi]->disposition & AV_DISPOSITION_ATTACHED_PIC))
        vi = -1;
    if (sidecarSrt(path))
        out.subs++;
    if (vi < 0) {
        if (!out.audio)
            return Fail::Unsupported;
        out.kind = Kind::Audio;
        if (cover >= 0) {
            out.w = fmt->streams[cover]->codecpar->width;
            out.h = fmt->streams[cover]->codecpar->height;
        }
        return Fail::None;
    }
    AVStream *st = fmt->streams[vi];
    out.vcodec = avcodec_get_name(st->codecpar->codec_id);
    out.w = st->codecpar->width;
    out.h = st->codecpar->height;
    out.orient = orientOfStream(st);
    if (gh0st::swapsSides(out.orient))
        std::swap(out.w, out.h);
    bool still = (fmt->iformat->flags & AVFMT_NOTIMESTAMPS) || strstr(out.format.c_str(), "_pipe") ||
                 out.format == "image2" || out.format == "ico";
    bool moving = out.format == "gif" || out.format == "apng";
    if (moving) {
        // Two pictures make it move.
        AVPacket *p = av_packet_alloc();
        int pics = 0;
        while (pics < 2 && av_read_frame(fmt, p) >= 0) {
            pics += p->stream_index == vi;
            av_packet_unref(p);
        }
        av_packet_free(&p);
        out.kind = pics > 1 ? Kind::Anim : Kind::Image;
    } else {
        out.kind = still ? Kind::Image : Kind::Video;
    }
    if (out.w <= 0 || out.h <= 0)
        return Fail::Corrupt;
    if (out.kind != Kind::Video && double(out.w) * out.h > MAX_PIXELS)
        return Fail::TooBig;
    return Fail::None;
}

Fail Stills::tags(const std::string &path, Tags &out)
{
    out = Tags();
    Av av;
    int r = av.open(path);
    if (r < 0)
        return fromAv(r);
    auto get = [&](const char *key) {
        const AVDictionaryEntry *e = av_dict_get(av.fmt->metadata, key, nullptr, 0);
        // Ogg and FLAC keep their tags on the stream.
        for (unsigned i = 0; !e && i < av.fmt->nb_streams; i++)
            e = av_dict_get(av.fmt->streams[i]->metadata, key, nullptr, 0);
        return clean(e ? e->value : nullptr);
    };
    out.title = get("title");
    out.artist = get("artist");
    if (out.artist.empty())
        out.artist = get("album_artist");
    out.album = get("album");
    out.track = get("track");
    out.date = get("date");
    return Fail::None;
}

void Stills::forget()
{
    basePath_.clear();
    base_ = Pic();
}

// Decodes the whole picture into base_, unless it is there already.
Fail Stills::base(const std::string &path, const Info &info)
{
    int64_t stamp = stampOf(path);
    if (path == basePath_ && stamp == baseStamp_ && !base_.empty())
        return Fail::None;
    forget();
    Fail fail = Fail::Corrupt;
    switch (info.codec) {
    case Codec::Heif: {
        heif_context *ctx = heif_context_alloc();
        heif_image_handle *hh = nullptr;
        heif_image *img = nullptr;
        if (heif_context_read_from_file(ctx, path.c_str(), nullptr).code == heif_error_Ok &&
            heif_context_get_primary_image_handle(ctx, &hh).code == heif_error_Ok) {
            bool alpha = heif_image_handle_has_alpha_channel(hh);
            heif_error e = heifDecode(hh, &img, alpha);
            if (e.code == heif_error_Ok) {
                int stride = 0;
                const uint8_t *p = heif_image_get_plane_readonly(img, heif_channel_interleaved, &stride);
                int w = heif_image_get_width(img, heif_channel_interleaved);
                int h = heif_image_get_height(img, heif_channel_interleaved);
                const uint8_t *data[4] = {p, nullptr, nullptr, nullptr};
                int ls[4] = {stride, 0, 0, 0};
                fail = packBase(data, ls, alpha ? AV_PIX_FMT_RGBA : AV_PIX_FMT_RGB24, w, h, 1, base_);
                heif_image_release(img);
            } else if (e.code == heif_error_Memory_allocation_error) {
                fail = Fail::NoMem;
            } else if (e.code == heif_error_Unsupported_feature ||
                       e.code == heif_error_Unsupported_filetype) {
                fail = Fail::Unsupported;
            }
            heif_image_handle_release(hh);
        }
        heif_context_free(ctx);
        break;
    }
    case Codec::Raw: {
        // No JPEG preview: a half-size render of the mosaic.
        libraw_data_t *lr = libraw_init(0);
        if (!lr)
            return Fail::NoMem;
        lr->params.half_size = 1;
        lr->params.use_camera_wb = 1;
        if (libraw_open_file(lr, path.c_str()) == LIBRAW_SUCCESS &&
            libraw_unpack(lr) == LIBRAW_SUCCESS && libraw_dcraw_process(lr) == LIBRAW_SUCCESS) {
            int e = 0;
            libraw_processed_image_t *im = libraw_dcraw_make_mem_image(lr, &e);
            if (im && im->type == LIBRAW_IMAGE_BITMAP && im->colors == 3 && im->bits == 8) {
                const uint8_t *data[4] = {im->data, nullptr, nullptr, nullptr};
                int ls[4] = {im->width * 3, 0, 0, 0};
                fail = packBase(data, ls, AV_PIX_FMT_RGB24, im->width, im->height, 1, base_);
            } else if (e == LIBRAW_UNSUFFICIENT_MEMORY) {
                fail = Fail::NoMem;
            }
            if (im)
                libraw_dcraw_clear_mem(im);
        } else {
            fail = Fail::Unsupported;
        }
        libraw_close(lr);
        break;
    }
    case Codec::WebpAnim: {
        int err = 0;
        std::vector<uint8_t> b = slurp(path, err);
        if (b.empty())
            return err ? fromErrno(err) : Fail::Corrupt;
        WebPData data = {b.data(), b.size()};
        WebPAnimDecoderOptions o;
        WebPAnimDecoderOptionsInit(&o);
        o.color_mode = MODE_RGBA;
        WebPAnimDecoder *d = WebPAnimDecoderNew(&data, &o);
        WebPAnimInfo ai;
        uint8_t *buf = nullptr;
        int ts = 0;
        if (d && WebPAnimDecoderGetInfo(d, &ai) && WebPAnimDecoderGetNext(d, &buf, &ts)) {
            const uint8_t *pl[4] = {buf, nullptr, nullptr, nullptr};
            int ls[4] = {int(ai.canvas_width) * 4, 0, 0, 0};
            fail = packBase(pl, ls, AV_PIX_FMT_RGBA, int(ai.canvas_width), int(ai.canvas_height), 1,
                            base_);
        }
        WebPAnimDecoderDelete(d);
        break;
    }
    case Codec::Av: {
        Av av;
        int r = av.open(path);
        if (r < 0)
            return fromAv(r);
        AVStream *st = nullptr;
        for (unsigned i = 0; info.kind == Kind::Audio && i < av.fmt->nb_streams; i++)
            if (av.fmt->streams[i]->disposition & AV_DISPOSITION_ATTACHED_PIC)
                st = av.fmt->streams[i];
        if (info.kind == Kind::Audio && !st) {
            std::string cover = folderCover(path);
            if (cover.empty())
                return Fail::Unsupported;
            Info ci;
            Fail f = probe(cover, ci);
            if (f != Fail::None)
                return f;
            f = base(cover, ci);
            // Kept under the song's name.
            basePath_ = path;
            baseStamp_ = stamp;
            return f;
        }
        if (!st) {
            int vi = av_find_best_stream(av.fmt, AVMEDIA_TYPE_VIDEO, -1, -1, nullptr, 0);
            if (vi < 0)
                return Fail::Unsupported;
            st = av.fmt->streams[vi];
        }
        if (double(st->codecpar->width) * st->codecpar->height > MAX_PIXELS)
            return Fail::TooBig;
        AVFrame *f = av_frame_alloc();
        // A video's picture: a tenth of the way in, past the titles.
        int64_t at = info.kind == Kind::Video ? durationMs(av.fmt) / 10 : 0;
        fail = decodeFrame(av.fmt, st, at, f);
        if (fail == Fail::None)
            fail = packBase(f->data, f->linesize, AVPixelFormat(f->format), f->width, f->height,
                            info.kind == Kind::Audio ? 1 : orientOfStream(st), base_);
        av_frame_free(&f);
        break;
    }
    case Codec::Jpeg: {
        // A cover beside a song: the whole picture, small enough to keep.
        FILE *f = fopen(path.c_str(), "rb");
        if (!f)
            return fromErrno(errno);
        int W = 0, H = 0, o = 1;
        Pic stored;
        if (gh0st::jpegHeader(f, nullptr, W, H, o)) {
            double k = std::min(1.0, std::sqrt(BASE_PIXELS / (double(W) * H)));
            int bw = std::max(1, int(W * k)), bh = std::max(1, int(H * k));
            if (gh0st::jpegRegion(f, nullptr, 0, 0, W, H, bw, bh, MAX_JPEG_PIXELS, stored)) {
                base_ = gh0st::orient(stored, o);
                fail = Fail::None;
            }
        }
        fclose(f);
        break;
    }
    }
    if (fail == Fail::None) {
        basePath_ = path;
        baseStamp_ = stamp;
    } else {
        base_ = Pic();
    }
    return fail;
}

Fail Stills::render(const std::string &path, const Info &info, int w, int h, float zoom, float cx,
                    float cy, bool fill, Pic &out)
{
    std::vector<uint8_t> preview;
    int orient = info.orient;
    bool jpeg = info.codec == Codec::Jpeg;
    if (info.codec == Codec::Raw && rawPreview(path, preview, orient))
        jpeg = true;
    if (jpeg) {
        FILE *f = nullptr;
        int W = 0, H = 0, o = 1;
        if (preview.empty()) {
            f = fopen(path.c_str(), "rb");
            if (!f)
                return fromErrno(errno);
        }
        const std::vector<uint8_t> *mem = preview.empty() ? nullptr : &preview;
        if (!gh0st::jpegHeader(f, mem, W, H, o)) {
            if (f)
                fclose(f);
            return Fail::Corrupt;
        }
        if (!mem)
            orient = o;
        double DW = gh0st::swapsSides(orient) ? H : W, DH = gh0st::swapsSides(orient) ? W : H;
        Frame fr = frameFor(DW, DH, w, h, zoom, cx, cy, fill);
        double x = fr.x, y = fr.y, rw = fr.w, rh = fr.h;
        gh0st::unorientRect(orient, DW, DH, x, y, rw, rh);
        bool sw = gh0st::swapsSides(orient);
        Pic stored;
        bool ok = gh0st::jpegRegion(f, mem, x, y, rw, rh, sw ? fr.oh : fr.ow, sw ? fr.ow : fr.oh,
                                    MAX_JPEG_PIXELS, stored);
        if (f)
            fclose(f);
        if (!ok)
            return Fail::Corrupt;
        out = gh0st::orient(stored, orient);
        return Fail::None;
    }
    Fail f = base(path, info);
    if (f != Fail::None)
        return f;
    // The base is smaller than the picture when the picture is large: the
    // frame is found in the picture's pixels, then read from the base.
    double W = info.kind == Kind::Audio ? base_.w : info.w;
    double H = info.kind == Kind::Audio ? base_.h : info.h;
    if (W <= 0 || H <= 0)
        return Fail::Corrupt;
    Frame fr = frameFor(W, H, w, h, zoom, cx, cy, fill);
    double kx = base_.w / W, ky = base_.h / H;
    out = sample(base_, fr.x * kx, fr.y * ky, fr.w * kx, fr.h * ky, fr.ow, fr.oh);
    return Fail::None;
}

Fail Stills::info(const std::string &path, Info &out)
{
    int64_t stamp = stampOf(path);
    if (path != infoPath_ || stamp != infoStamp_) {
        infoFail_ = probe(path, info_);
        infoPath_ = path;
        infoStamp_ = stamp;
    }
    out = info_;
    return infoFail_;
}

Fail Stills::view(const std::string &path, int w, int h, float zoom, float cx, float cy, Pic &out)
{
    Info info;
    Fail f = this->info(path, info);
    if (f != Fail::None)
        return f;
    return render(path, info, w, h, zoom, cx, cy, false, out);
}

Fail Stills::thumb(const std::string &path, int w, int h, Pic &out)
{
    Info info;
    Fail f = this->info(path, info);
    if (f != Fail::None)
        return f;
    f = render(path, info, w, h, 1, 0.5f, 0.5f, true, out);
    // A tile is asked once; its picture is not kept.
    if (info.codec != Codec::Jpeg && path == basePath_ && (base_.w > 2 * w || base_.h > 2 * h)) {
        // Large bases stay only for the picture on screen.
        forget();
    }
    return f;
}

} // namespace mediad
