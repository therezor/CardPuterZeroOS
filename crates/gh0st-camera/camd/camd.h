// gh0st-camd: the camera helper. Shared types; docs/camera.md has the
// protocol and the reasons.
#pragma once

#include "pix.h"

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace camd {

// Pictures, the player, the link and the ring are shared with gh0st-mediad
// (native/av).
using namespace gh0st;

enum class Fault { None, NoCamera, NoMemory, Busy, Other };

enum class Mode { Photo, Video1080, Video720 };

struct Controls {
    float zoom = 1.0f;
    float ev = 0.0f;
    // Mains frequency against flicker: 0 leaves it to the IPA.
    int flicker = 0;
    int quality = 95;
};

struct ShotMeta {
    int64_t exposure_us = 0;
    float gain = 1.0f;
    float zoom = 1.0f;
    float ev = 0.0f;
    std::string model;
};

// A full-size video frame as the encoder takes it: one dmabuf and its
// length, when the sensor began it, the planes for a poster, and a token
// that gives it back (Backend::release).
struct VideoFrame {
    int fd;
    size_t size;
    int64_t tsUs;
    void *token;
    View view;
};

// The sensor. Everything runs on the loop's thread; a backend writes a
// byte to the wake fd when a frame is ready.
class Backend {
public:
    virtual ~Backend() = default;
    // Finds and claims the sensor.
    virtual Fault open() = 0;
    virtual std::string sensor() const = 0;
    virtual int fullW() const = 0;
    virtual int fullH() const = 0;
    // Streams the preview for `mode`, aimed at w x h.
    virtual Fault preview(Mode mode, int w, int h, const Controls &c) = 0;
    virtual void set(const Controls &c) = 0;
    // Stops the preview and takes one still. The sensor is left stopped.
    virtual Fault still(const Controls &c, Yuv420 &out, ShotMeta &meta, std::string &why) = 0;
    // Streaming off, buffers freed. The sensor stays claimed.
    virtual void stop() = 0;
    // The sensor let go.
    virtual void close() = 0;
    // Hands the newest preview frame to `sink`, if one came, and in a video
    // mode every full-size frame to `video`: one it takes (true) stays out
    // until release(). Called on a wake.
    virtual void frames(const std::function<void(const View &)> &sink,
                        const std::function<bool(const VideoFrame &)> &video) = 0;
    virtual void release(void *) {}
    // The full-size stream of a video mode: false in photo mode.
    virtual bool videoStream(int &, int &, int &) const { return false; }
    // Frames the sensor made that never reached us, since it opened.
    virtual uint32_t lostFrames() const { return 0; }
};

std::unique_ptr<Backend> makeBackend(int wakeFd);
// `gh0st-camd --probe [DIR]`: what the sensor and this board can do.
int probe(const char *dir);

uint64_t nowMs();

} // namespace camd
