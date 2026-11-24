// Tiles drawn before, on the card: ~/.cache/gh0st-media/thumbs. A grid of a
// folder seen before fills without decoding anything.
#pragma once

#include "pix.h"

#include <cstddef>
#include <string>

namespace mediad {

class ThumbCache {
public:
    // `dir` empty: nothing is kept.
    explicit ThumbCache(std::string dir);
    // A tile of `path` at w x h. `bad`: the file gave no picture before.
    bool get(const std::string &path, int w, int h, gh0st::Pic &out, bool &bad) const;
    // Keeps a tile, or with `pic` null marks the file as one that failed.
    void put(const std::string &path, int w, int h, const gh0st::Pic *pic) const;
    // Oldest first until the folder holds at most `keep` bytes, once it
    // holds more than `most`.
    void trim(size_t most, size_t keep) const;

    static std::string defaultDir();

private:
    std::string name(const std::string &path, int w, int h) const;
    std::string dir_;
};

} // namespace mediad
