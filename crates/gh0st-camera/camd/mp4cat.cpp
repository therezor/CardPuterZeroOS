// Wraps a raw H.264 stream (Annex B, one slice per frame) in our MP4 at
// 30 fps, with an ADTS AAC stream as its sound: a host check of mp4.cpp
// against ffprobe.
//   make mp4cat O=out && out/mp4cat in.h264 out.mp4 [in.aac]
#include "mp4.h"

#include <cstdio>
#include <fstream>
#include <iterator>

int main(int argc, char **argv)
{
    if (argc != 3 && argc != 4) {
        fprintf(stderr, "usage: mp4cat in.h264 out.mp4 [in.aac]\n");
        return 2;
    }
    std::ifstream f(argv[1], std::ios::binary);
    std::vector<uint8_t> in((std::istreambuf_iterator<char>(f)), {});
    // ADTS frames: (offset of the raw frame, its length).
    std::vector<uint8_t> aac;
    std::vector<std::pair<size_t, size_t>> frames;
    camd::AudioTrack track;
    if (argc == 4) {
        std::ifstream a(argv[3], std::ios::binary);
        aac.assign(std::istreambuf_iterator<char>(a), {});
        for (size_t i = 0; i + 7 <= aac.size();) {
            if (aac[i] != 0xff || (aac[i + 1] & 0xf0) != 0xf0)
                return fprintf(stderr, "not ADTS at %zu\n", i), 1;
            size_t head = aac[i + 1] & 1 ? 7 : 9;
            size_t len = (aac[i + 3] & 3) << 11 | aac[i + 4] << 3 | aac[i + 5] >> 5;
            if (len < head || i + len > aac.size())
                break;
            if (frames.empty()) {
                int aot = (aac[i + 2] >> 6) + 1, sri = (aac[i + 2] >> 2) & 15;
                int ch = (aac[i + 2] & 1) << 2 | aac[i + 3] >> 6;
                static const int rates[] = {96000, 88200, 64000, 48000, 44100, 32000,
                                            24000, 22050, 16000, 12000, 11025, 8000};
                track.rate = sri < 12 ? rates[sri] : 48000;
                track.channels = ch;
                track.asc = {uint8_t(aot << 3 | sri >> 1), uint8_t((sri & 1) << 7 | ch << 3)};
            }
            frames.emplace_back(i + head, len - head);
            i += len;
        }
    }
    camd::Mp4 mp4;
    std::string why;
    if (!mp4.open(argv[2], 1920, 1080, frames.empty() ? nullptr : &track, why)) {
        fprintf(stderr, "%s\n", why.c_str());
        return 1;
    }
    // Access units end at each slice: everything up to and including it.
    size_t au = 0, i = 0, sent = 0;
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
            int64_t ts = int64_t(frame) * 33333;
            // The sound up to this frame, as a recording interleaves it.
            while (sent < frames.size() &&
                   int64_t(sent) * 1024 * 1000000 / track.rate <= ts) {
                if (key && sent == 0 && frame == 0)
                    break;
                auto &fr = frames[sent++];
                mp4.addAudio(&aac[fr.first], fr.second,
                             int64_t(sent - 1) * 1024 * 1000000 / track.rate);
            }
            mp4.add(&in[au], next - au, ts, key);
            frame++;
            key = false;
            au = next;
        }
        i = next;
    }
    for (; sent < frames.size(); sent++)
        mp4.addAudio(&aac[frames[sent].first], frames[sent].second,
                     int64_t(sent) * 1024 * 1000000 / track.rate);
    mp4.finish();
    printf("%zu audio frames, ", sent);
    printf("%d frames, %d fragments, %llu bytes\n", frame, mp4.fragments(),
           (unsigned long long)mp4.bytes());
    return 0;
}
