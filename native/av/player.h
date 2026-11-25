// Plays a video or a song: the picture fitted to the screen and the sound
// to the speaker or headphones. Three threads while it plays, none once it
// closes: one reads the file, one decodes the picture and keeps it on time,
// one decodes the sound and writes it to the server. Only the loop's
// thread touches the link and the ring; a byte on the wake pipe tells it a
// picture or news is ready.
#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace gh0st {

class Player {
public:
    // A picture larger than this is refused before a decoder starts its
    // threads: twice 1080p, where a software decoder's threads already
    // hold 95 MB (docs/hardware-facts.md, Media).
    static constexpr int64_t MAX_PIXELS = 2 * 1920 * 1080;
    // A .srt beside the file is read whole as it opens; a larger one is
    // left out.
    static constexpr int64_t MAX_SRT_BYTES = 4 << 20;

    struct Options {
        // EXIF orientation the pictures are turned by, 1 upright.
        int orient = 1;
        // At the end, from the start again: a moving picture.
        bool loop = false;
        // No sound, even when the file has some.
        bool silent = false;
        // Opens, shows its first picture and waits for pause(false).
        bool paused = false;
        // The sound track, counted among the sound tracks; -1 the best.
        int audio = -1;
        // The subtitle track, counted among the subtitle tracks, a .srt
        // beside the file after them; -1 none.
        int subs = -1;
        int64_t startMs = 0;
        // Never the VPU: measures what software decoding keeps up with.
        bool software = false;
    };

    struct Status {
        int64_t posMs = 0, durMs = 0;
        bool paused = false, ended = false;
        // Pictures dropped for being late, since it opened.
        int dropped = 0;
        // How much decoding it skips to keep up: 0 none, 4 key frames only.
        int slow = 0;
        // Set once playing failed; the player has stopped.
        std::string error;
    };

    explicit Player(int wakeFd);
    ~Player();
    // Opens `path` and plays it from the start, fitted in w x h.
    bool open(const std::string &path, int w, int h, std::string &why);
    bool open(const std::string &path, int w, int h, const Options &o, std::string &why);
    bool active() const;
    // False for sound alone.
    bool hasPicture() const;
    void pause(bool on);
    // To `ms` from the start. Paused stays paused, showing that picture.
    void seek(int64_t ms);
    // 50 to 200: half speed to double, the sound at its own pitch.
    void speed(int percent);
    // Plays a to b over and over; b <= a stops it.
    void repeat(int64_t a, int64_t b);
    // Paused: the next picture.
    void step();
    // Ends playing and frees the decoders and the sound stream.
    void close();
    // The newest picture, if one came since the last call: RGB565, w x h.
    bool picture(std::vector<uint16_t> &px, int &w, int &h);
    // The status, when it changed: each second played, a pause, the end.
    bool news(Status &s);
    // Where it is now, for opening it again with other tracks.
    int64_t position() const;

private:
    struct State;
    std::unique_ptr<State> s_;
    int wake_;
};

} // namespace gh0st
