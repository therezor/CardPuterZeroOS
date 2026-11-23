// Wraps a raw H.264 stream (Annex B, one slice per frame) in our MP4 at
// 30 fps: a host check of mp4.cpp against ffprobe.
//   make mp4cat O=out && out/mp4cat in.h264 out.mp4
#include "mp4.h"

#include <cstdio>
#include <fstream>
#include <iterator>

int main(int argc, char **argv)
{
    if (argc != 3) {
        fprintf(stderr, "usage: mp4cat in.h264 out.mp4\n");
        return 2;
    }
    std::ifstream f(argv[1], std::ios::binary);
    std::vector<uint8_t> in((std::istreambuf_iterator<char>(f)), {});
    camd::Mp4 mp4;
    std::string why;
    if (!mp4.open(argv[2], 1920, 1080, why)) {
        fprintf(stderr, "%s\n", why.c_str());
        return 1;
    }
    // Access units end at each slice: everything up to and including it.
    size_t au = 0, i = 0;
    int frame = 0;
    bool key = false;
    while (i + 4 < in.size()) {
        bool start = in[i] == 0 && in[i + 1] == 0 && in[i + 2] == 1;
        if (!start) {
            i++;
            continue;
        }
        int type = in[i + 3] & 0x1f;
        size_t next = i + 3;
        while (next + 3 <= in.size() && !(in[next] == 0 && in[next + 1] == 0 && in[next + 2] == 1))
            next++;
        if (next + 3 > in.size())
            next = in.size();
        if (type == 5)
            key = true;
        if (type == 1 || type == 5) {
            mp4.add(&in[au], next - au, int64_t(frame) * 33333, key);
            frame++;
            key = false;
            au = next;
        }
        i = next;
    }
    mp4.finish();
    printf("%d frames, %d fragments, %llu bytes\n", frame, mp4.fragments(),
           (unsigned long long)mp4.bytes());
    return 0;
}
