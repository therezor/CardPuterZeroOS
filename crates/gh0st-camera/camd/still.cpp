#include "still.h"

#include <algorithm>
#include <cerrno>
#include <cmath>
#include <csetjmp>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <fcntl.h>
#include <unistd.h>

#include <jpeglib.h>

namespace camd {

namespace {

struct JpegErr {
    jpeg_error_mgr pub;
    jmp_buf jb;
};

void onJpegError(j_common_ptr c)
{
    longjmp(reinterpret_cast<JpegErr *>(c->err)->jb, 1);
}

// Box filter of one plane (`ch` interleaved channels) into dw x dh.
void box(const uint8_t *src, int sw, int sh, int sstride, int ch, uint8_t *dst, int dw, int dh,
         int dstride)
{
    for (int y = 0; y < dh; y++) {
        int y0 = int(int64_t(y) * sh / dh), y1 = std::max(y0 + 1, int(int64_t(y + 1) * sh / dh));
        for (int x = 0; x < dw; x++) {
            int x0 = int(int64_t(x) * sw / dw), x1 = std::max(x0 + 1, int(int64_t(x + 1) * sw / dw));
            for (int k = 0; k < ch; k++) {
                uint32_t sum = 0;
                for (int yy = y0; yy < y1; yy++) {
                    const uint8_t *r = src + size_t(yy) * sstride + k;
                    for (int xx = x0; xx < x1; xx++)
                        sum += r[xx * ch];
                }
                dst[size_t(y) * dstride + x * ch + k] = uint8_t(sum / ((y1 - y0) * (x1 - x0)));
            }
        }
    }
}

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

// Limited to full range, for a JPEG made from video.
inline uint8_t fullLuma(int y)
{
    return clamp8(((y - 16) * 298) >> 8);
}

inline uint8_t fullChroma(int c)
{
    return clamp8(128 + (((c - 128) * 291) >> 8));
}

// Raw 4:2:0 data into libjpeg, which wants rows padded to whole MCUs: each
// band of 16 rows is copied into buffers that wide, edges repeated.
bool compress(const Yuv420 &img, int quality, FILE *file, unsigned char **mem,
              unsigned long *memLen, const std::vector<uint8_t> *app1)
{
    const int pw = (img.w + 15) & ~15, cpw = pw / 2, cw = img.cw(), ch = img.ch();
    std::vector<uint8_t> yb(size_t(pw) * 16), ub(size_t(cpw) * 8), vb(size_t(cpw) * 8);
    jpeg_compress_struct c;
    JpegErr err;
    c.err = jpeg_std_error(&err.pub);
    err.pub.error_exit = onJpegError;
    if (setjmp(err.jb)) {
        jpeg_destroy_compress(&c);
        return false;
    }
    jpeg_create_compress(&c);
    if (file)
        jpeg_stdio_dest(&c, file);
    else
        jpeg_mem_dest(&c, mem, memLen);
    c.image_width = img.w;
    c.image_height = img.h;
    c.input_components = 3;
    c.in_color_space = JCS_YCbCr;
    jpeg_set_defaults(&c);
    jpeg_set_colorspace(&c, JCS_YCbCr);
    jpeg_set_quality(&c, quality, TRUE);
    c.raw_data_in = TRUE;
    c.dct_method = JDCT_ISLOW;
    c.write_JFIF_header = FALSE;
    c.comp_info[0].h_samp_factor = c.comp_info[0].v_samp_factor = 2;
    for (int i = 1; i < 3; i++)
        c.comp_info[i].h_samp_factor = c.comp_info[i].v_samp_factor = 1;
    jpeg_start_compress(&c, TRUE);
    if (app1 && !app1->empty())
        jpeg_write_marker(&c, JPEG_APP0 + 1, app1->data(), unsigned(app1->size()));
    JSAMPROW yr[16], ur[8], vr[8];
    JSAMPARRAY planes[3] = {yr, ur, vr};
    auto fill = [](uint8_t *dst, const uint8_t *src, int w, int pw) {
        memcpy(dst, src, w);
        memset(dst + w, src[w - 1], pw - w);
    };
    for (int row = 0; row < img.h; row += 16) {
        for (int i = 0; i < 16; i++) {
            int sy = std::min(row + i, img.h - 1);
            yr[i] = &yb[size_t(i) * pw];
            fill(yr[i], &img.y[size_t(sy) * img.w], img.w, pw);
        }
        for (int i = 0; i < 8; i++) {
            int sy = std::min(row / 2 + i, ch - 1);
            ur[i] = &ub[size_t(i) * cpw];
            vr[i] = &vb[size_t(i) * cpw];
            fill(ur[i], &img.u[size_t(sy) * cw], cw, cpw);
            fill(vr[i], &img.v[size_t(sy) * cw], cw, cpw);
        }
        jpeg_write_raw_data(&c, planes, 16);
    }
    jpeg_finish_compress(&c);
    jpeg_destroy_compress(&c);
    return true;
}

// EXIF, little-endian TIFF.
enum { BYTE = 1, ASCII = 2, SHORT = 3, LONG = 4, RATIONAL = 5, UNDEF = 7, SRATIONAL = 10 };

struct Entry {
    uint16_t tag, type;
    uint32_t count;
    std::vector<uint8_t> data;
};

void le16(std::vector<uint8_t> &b, uint32_t v)
{
    b.push_back(v & 0xff);
    b.push_back((v >> 8) & 0xff);
}

void le32(std::vector<uint8_t> &b, uint32_t v)
{
    le16(b, v & 0xffff);
    le16(b, v >> 16);
}

Entry ascii(uint16_t tag, const std::string &s)
{
    Entry e{tag, ASCII, uint32_t(s.size() + 1), {s.begin(), s.end()}};
    e.data.push_back(0);
    return e;
}

Entry short1(uint16_t tag, uint16_t v)
{
    Entry e{tag, SHORT, 1, {}};
    le16(e.data, v);
    return e;
}

Entry long1(uint16_t tag, uint32_t v)
{
    Entry e{tag, LONG, 1, {}};
    le32(e.data, v);
    return e;
}

Entry rational(uint16_t tag, uint32_t n, uint32_t d, bool sign = false)
{
    Entry e{tag, uint16_t(sign ? SRATIONAL : RATIONAL), 1, {}};
    le32(e.data, n);
    le32(e.data, d);
    return e;
}

size_t ifdSize(const std::vector<Entry> &ifd)
{
    size_t n = 2 + 12 * ifd.size() + 4;
    for (auto &e : ifd)
        if (e.data.size() > 4)
            n += (e.data.size() + 1) & ~size_t(1);
    return n;
}

void putIfd(std::vector<uint8_t> &b, const std::vector<Entry> &ifd, uint32_t at, uint32_t next)
{
    uint32_t extra = at + 2 + 12 * uint32_t(ifd.size()) + 4;
    std::vector<uint8_t> tail;
    le16(b, uint16_t(ifd.size()));
    for (auto &e : ifd) {
        le16(b, e.tag);
        le16(b, e.type);
        le32(b, e.count);
        if (e.data.size() <= 4) {
            std::vector<uint8_t> v = e.data;
            v.resize(4, 0);
            b.insert(b.end(), v.begin(), v.end());
        } else {
            le32(b, extra + uint32_t(tail.size()));
            tail.insert(tail.end(), e.data.begin(), e.data.end());
            if (tail.size() & 1)
                tail.push_back(0);
        }
    }
    le32(b, next);
    b.insert(b.end(), tail.begin(), tail.end());
}

} // namespace

Yuv420 shrink(const Yuv420 &img, int w, int h)
{
    Yuv420 out;
    out.resize(w, h);
    box(img.y.data(), img.w, img.h, img.w, 1, out.y.data(), w, h, w);
    box(img.u.data(), img.cw(), img.ch(), img.cw(), 1, out.u.data(), out.cw(), out.ch(), out.cw());
    box(img.v.data(), img.cw(), img.ch(), img.cw(), 1, out.v.data(), out.cw(), out.ch(), out.cw());
    return out;
}

std::vector<uint8_t> exifBlock(const Yuv420 &img, const ShotMeta &meta,
                               const std::vector<uint8_t> &thumb)
{
    time_t now = time(nullptr);
    struct tm tm;
    localtime_r(&now, &tm);
    char date[20], off[16];
    strftime(date, sizeof date, "%Y:%m:%d %H:%M:%S", &tm);
    long gmt = tm.tm_gmtoff / 60;
    snprintf(off, sizeof off, "%c%02d:%02d", gmt < 0 ? '-' : '+', int(labs(gmt) / 60 % 24),
             int(labs(gmt) % 60));

    std::vector<Entry> ifd0 = {
        ascii(0x010f, "M5Stack"),
        ascii(0x0110, "CardputerZero"),
        short1(0x0112, 1),
        rational(0x011a, 72, 1),
        rational(0x011b, 72, 1),
        short1(0x0128, 2),
        ascii(0x0131, "gh0st-camd"),
        ascii(0x0132, date),
        short1(0x0213, 1),
        long1(0x8769, 0), // the Exif IFD, set below
    };
    int iso = int(std::lround(meta.gain * 100));
    Entry version{0x9000, UNDEF, 4, {'0', '2', '3', '2'}};
    std::vector<Entry> exif = {
        rational(0x829a, uint32_t(std::max<int64_t>(meta.exposure_us, 1)), 1000000),
        rational(0x829d, 20, 10),
        short1(0x8827, uint16_t(std::clamp(iso, 1, 65535))),
        version,
        ascii(0x9003, date),
        ascii(0x9004, date),
        ascii(0x9010, off),
        ascii(0x9011, off),
        rational(0x9204, uint32_t(int32_t(std::lround(meta.ev * 100))), 100, true),
        rational(0x920a, 285, 100),
        short1(0xa001, 1),
        long1(0xa002, uint32_t(img.w)),
        long1(0xa003, uint32_t(img.h)),
        short1(0xa402, 0),
        short1(0xa403, 0),
        rational(0xa404, uint32_t(std::lround(meta.zoom * 100)), 100),
    };
    std::vector<Entry> ifd1 = {short1(0x0103, 6), long1(0x0201, 0),
                               long1(0x0202, uint32_t(thumb.size()))};

    uint32_t at0 = 8, atExif = at0 + uint32_t(ifdSize(ifd0));
    uint32_t at1 = atExif + uint32_t(ifdSize(exif));
    uint32_t atThumb = at1 + uint32_t(ifdSize(ifd1));
    ifd0.back().data.clear();
    le32(ifd0.back().data, atExif);
    ifd1[1].data.clear();
    le32(ifd1[1].data, atThumb);

    std::vector<uint8_t> b = {'E', 'x', 'i', 'f', 0, 0, 'I', 'I', 42, 0};
    le32(b, at0);
    std::vector<uint8_t> tiff;
    putIfd(tiff, ifd0, at0, thumb.empty() ? 0 : at1);
    putIfd(tiff, exif, atExif, 0);
    if (!thumb.empty()) {
        putIfd(tiff, ifd1, at1, 0);
        tiff.insert(tiff.end(), thumb.begin(), thumb.end());
    }
    b.insert(b.end(), tiff.begin(), tiff.end());
    return b;
}

bool saveJpeg(const Yuv420 &img, const ShotMeta &meta, int quality, const std::string &path,
              std::string &why)
{
    std::vector<uint8_t> thumb;
    {
        int tw = 160, th = std::max(2, (160 * img.h / std::max(img.w, 1)) & ~1);
        Yuv420 small = shrink(img, tw, th);
        unsigned char *mem = nullptr;
        unsigned long len = 0;
        if (compress(small, 80, nullptr, &mem, &len, nullptr))
            thumb.assign(mem, mem + len);
        free(mem);
    }
    std::vector<uint8_t> app1 = exifBlock(img, meta, thumb);
    // An APP1 segment holds 64 KB: drop the thumbnail before the EXIF.
    if (app1.size() > 65533)
        app1 = exifBlock(img, meta, {});

    std::string part = path + ".part";
    FILE *f = fopen(part.c_str(), "wb");
    if (!f) {
        why = strerror(errno);
        return false;
    }
    bool ok = compress(img, quality, f, nullptr, nullptr, &app1);
    ok = ok && fflush(f) == 0 && fsync(fileno(f)) == 0;
    if (!ok)
        why = errno ? strerror(errno) : "jpeg";
    if (fclose(f) != 0 && ok) {
        ok = false;
        why = strerror(errno);
    }
    if (ok && rename(part.c_str(), path.c_str()) != 0) {
        ok = false;
        why = strerror(errno);
    }
    if (!ok) {
        unlink(part.c_str());
        return false;
    }
    std::string dir = path.substr(0, path.find_last_of('/') + 1);
    int d = ::open(dir.empty() ? "." : dir.c_str(), O_RDONLY);
    if (d >= 0) {
        fsync(d);
        ::close(d);
    }
    return true;
}

namespace {

constexpr long DECODE_MEMORY = 96L << 20;
// Four times the sensor's 8 MP.
constexpr double DECODE_PIXELS = 32e6;

// The JPEG thumbnail in a photo's EXIF (IFD1), as our stills carry one.
bool exifThumb(FILE *f, std::vector<uint8_t> &jpg)
{
    uint8_t head[65536 + 4];
    size_t n = fread(head, 1, sizeof head, f);
    rewind(f);
    if (n < 20 || head[0] != 0xff || head[1] != 0xd8 || head[2] != 0xff || head[3] != 0xe1 ||
        memcmp(head + 6, "Exif\0\0", 6) != 0)
        return false;
    const uint8_t *t = head + 12;
    size_t tn = std::min(n - 12, size_t(head[4] << 8 | head[5]) - 8);
    bool le = t[0] == 'I';
    auto u16 = [&](size_t o) -> uint32_t {
        return o + 2 > tn ? 0 : le ? (t[o] | t[o + 1] << 8) : (t[o] << 8 | t[o + 1]);
    };
    auto u32 = [&](size_t o) -> uint32_t {
        return o + 4 > tn ? 0 : le ? (u16(o) | u16(o + 2) << 16) : (u16(o) << 16 | u16(o + 2));
    };
    uint32_t ifd0 = u32(4);
    uint32_t ifd1 = u32(ifd0 + 2 + 12 * u16(ifd0));
    if (!ifd1)
        return false;
    uint32_t at = 0, len = 0;
    for (uint32_t i = 0, k = u16(ifd1); i < k; i++) {
        size_t e = ifd1 + 2 + 12 * i;
        if (u16(e) == 0x0201)
            at = u32(e + 8);
        else if (u16(e) == 0x0202)
            len = u32(e + 8);
    }
    // In 64 bits: a crafted offset near 4 GB would wrap a 32-bit sum.
    if (!at || !len || size_t(at) + len > tn)
        return false;
    jpg.assign(t + at, t + at + len);
    return true;
}

// Decodes the part of a JPEG that a w x h view at `zoom` centred on (cx, cy)
// shows, at the scale it is shown: libjpeg-turbo skips the rows above,
// crops the columns, and scales the DCT. Zoom 1 fits the whole picture;
// the most is one picture pixel to one screen pixel.
bool decodeView(FILE *f, const std::vector<uint8_t> *mem, int w, int h, float zoom, float cx,
                float cy, std::vector<uint16_t> &out)
{
    std::vector<uint8_t> rows, fit;
    jpeg_decompress_struct d;
    JpegErr err;
    d.err = jpeg_std_error(&err.pub);
    err.pub.error_exit = onJpegError;
    if (setjmp(err.jb)) {
        jpeg_destroy_decompress(&d);
        return false;
    }
    jpeg_create_decompress(&d);
    // A progressive JPEG of a huge size wants its whole picture in memory.
    // Past this libjpeg stops with an error instead, as it has no backing
    // store, and the helper lives on.
    d.mem->max_memory_to_use = DECODE_MEMORY;
    if (mem)
        jpeg_mem_src(&d, mem->data(), mem->size());
    else
        jpeg_stdio_src(&d, f);
    jpeg_read_header(&d, TRUE);
    // Rows for the view are sized from the picture's width: a huge one from
    // elsewhere is skipped, not decoded.
    if (double(d.image_width) * d.image_height > DECODE_PIXELS) {
        jpeg_destroy_decompress(&d);
        return false;
    }
    const double W = d.image_width, H = d.image_height;
    double scale = std::min(w / W, h / H) * std::max(1.0f, zoom);
    scale = std::min(scale, 1.0);
    double rw = std::min(W, w / scale), rh = std::min(H, h / scale);
    double rx = std::clamp(cx * W - rw / 2, 0.0, W - rw);
    double ry = std::clamp(cy * H - rh / 2, 0.0, H - rh);
    int ow = std::clamp(int(std::lround(rw * scale)), 1, w);
    int oh = std::clamp(int(std::lround(rh * scale)), 1, h);
    d.scale_num = std::clamp(unsigned(std::ceil(8 * scale)), 1u, 8u);
    d.scale_denom = 8;
    d.out_color_space = JCS_RGB;
    d.dct_method = JDCT_IFAST;
    jpeg_start_decompress(&d);
    const double k = double(d.output_width) / W;
    JDIMENSION dx = JDIMENSION(rx * k), dw = std::max<JDIMENSION>(1, JDIMENSION(rw * k));
    JDIMENSION dy = JDIMENSION(ry * k), dh = std::max<JDIMENSION>(1, JDIMENSION(rh * k));
    dw = std::min(dw, d.output_width - dx);
    dh = std::min(dh, d.output_height - dy);
    JDIMENSION xoff = dx, cw = dw;
    if (dw < d.output_width)
        jpeg_crop_scanline(&d, &xoff, &cw);
    if (dy)
        jpeg_skip_scanlines(&d, dy);
    const int stride = int(d.output_width) * 3;
    rows.resize(size_t(stride) * dh);
    for (JDIMENSION y = 0; y < dh; y++) {
        JSAMPROW r = &rows[size_t(y) * stride];
        jpeg_read_scanlines(&d, &r, 1);
    }
    jpeg_abort_decompress(&d);
    jpeg_destroy_decompress(&d);

    fit.resize(size_t(ow) * oh * 3);
    box(rows.data() + size_t(dx - xoff) * 3, int(dw), int(dh), stride, 3, fit.data(), ow, oh,
        ow * 3);
    out.assign(size_t(w) * h, 0);
    int ox = (w - ow) / 2, oy = (h - oh) / 2;
    for (int y = 0; y < oh; y++)
        for (int x = 0; x < ow; x++) {
            const uint8_t *p = &fit[(size_t(y) * ow + x) * 3];
            out[size_t(y + oy) * w + x + ox] = pack565(p[0], p[1], p[2]);
        }
    return true;
}

} // namespace

bool jpegTo565(const std::string &path, int w, int h, float zoom, float cx, float cy,
               std::vector<uint16_t> &out)
{
    FILE *f = fopen(path.c_str(), "rb");
    if (!f || w <= 0 || h <= 0) {
        if (f)
            fclose(f);
        return false;
    }
    bool ok = false;
    // A small, whole picture comes from the 160x120 EXIF thumbnail: a
    // gallery screen of tiles in milliseconds, not a decode of 8 MP each.
    std::vector<uint8_t> thumb;
    if (zoom <= 1 && w <= 160 && h <= 120 && exifThumb(f, thumb))
        ok = decodeView(nullptr, &thumb, w, h, 1, 0.5f, 0.5f, out);
    if (!ok)
        ok = decodeView(f, nullptr, w, h, zoom, cx, cy, out);
    fclose(f);
    return ok;
}

Yuv420 viewToYuv(const View &v)
{
    Yuv420 out;
    out.resize(v.w & ~1, v.h & ~1);
    if (v.fmt == View::YUV420) {
        for (int y = 0; y < out.h; y++)
            memcpy(&out.y[size_t(y) * out.w], v.p[0] + size_t(y) * v.stride[0], out.w);
        for (int y = 0; y < out.ch(); y++) {
            memcpy(&out.u[size_t(y) * out.cw()], v.p[1] + size_t(y) * v.stride[1], out.cw());
            memcpy(&out.v[size_t(y) * out.cw()], v.p[2] + size_t(y) * v.stride[2], out.cw());
        }
        // A JPEG is full range: stretch video's 16 to 235.
        if (v.limited) {
            for (auto &p : out.y)
                p = fullLuma(p);
            for (auto &p : out.u)
                p = fullChroma(p);
            for (auto &p : out.v)
                p = fullChroma(p);
        }
        return out;
    }
    for (int y = 0; y < out.h; y++)
        for (int x = 0; x < out.w; x++) {
            uint16_t px;
            memcpy(&px, v.p[0] + size_t(y) * v.stride[0] + x * 2, 2);
            int r = (px >> 11) << 3, g = ((px >> 5) & 63) << 2, b = (px & 31) << 3;
            out.y[size_t(y) * out.w + x] = clamp8((77 * r + 150 * g + 29 * b) >> 8);
            if (!(x & 1) && !(y & 1)) {
                size_t c = size_t(y / 2) * out.cw() + x / 2;
                out.u[c] = clamp8(128 + ((-43 * r - 85 * g + 128 * b) >> 8));
                out.v[c] = clamp8(128 + ((128 * r - 107 * g - 21 * b) >> 8));
            }
        }
    return out;
}

void viewTo565(const View &v, int w, int h, uint16_t *out)
{
    if (v.fmt == View::RGB565 && v.w == w && v.h == h) {
        for (int y = 0; y < h; y++)
            memcpy(out + size_t(y) * w, v.p[0] + size_t(y) * v.stride[0], size_t(w) * 2);
        return;
    }
    // Camera memory is uncached: reading it a byte at a time costs most of
    // a frame. Each source row comes out in one copy, then converts from
    // cache.
    std::vector<uint8_t> row(size_t(v.w) * 2), urow(v.w / 2 + 1), vrow(v.w / 2 + 1);
    int lastY = -1, lastC = -1;
    for (int y = 0; y < h; y++) {
        int sy = int(int64_t(y) * v.h / h);
        if (sy != lastY) {
            memcpy(row.data(), v.p[0] + size_t(sy) * v.stride[0],
                   v.fmt == View::RGB565 ? size_t(v.w) * 2 : size_t(v.w));
            lastY = sy;
        }
        if (v.fmt == View::YUV420 && sy / 2 != lastC) {
            memcpy(urow.data(), v.p[1] + size_t(sy / 2) * v.stride[1], v.w / 2);
            memcpy(vrow.data(), v.p[2] + size_t(sy / 2) * v.stride[2], v.w / 2);
            lastC = sy / 2;
        }
        uint16_t *o = out + size_t(y) * w;
        for (int x = 0; x < w; x++) {
            int sx = int(int64_t(x) * v.w / w);
            if (v.fmt == View::RGB565) {
                memcpy(&o[x], &row[size_t(sx) * 2], 2);
            } else {
                int Y = row[sx], U = urow[sx / 2], V = vrow[sx / 2];
                o[x] = v.limited ? yuv565Video(Y, U, V) : yuv565(Y, U, V);
            }
        }
    }
}

} // namespace camd
