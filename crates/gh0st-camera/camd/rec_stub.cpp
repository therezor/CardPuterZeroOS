// Hosts have no hardware encoder: recording says so.
#include "rec.h"

namespace camd {

struct Recorder::State {
    std::string path;
};

Recorder::Recorder(Backend &) : s_(std::make_unique<State>()) {}
Recorder::~Recorder() = default;
bool Recorder::start(const std::string &, std::string &why)
{
    why = "no video encoder on this host";
    return false;
}
bool Recorder::active() const { return false; }
int Recorder::fd() const { return -1; }
bool Recorder::frame(const VideoFrame &) { return false; }
bool Recorder::service() { return false; }
void Recorder::stop() {}
int64_t Recorder::ms() const { return 0; }
uint64_t Recorder::bytes() const { return 0; }
int Recorder::dropped() const { return 0; }
bool Recorder::failed() { return false; }
std::string Recorder::why() { return ""; }
const std::string &Recorder::path() const { return s_->path; }

} // namespace camd
