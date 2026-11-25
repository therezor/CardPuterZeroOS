#include "decode.h"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <memory>
#include <new>

namespace gh0st {

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

namespace {

constexpr long DECODE_MEMORY = 96L << 20;
// Four times the sensor's 8 MP.
constexpr double DECODE_PIXELS = 32e6;

// The rows a decode reads into. They grow after setjmp, and a local
// changed between setjmp and longjmp is indeterminate after the jump: on
// the heap they stay whole.
std::unique_ptr<std::vector<uint8_t>> scratch()
{
    return std::make_unique<std::vector<uint8_t>>();
}

// False when memory runs out. A throw here would skip
// jpeg_destroy_decompress.
bool grow(std::vector<uint8_t> &v, size_t n)
{
    try {
        v.resize(n);
        return true;
    } catch (const std::bad_alloc &) {
        return false;
    }
}

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
    std::unique_ptr<std::vector<uint8_t>> rows = scratch();
    std::vector<uint8_t> fit;
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
    if (!grow(*rows, size_t(stride) * dh)) {
        jpeg_destroy_decompress(&d);
        return false;
    }
    for (JDIMENSION y = 0; y < dh; y++) {
        JSAMPROW r = &(*rows)[size_t(y) * stride];
        jpeg_read_scanlines(&d, &r, 1);
    }
    jpeg_abort_decompress(&d);
    jpeg_destroy_decompress(&d);

