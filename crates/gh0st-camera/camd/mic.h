// A recording's sound: the microphone to AAC-LC on a thread of its own,
// open only from the start of a recording to its end.
#pragma once

#include "mp4.h"

#include <atomic>
#include <cstdint>
#include <deque>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace camd {

class Mic {
public:
    struct Packet {
        std::vector<uint8_t> data;
        // On the clock the sensor stamps frames with.
        int64_t tsUs;
    };

    ~Mic();
    // `source` empty: the one autoSource picks. Fills `track` for the MP4.
    // The server is reached on the mic's own thread: a cold PipeWire takes
    // half a second, which the preview must not wait out. A server that
    // won't open shows later, in `failed`.
    bool open(const std::string &source, AudioTrack &track, std::string &why);
    bool active() const { return thread_.joinable(); }
    // Packets encoded since the last call.
    void take(std::deque<Packet> &out);
    // Stops the capture and encodes what it had.
    void close();
    // Set when the capture couldn't start or stopped on its own; `why` says.
    bool failed() const { return failed_; }
    std::string why();

private:
    void run();

    void *enc_ = nullptr;
    std::string source_, why_;
    std::thread thread_;
    std::atomic<bool> quit_{false}, failed_{false};
    std::mutex mu_;
    std::deque<Packet> q_;
};

} // namespace camd
