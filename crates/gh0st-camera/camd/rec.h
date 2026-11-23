// A recording: full-size frames from the backend through the hardware
// encoder into a fragmented MP4.
#pragma once

#include "camd.h"

#include <memory>
#include <string>

namespace camd {

class Recorder {
public:
    explicit Recorder(Backend &be);
    ~Recorder();
    // Starts on the backend's video stream, writing `path`.
    bool start(const std::string &path, std::string &why);
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
    const std::string &path() const;

private:
    struct State;
    std::unique_ptr<State> s_;
};

} // namespace camd
