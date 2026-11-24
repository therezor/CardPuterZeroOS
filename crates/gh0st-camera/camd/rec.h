// A recording: full-size frames from the backend through the hardware
// encoder, and the microphone through AAC, into a fragmented MP4.
#pragma once

#include "camd.h"

#include <memory>
#include <string>

namespace camd {

class Recorder {
public:
    explicit Recorder(Backend &be);
    ~Recorder();
    // Starts on the backend's video stream, writing `path`. With `mic`,
    // sound from `source` (empty: autoSource's pick). A microphone that
    // won't open leaves the video silent and says why in `micWhy`.
    bool start(const std::string &path, bool mic, const std::string &source, std::string &why,
               std::string &micWhy);
    bool active() const;
    // The encoder's fd to poll while active; -1 otherwise.
    int fd() const;
    // A new full-size frame. False: not taken, the backend keeps it.
    bool frame(const VideoFrame &f);
    // After the fd polled. True when another second reached the file.
    bool service();
    // Drains the encoder, writes the last second, closes the file.
    void stop();
    int64_t ms() const;
    uint64_t bytes() const;
    // Frames dropped because the encoder was busy.
    int dropped() const;
    bool failed();
    std::string why();
    // The microphone failed since the last call, and why; once each.
    bool micLost(std::string &why);
    const std::string &path() const;

private:
    struct State;
    std::unique_ptr<State> s_;
};

} // namespace camd
