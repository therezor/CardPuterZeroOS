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
    // A tile used counts as new again, once a day at most: the card is
    // not written on every look.
    bool get(const std::string &path, int w, int h, gh0st::Pic &out, bool &bad) const;
    // Keeps a tile, or with `pic` null marks the file as one that failed.
    // One that takes the folder past trim's `most` trims it again.
    void put(const std::string &path, int w, int h, const gh0st::Pic *pic);
    // The least used first until the folder holds at most `keep` bytes,
    // once it holds more than `most`.
    void trim(size_t most, size_t keep);

    static std::string defaultDir();

private:
    std::string name(const std::string &path, int w, int h) const;
    std::string dir_;
    // The folder's size, counted by trim and added to by put, and the
    // caps trim was last given.
    size_t total_ = 0, most_ = 0, keep_ = 0;
};

} // namespace mediad
