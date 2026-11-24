// Hosts have no sound server for us: no microphones, and playback is
// silent but keeps time, so the picture still plays at its pace.
#include "pulse.h"

#include <chrono>
#include <thread>

namespace gh0st {

std::vector<Source> sources()
{
    return {};
}

std::string autoSource(const std::vector<Source> &)
{
    return "";
}

PaIn::~PaIn() = default;
bool PaIn::open(const std::string &, int rate, int channels, std::string &why)
{
    rate_ = rate;
    bytesPerFrame_ = 2 * channels;
    why = "no sound server on this host";
    return false;
}
bool PaIn::read(int16_t *, int)
{
    return s_ != nullptr;
}
int64_t PaIn::latencyUs()
{
    return 0;
}
void PaIn::close() {}

PaOut::~PaOut() = default;
bool PaOut::open(int rate, int channels, std::string &)
{
    s_ = this;
    rate_ = rate;
    bytesPerFrame_ = 2 * channels;
    return true;
}
bool PaOut::write(const int16_t *, int frames)
{
    std::this_thread::sleep_for(std::chrono::microseconds(int64_t(frames) * 1000000 / rate_));
    return s_ != nullptr;
}
int64_t PaOut::latencyUs()
{
    return 0;
}
void PaOut::flush() {}
void PaOut::close()
{
    s_ = nullptr;
}

} // namespace gh0st