    fit.resize(size_t(ow) * oh * 3);
    box(rows->data() + size_t(dx - xoff) * 3, int(dw), int(dh), stride, 3, fit.data(), ow, oh,
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
        // An odd width's last column has a chroma sample of its own.
        if (v.fmt == View::YUV420 && sy / 2 != lastC) {
            memcpy(urow.data(), v.p[1] + size_t(sy / 2) * v.stride[1], (v.w + 1) / 2);
            memcpy(vrow.data(), v.p[2] + size_t(sy / 2) * v.stride[2], (v.w + 1) / 2);
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

bool jpegHeader(FILE *f, const std::vector<uint8_t> *mem, int &w, int &h, int &orient)
{
    jpeg_decompress_struct d;
    JpegErr err;
    d.err = jpeg_std_error(&err.pub);
    err.pub.error_exit = onJpegError;
    if (setjmp(err.jb)) {
        jpeg_destroy_decompress(&d);
        if (f)
            rewind(f);
        return false;
    }
    jpeg_create_decompress(&d);
    jpeg_save_markers(&d, JPEG_APP0 + 1, 0xffff);
    if (mem)
        jpeg_mem_src(&d, mem->data(), mem->size());
    else
        jpeg_stdio_src(&d, f);
    jpeg_read_header(&d, TRUE);
    w = int(d.image_width);
    h = int(d.image_height);
    orient = 1;
    for (jpeg_saved_marker_ptr m = d.marker_list; m; m = m->next) {
        if (m->marker != JPEG_APP0 + 1 || m->data_length < 14 ||
            memcmp(m->data, "Exif\0\0", 6) != 0)
            continue;
        const uint8_t *t = m->data + 6;
        size_t tn = m->data_length - 6;
        bool le = t[0] == 'I';
        auto u16 = [&](size_t o) -> uint32_t {
            return o + 2 > tn ? 0 : le ? (t[o] | t[o + 1] << 8) : (t[o] << 8 | t[o + 1]);
        };
        auto u32 = [&](size_t o) -> uint32_t {
            return o + 4 > tn ? 0 : le ? (u16(o) | u16(o + 2) << 16) : (u16(o) << 16 | u16(o + 2));
        };
        uint32_t ifd0 = u32(4);
        for (uint32_t i = 0, k = u16(ifd0); i < k && i < 512; i++) {
            size_t e = size_t(ifd0) + 2 + 12 * i;
            if (u16(e) == 0x0112) {
                uint32_t v = u16(e + 8);
                if (v >= 1 && v <= 8)
                    orient = int(v);
            }
        }
        break;
    }
    jpeg_destroy_decompress(&d);
    if (f)
        rewind(f);
    return w > 0 && h > 0;
}

bool jpegRegion(FILE *f, const std::vector<uint8_t> *mem, double sx, double sy, double sw,
                double sh, int ow, int oh, double maxPixels, Pic &out)
{
    std::unique_ptr<std::vector<uint8_t>> rows = scratch();
    std::vector<uint8_t> fit;
    jpeg_decompress_struct d;
    JpegErr err;
    d.err = jpeg_std_error(&err.pub);
    err.pub.error_exit = onJpegError;
    if (setjmp(err.jb)) {
        jpeg_destroy_decompress(&d);
        return false;
    }
    jpeg_create_decompress(&d);
    d.mem->max_memory_to_use = DECODE_MEMORY;
    if (mem)
        jpeg_mem_src(&d, mem->data(), mem->size());
    else
        jpeg_stdio_src(&d, f);
    jpeg_read_header(&d, TRUE);
    if (double(d.image_width) * d.image_height > maxPixels || ow <= 0 || oh <= 0) {
        jpeg_destroy_decompress(&d);
        return false;
    }
    const double W = d.image_width, H = d.image_height;
    sw = std::clamp(sw, 1.0, W);
    sh = std::clamp(sh, 1.0, H);
    sx = std::clamp(sx, 0.0, W - sw);
    sy = std::clamp(sy, 0.0, H - sh);
    double scale = std::min(1.0, std::max(ow / sw, oh / sh));
    d.scale_num = std::clamp(unsigned(std::ceil(8 * scale)), 1u, 8u);
    d.scale_denom = 8;
    d.out_color_space = JCS_RGB;
    d.dct_method = JDCT_IFAST;
    jpeg_start_decompress(&d);
    const double k = double(d.output_width) / W;
    JDIMENSION dx = JDIMENSION(sx * k), dw = std::max<JDIMENSION>(1, JDIMENSION(sw * k));
    JDIMENSION dy = JDIMENSION(sy * k), dh = std::max<JDIMENSION>(1, JDIMENSION(sh * k));
    dx = std::min(dx, d.output_width - 1);
    dy = std::min(dy, d.output_height - 1);
    dw = std::min(dw, d.output_width - dx);
    dh = std::min(dh, d.output_height - dy);
    JDIMENSION xoff = dx, cw = dw;
    if (dw < d.output_width)
        jpeg_crop_scanline(&d, &xoff, &cw);
    if (dy)
        jpeg_skip_scanlines(&d, dy);
    const int stride = int(d.output_width) * 3;
    if (!grow(*rows, size_t(stride) * dh)) {
        jpeg_destroy_decompress(&d);
        return false;
    }
    for (JDIMENSION y = 0; y < dh; y++) {
        JSAMPROW r = &(*rows)[size_t(y) * stride];
        jpeg_read_scanlines(&d, &r, 1);
    }
    jpeg_abort_decompress(&d);
    jpeg_destroy_decompress(&d);

    fit.resize(size_t(ow) * oh * 3);
    box(rows->data() + size_t(dx - xoff) * 3, int(dw), int(dh), stride, 3, fit.data(), ow, oh,
        ow * 3);
    out.w = ow;
    out.h = oh;
    out.px.resize(size_t(ow) * oh);
    for (size_t i = 0; i < out.px.size(); i++)
        out.px[i] = pack565(fit[i * 3], fit[i * 3 + 1], fit[i * 3 + 2]);
    return true;
}

Pic orient(const Pic &src, int o)
{
    if (o <= 1 || o > 8)
        return src;
    Pic out;
    const int SW = src.w, SH = src.h;
    out.w = swapsSides(o) ? SH : SW;
    out.h = swapsSides(o) ? SW : SH;
    out.px.resize(src.px.size());
    for (int y = 0; y < out.h; y++) {
        uint16_t *row = &out.px[size_t(y) * out.w];
        for (int x = 0; x < out.w; x++) {
            int sx, sy;
            switch (o) {
            case 2: sx = SW - 1 - x; sy = y; break;
            case 3: sx = SW - 1 - x; sy = SH - 1 - y; break;
            case 4: sx = x; sy = SH - 1 - y; break;
            case 5: sx = y; sy = x; break;
            case 6: sx = y; sy = SH - 1 - x; break;
            case 7: sx = SW - 1 - y; sy = SH - 1 - x; break;
            default: sx = SW - 1 - y; sy = x; break;
            }
            row[x] = src.px[size_t(sy) * SW + sx];
        }
    }
    return out;
}

void unorientRect(int o, double W, double H, double &x, double &y, double &w, double &h)
{
    // W x H is the upright size; the stored picture is H x W when the
    // sides swap.
    const double SW = swapsSides(o) ? H : W, SH = swapsSides(o) ? W : H;
    double nx = x, ny = y, nw = w, nh = h;
    switch (o) {
    case 2: nx = SW - (x + w); break;
    case 3: nx = SW - (x + w); ny = SH - (y + h); break;
    case 4: ny = SH - (y + h); break;
    case 5: nx = y; ny = x; nw = h; nh = w; break;
    case 6: nx = y; ny = SH - (x + w); nw = h; nh = w; break;
    case 7: nx = SW - (y + h); ny = SH - (x + w); nw = h; nh = w; break;
    case 8: nx = SW - (y + h); ny = x; nw = h; nh = w; break;
    default: break;
    }
    x = nx;
    y = ny;
    w = nw;
    h = nh;
}

} // namespace gh0st
