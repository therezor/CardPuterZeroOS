// Plays an animated WebP, which libavformat 7.1 can't: libwebpdemux
// decodes each frame onto its canvas, a thread keeps them on time, and the
// loop's thread takes the newest picture as from the player.
#pragma once

#include "player.h"

#include <memory>
#include <string>
#include <vector>

namespace mediad {

class Anim {
public:
    explicit Anim(int wakeFd);
    ~Anim();
    bool open(const std::string &path, int w, int h, bool paused, std::string &why);
    bool active() const;
    void pause(bool on);
    // Back to the first frame.
    void restart();
    void close();
    bool picture(std::vector<uint16_t> &px, int &w, int &h);
    bool news(gh0st::Player::Status &s);

private:
    struct State;
    std::unique_ptr<State> s_;
    int wake_;
};

} // namespace mediad
