// The hardware H.264 encoder (bcm2835-codec, /dev/video11) through V4L2
// mem2mem. Frames go in as the ISP's own dmabufs, so nothing is copied;
// each comes back (by its token) once the encoder is done with it.
#pragma once

#include <cstddef>
#include <cstdint>
#include <functional>
#include <string>
#include <vector>

namespace camd {

class Venc {
public:
    using Released = std::function<void(void *token)>;
    using Unit = std::function<void(const uint8_t *data, size_t len, int64_t tsUs, bool key)>;

    ~Venc();
    bool open(int w, int h, int stride, int fps, int bitrate, std::string &why);
    int fd() const { return fd_; }
    // Queues a frame. False when every input slot is in use: drop it.
    bool push(int dmabuf, size_t size, int64_t tsUs, void *token);
    // Takes back what the encoder is done with, and what it made.
    void service(const Released &released, const Unit &unit);
    // Asks for the frames still inside, waits up to `ms` for the last one,
    // and hands every token back.
    void drain(int ms, const Released &released, const Unit &unit);
    void close(const Released &released);
    bool isOpen() const { return fd_ >= 0; }

private:
    int fd_ = -1;
    std::vector<void *> slots_;
    struct Map {
        void *p;
        size_t len;
    };
    std::vector<Map> caps_;
    bool last_ = false;
};

} // namespace camd
