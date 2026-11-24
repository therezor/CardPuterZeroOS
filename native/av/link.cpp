#include "link.h"

#include <cerrno>
#include <cstdio>
#include <sys/socket.h>
#include <unistd.h>

namespace gh0st {

static const size_t MAX = 8192;

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
        return false;
    int flags = MSG_DONTWAIT;
#ifdef MSG_NOSIGNAL
    flags |= MSG_NOSIGNAL;
#endif
    return ::send(fd_, msg.data(), msg.size(), flags) >= 0;
}

bool Link::recv(std::vector<std::string> &fields)
{
    for (;;) {
        char buf[MAX];
        iovec iov{buf, sizeof buf};
        msghdr h{};
        h.msg_iov = &iov;
        h.msg_iovlen = 1;
        ssize_t n = ::recvmsg(fd_, &h, MSG_DONTWAIT);
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
        // Longer than a peer may send: dropped whole, never half read.
        if (h.msg_flags & MSG_TRUNC)
            continue;
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
}

// The length of the UTF-8 sequence at s[i], or 0 if it isn't one: the
// rules Rust's str::from_utf8 keeps.
static size_t utf8At(const std::string &s, size_t i)
{
    auto b = [&](size_t k) { return k < s.size() ? (unsigned char)s[k] : 0x100u; };
    auto cont = [&](size_t k, unsigned lo, unsigned hi) { return b(k) >= lo && b(k) <= hi; };
    unsigned c = b(i);
    if (c < 0x80)
        return 1;
    if (c >= 0xc2 && c <= 0xdf)
        return cont(i + 1, 0x80, 0xbf) ? 2 : 0;
    if (c == 0xe0)
        return cont(i + 1, 0xa0, 0xbf) && cont(i + 2, 0x80, 0xbf) ? 3 : 0;
    if ((c >= 0xe1 && c <= 0xec) || c == 0xee || c == 0xef)
        return cont(i + 1, 0x80, 0xbf) && cont(i + 2, 0x80, 0xbf) ? 3 : 0;
    if (c == 0xed)
        return cont(i + 1, 0x80, 0x9f) && cont(i + 2, 0x80, 0xbf) ? 3 : 0;
    if (c == 0xf0)
        return cont(i + 1, 0x90, 0xbf) && cont(i + 2, 0x80, 0xbf) && cont(i + 3, 0x80, 0xbf) ? 4
                                                                                             : 0;
    if (c >= 0xf1 && c <= 0xf3)
        return cont(i + 1, 0x80, 0xbf) && cont(i + 2, 0x80, 0xbf) && cont(i + 3, 0x80, 0xbf) ? 4
                                                                                             : 0;
    if (c == 0xf4)
        return cont(i + 1, 0x80, 0x8f) && cont(i + 2, 0x80, 0xbf) && cont(i + 3, 0x80, 0xbf) ? 4
                                                                                             : 0;
    return 0;
}

std::string pathField(const std::string &path)
{
    std::string out;
    char hex[4];
    for (size_t i = 0; i < path.size();) {
        size_t n = utf8At(path, i);
        unsigned char c = path[i];
        if (n == 0 || c == '%' || c < 0x20 || c == 0x7f) {
            snprintf(hex, sizeof hex, "%%%02X", c);
            out += hex;
            i++;
        } else {
            out.append(path, i, n);
            i += n;
        }
    }
    return out;
}

static int hexOf(char c)
{
    if (c >= '0' && c <= '9')
        return c - '0';
    if (c >= 'a' && c <= 'f')
        return c - 'a' + 10;
    if (c >= 'A' && c <= 'F')
        return c - 'A' + 10;
    return -1;
}

std::string pathOf(const std::string &field)
{
    std::string out;
    for (size_t i = 0; i < field.size(); i++) {
        if (field[i] == '%' && i + 2 < field.size()) {
            int h = hexOf(field[i + 1]), l = hexOf(field[i + 2]);
            if (h >= 0 && l >= 0) {
                out += char(h << 4 | l);
                i += 2;
                continue;
            }
        }
        out += field[i];
    }
    return out;
}

} // namespace gh0st
