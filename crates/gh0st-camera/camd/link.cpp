#include "link.h"

#include <cerrno>
#include <sys/socket.h>
#include <unistd.h>

namespace camd {

static const size_t MAX = 512;

Link::Link(int fd) : fd_(fd) {}

Link::~Link()
{
    if (fd_ >= 0)
        ::close(fd_);
}

bool Link::send(const std::vector<std::string> &fields) const
{
    std::string msg;
    for (size_t i = 0; i < fields.size(); i++) {
        if (i)
            msg += '\t';
        msg += fields[i];
    }
    if (msg.size() > MAX)
        msg.resize(MAX);
    int flags = MSG_DONTWAIT;
#ifdef MSG_NOSIGNAL
    flags |= MSG_NOSIGNAL;
#endif
    return ::send(fd_, msg.data(), msg.size(), flags) >= 0;
}

bool Link::recv(std::vector<std::string> &fields)
{
    char buf[MAX];
    ssize_t n = ::recv(fd_, buf, sizeof buf, MSG_DONTWAIT);
    if (n < 0) {
        if (errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)
            closed_ = true;
        return false;
    }
    // The app never sends an empty message: none is the end.
    if (n == 0) {
        closed_ = true;
        return false;
    }
    fields.clear();
    std::string cur;
    for (ssize_t i = 0; i < n; i++) {
        if (buf[i] == '\t') {
            fields.push_back(cur);
            cur.clear();
        } else {
            cur += buf[i];
        }
    }
    fields.push_back(cur);
    return true;
}

} // namespace camd
