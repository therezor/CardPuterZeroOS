// The app's link: one message per packet, fields split by tabs, at most
// 8 KB. The same wire as gh0st_core::link, version 2: a message too long is
// refused, never cut. A path field is escaped (pathField, pathOf) where
// a protocol says so: the media helper's does.
#pragma once

#include <string>
#include <vector>

namespace gh0st {

class Link {
public:
    explicit Link(int fd);
    ~Link();
    int fd() const { return fd_; }
    // False when it didn't go: too long, or nobody reads.
    bool send(const std::vector<std::string> &fields) const;
    // The next message. False when none waits; closed() says if the app
    // hung up. Empty fields are kept: `rec start <path> ` has one.
    bool recv(std::vector<std::string> &fields);
    bool closed() const { return closed_; }

private:
    int fd_;
    bool closed_ = false;
};

// A path as one field: %XX for %, control bytes and bytes that are not
// UTF-8, as gh0st_core::link::path_field writes it.
std::string pathField(const std::string &path);
// The path a field names. A broken escape stays as it is.
std::string pathOf(const std::string &field);

} // namespace gh0st
