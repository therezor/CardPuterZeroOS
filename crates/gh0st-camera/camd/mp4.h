// Fragmented MP4 for H.264: an empty moov, then a moof and mdat per second.
// A file cut short (a dead battery, a kill) keeps every whole fragment.
// Written by hand: libavformat would pull tens of MB onto the image.
#pragma once

#include <condition_variable>
#include <cstdint>
#include <deque>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace camd {

class Mp4 {
public:
    ~Mp4();
    bool open(const std::string &path, int w, int h, std::string &why);
    // One access unit from the encoder, Annex B, stamped in microseconds.
    // Units before the first keyframe are dropped.
    void add(const uint8_t *data, size_t len, int64_t tsUs, bool key);
    // Writes what is left and closes the file. Blocks until it is on disk.
    void finish();
    bool isOpen() const { return fd_ >= 0; }
    // Whole fragments handed to the disk, and their length so far.
    int fragments() const { return frags_; }
    int64_t durationUs() const { return lastTs_ - firstTs_; }
    uint64_t bytes() const { return written_; }
    // Set once a write fails; the recording should stop.
    bool failed();
    std::string why();

private:
    struct Sample {
        uint32_t size;
        int64_t ts;
        bool key;
    };
    void flush(int64_t nextTs);
    void header();
    void queue(std::vector<uint8_t> &&b);
    void writer();
    uint64_t tick(int64_t tsUs) const;

    int fd_ = -1;
    int w_ = 0, h_ = 0;
    std::vector<uint8_t> sps_, pps_;
    bool headed_ = false;
    std::vector<Sample> samples_;
    std::vector<uint8_t> data_;
    int64_t firstTs_ = -1, lastTs_ = -1;
    uint32_t seq_ = 0;
    uint32_t lastDur_ = 3000;
    int frags_ = 0;
    uint64_t written_ = 0;

    std::mutex mu_;
    std::condition_variable cv_;
    std::deque<std::vector<uint8_t>> q_;
    bool done_ = false, bad_ = false;
    std::string why_;
    std::thread thread_;
};

} // namespace camd
