// Plays a video from the gallery: the picture fitted to the screen and the
// sound to the speaker or headphones. Three threads while it plays, none
// once it closes: one reads the file, one decodes the picture and keeps it
// on time, one decodes the sound and writes it to the server. Only the
// loop's thread touches the link and the ring; a byte on the wake pipe
// tells it a picture or news is ready.
#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace gh0st {

class Player {
public:
    struct Status {
        int64_t posMs = 0, durMs = 0;
        bool paused = false, ended = false;
        // Set once playing failed; the player has stopped.
        std::string error;
    };

    explicit Player(int wakeFd);
    ~Player();
    // Opens `path` and plays it from the start, fitted in w x h.
    bool open(const std::string &path, int w, int h, std::string &why);
    bool active() const;
    void pause(bool on);
    // To `ms` from the start. Paused stays paused, showing that picture.
    void seek(int64_t ms);
    // Ends playing and frees the decoders and the sound stream.
    void close();
    // The newest picture, if one came since the last call: RGB565, w x h.
    bool picture(std::vector<uint16_t> &px, int &w, int &h);
    // The status, when it changed: each second played, a pause, the end.
    bool news(Status &s);

private:
    struct State;
    std::unique_ptr<State> s_;
    int wake_;
};

} // namespace gh0st
