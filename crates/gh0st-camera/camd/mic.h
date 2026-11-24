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
    bool open(const std::string &source, AudioTrack &track, std::string &why);
    bool active() const { return thread_.joinable(); }
    // Packets encoded since the last call.
    void take(std::deque<Packet> &out);
    // Stops the capture and encodes what it had.
    void close();
    // Set when the capture stopped on its own.
    bool failed() const { return failed_; }

private:
    void run();

    void *enc_ = nullptr;
    void *in_ = nullptr;
    std::thread thread_;
    std::atomic<bool> quit_{false}, failed_{false};
    std::mutex mu_;
    std::deque<Packet> q_;
};

} // namespace camd
