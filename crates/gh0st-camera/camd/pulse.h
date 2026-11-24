// Sound in and out through PulseAudio's simple API, which PipeWire
// answers. PipeWire is started by its socket only when one of these opens,
// and its node sleeps again once they close.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace camd {

struct Source {
    std::string name, desc;
};

// The microphones there are, monitors left out. Empty on no server.
std::vector<Source> sources();

// The one a recording takes when the user left it to us: a plugged-in one
// (USB, Bluetooth) over the board's own, else the server's default ("").
// The headset jack needs nothing here: the sound driver moves the board's
// own input to it when something is plugged in.
std::string autoSource(const std::vector<Source> &all);

class PaIn {
public:
    ~PaIn();
    // `dev` empty: the server's default.
    bool open(const std::string &dev, int rate, int channels, std::string &why);
    // Blocks for `frames` frames of S16.
    bool read(int16_t *buf, int frames);
    // How long the newest frame read waited in the server.
    int64_t latencyUs();
    void close();

private:
    void *s_ = nullptr;
    int bytesPerFrame_ = 2;
    int rate_ = 0;
};

class PaOut {
public:
    ~PaOut();
    bool open(int rate, int channels, std::string &why);
    // Blocks while the server's buffer is full, so it paces the caller.
    bool write(const int16_t *buf, int frames);
    // What is written but not yet heard.
    int64_t latencyUs();
    // Drops what is buffered: a seek or a pause.
    void flush();
    void close();

private:
    void *s_ = nullptr;
    int bytesPerFrame_ = 2;
    int rate_ = 0;
};

} // namespace camd
