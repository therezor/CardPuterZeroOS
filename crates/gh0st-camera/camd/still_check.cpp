// Host checks for still.cpp's readers of files from elsewhere: a crafted
// EXIF and a huge picture. `make check` builds and runs them.
#include "still.cpp"

#include <cstdio>
#include <cstdlib>

namespace {

int failed = 0;

void expect(bool ok, const char *what)
{
    std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
    failed += !ok;
}

// An APP1 whose IFD1 points the thumbnail at 0xfffffff0 for 32 bytes: the
// 32-bit sum wraps to 16, inside the segment.
std::vector<uint8_t> wrappingExif()
{
    std::vector<uint8_t> b = {0xff, 0xd8, 0xff, 0xe1, 0x00, 0x40, 'E', 'x', 'i', 'f', 0, 0};
    const uint8_t tiff[] = {'I',  'I',  42,   0,    8,    0,    0,    0,    0, 0, 14, 0, 0, 0,
                            2,    0,    0x01, 0x02, 4,    0,    1,    0,    0, 0, 0xf0, 0xff,
                            0xff, 0xff, 0x02, 0x02, 4,    0,    1,    0,    0, 0, 0x20, 0,
                            0,    0,    0,    0,    0,    0};
    b.insert(b.end(), tiff, tiff + sizeof tiff);
    b.resize(200, 0);
    return b;
}

// A real 8x8 JPEG whose frame header then claims 60000x60000.
std::string hugeJpeg()
{
    jpeg_compress_struct c;
    jpeg_error_mgr e;
    c.err = jpeg_std_error(&e);
    jpeg_create_compress(&c);
    unsigned char *mem = nullptr;
    unsigned long n = 0;
    jpeg_mem_dest(&c, &mem, &n);
    c.image_width = c.image_height = 8;
    c.input_components = 1;
    c.in_color_space = JCS_GRAYSCALE;
    jpeg_set_defaults(&c);
    jpeg_start_compress(&c, TRUE);
    uint8_t row[8] = {};
    for (int y = 0; y < 8; y++) {
        JSAMPROW r = row;
        jpeg_write_scanlines(&c, &r, 1);
    }
    jpeg_finish_compress(&c);
    jpeg_destroy_compress(&c);
    for (unsigned long i = 0; i + 8 < n; i++)
        if (mem[i] == 0xff && mem[i + 1] == 0xc0) {
            mem[i + 5] = mem[i + 7] = 60000 >> 8;
            mem[i + 6] = mem[i + 8] = 60000 & 0xff;
            break;
        }
    std::string path = std::string(std::getenv("TMPDIR") ? std::getenv("TMPDIR") : "/tmp") +
                       "/gh0st-camd-huge.jpg";
    FILE *f = std::fopen(path.c_str(), "wb");
    std::fwrite(mem, 1, n, f);
    std::fclose(f);
    std::free(mem);
    return path;
}

} // namespace

int main()
{
    std::vector<uint8_t> b = wrappingExif();
    FILE *f = std::tmpfile();
    std::fwrite(b.data(), 1, b.size(), f);
    std::rewind(f);
    std::vector<uint8_t> jpg;
    expect(!camd::exifThumb(f, jpg), "a thumbnail offset that wraps is refused");
    std::fclose(f);

    std::string path = hugeJpeg();
    std::vector<uint16_t> out;
    expect(!camd::jpegTo565(path, 320, 170, 1, 0.5f, 0.5f, out), "a 3.6 GP picture is skipped");
    std::remove(path.c_str());
    return failed ? 1 : 0;
}
