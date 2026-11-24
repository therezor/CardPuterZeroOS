// The app's link: one message per packet, fields split by tabs, at most
// 512 bytes. The same wire as gh0st_core::link.
#pragma once

#include <string>
#include <vector>

namespace gh0st {

class Link {
public:
    explicit Link(int fd);
    ~Link();
    int fd() const { return fd_; }
    bool send(const std::vector<std::string> &fields) const;
    // The next message. False when none waits; closed() says if the app
    // hung up. Empty fields are kept: `rec start <path> ` has one.
    bool recv(std::vector<std::string> &fields);
    bool closed() const { return closed_; }

private:
    int fd_;
    bool closed_ = false;
};

} // namespace gh0st
