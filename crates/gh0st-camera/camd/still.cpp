#include "still.h"

#include <algorithm>
#include <cerrno>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <fcntl.h>
#include <unistd.h>

namespace camd {

namespace {

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

} // namespace camd
