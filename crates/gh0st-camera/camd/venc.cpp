#include "venc.h"
#include "camd.h"

#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <linux/videodev2.h>
#include <poll.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

namespace camd {

namespace {

const char *DEVICE = "/dev/video11";
// Frames in flight to the encoder, two fewer than the camera's video
// buffers (VIDEO_BUFS), and buffers for what it makes, which the writer
// takes at once. 512 KB holds any 1080p frame at our rates, as rpicam-vid.
// Fewer of either dropped frames at 1080p30 (hardware-facts.md, Camera).
const unsigned IN_SLOTS = 3;
const unsigned OUT_BUFS = 6;
const unsigned OUT_SIZE = 512 << 10;

int xioctl(int fd, unsigned long req, void *arg)
{
    int r;
    do
        r = ioctl(fd, req, arg);
    while (r < 0 && errno == EINTR);
    return r;
}

bool control(int fd, uint32_t id, int32_t value)
{
    v4l2_control c = {};
    c.id = id;
    c.value = value;
    return xioctl(fd, VIDIOC_S_CTRL, &c) == 0;
}

} // namespace

Venc::~Venc()
{
    close([](void *) {});
}

bool Venc::open(int w, int h, int stride, int fps, int bitrate, std::string &why)
{
    auto fail = [&](const char *what) {
        why = std::string(what) + ": " + strerror(errno);
        close([](void *) {});
        return false;
    };
    fd_ = ::open(DEVICE, O_RDWR | O_NONBLOCK | O_CLOEXEC);
    if (fd_ < 0)
        return fail("encoder");
    control(fd_, V4L2_CID_MPEG_VIDEO_BITRATE_MODE, V4L2_MPEG_VIDEO_BITRATE_MODE_VBR);
    if (!control(fd_, V4L2_CID_MPEG_VIDEO_BITRATE, bitrate))
        return fail("bitrate");
    control(fd_, V4L2_CID_MPEG_VIDEO_H264_PROFILE, V4L2_MPEG_VIDEO_H264_PROFILE_HIGH);
    if (!control(fd_, V4L2_CID_MPEG_VIDEO_H264_LEVEL, V4L2_MPEG_VIDEO_H264_LEVEL_4_1))
        control(fd_, V4L2_CID_MPEG_VIDEO_H264_LEVEL, V4L2_MPEG_VIDEO_H264_LEVEL_4_0);
    // A keyframe each second: each MP4 fragment starts on one.
    control(fd_, V4L2_CID_MPEG_VIDEO_H264_I_PERIOD, fps);
    control(fd_, V4L2_CID_MPEG_VIDEO_REPEAT_SEQ_HEADER, 1);

    v4l2_format f = {};
    f.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    f.fmt.pix_mp.width = w;
    f.fmt.pix_mp.height = h;
    f.fmt.pix_mp.pixelformat = V4L2_PIX_FMT_YUV420;
    f.fmt.pix_mp.field = V4L2_FIELD_NONE;
    f.fmt.pix_mp.colorspace = V4L2_COLORSPACE_REC709;
    f.fmt.pix_mp.num_planes = 1;
    f.fmt.pix_mp.plane_fmt[0].bytesperline = stride;
    if (xioctl(fd_, VIDIOC_S_FMT, &f) < 0)
        return fail("input format");
    f = {};
    f.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    f.fmt.pix_mp.width = w;
    f.fmt.pix_mp.height = h;
    f.fmt.pix_mp.pixelformat = V4L2_PIX_FMT_H264;
    f.fmt.pix_mp.field = V4L2_FIELD_NONE;
    f.fmt.pix_mp.num_planes = 1;
    f.fmt.pix_mp.plane_fmt[0].sizeimage = OUT_SIZE;
    if (xioctl(fd_, VIDIOC_S_FMT, &f) < 0)
        return fail("output format");
    v4l2_streamparm parm = {};
    parm.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    parm.parm.output.timeperframe.numerator = 1000;
    parm.parm.output.timeperframe.denominator = fps * 1000;
    xioctl(fd_, VIDIOC_S_PARM, &parm);

    v4l2_requestbuffers rb = {};
    rb.count = IN_SLOTS;
    rb.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    rb.memory = V4L2_MEMORY_DMABUF;
    if (xioctl(fd_, VIDIOC_REQBUFS, &rb) < 0)
        return fail("input buffers");
    slots_.assign(rb.count, nullptr);
    rb = {};
    rb.count = OUT_BUFS;
    rb.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    rb.memory = V4L2_MEMORY_MMAP;
    if (xioctl(fd_, VIDIOC_REQBUFS, &rb) < 0)
        return fail("output buffers");
    for (unsigned i = 0; i < rb.count; i++) {
        v4l2_plane pl[VIDEO_MAX_PLANES] = {};
        v4l2_buffer b = {};
        b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.index = i;
        b.length = 1;
        b.m.planes = pl;
        if (xioctl(fd_, VIDIOC_QUERYBUF, &b) < 0)
            return fail("query");
        void *p = mmap(nullptr, pl[0].length, PROT_READ | PROT_WRITE, MAP_SHARED, fd_,
                       pl[0].m.mem_offset);
        if (p == MAP_FAILED)
            return fail("map");
        caps_.push_back({p, pl[0].length});
        if (xioctl(fd_, VIDIOC_QBUF, &b) < 0)
            return fail("queue");
    }
    int type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    if (xioctl(fd_, VIDIOC_STREAMON, &type) < 0)
        return fail("stream in");
    type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    if (xioctl(fd_, VIDIOC_STREAMON, &type) < 0)
        return fail("stream out");
    last_ = false;
    return true;
}

bool Venc::push(int dmabuf, size_t size, int64_t tsUs, void *token)
{
    unsigned i = 0;
    while (i < slots_.size() && slots_[i])
        i++;
    if (i == slots_.size())
        return false;
    v4l2_plane pl[VIDEO_MAX_PLANES] = {};
    v4l2_buffer b = {};
    b.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    b.memory = V4L2_MEMORY_DMABUF;
    b.index = i;
    b.field = V4L2_FIELD_NONE;
    b.length = 1;
    b.timestamp.tv_sec = tsUs / 1000000;
    b.timestamp.tv_usec = tsUs % 1000000;
    b.m.planes = pl;
    pl[0].m.fd = dmabuf;
    pl[0].bytesused = size;
    pl[0].length = size;
    if (xioctl(fd_, VIDIOC_QBUF, &b) < 0)
        return false;
    slots_[i] = token;
    return true;
}

void Venc::service(const Released &released, const Unit &unit)
{
    for (;;) {
        v4l2_plane pl[VIDEO_MAX_PLANES] = {};
        v4l2_buffer b = {};
        b.type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
        b.memory = V4L2_MEMORY_DMABUF;
        b.length = 1;
        b.m.planes = pl;
        if (xioctl(fd_, VIDIOC_DQBUF, &b) < 0)
            break;
        if (b.index < slots_.size() && slots_[b.index]) {
            void *t = slots_[b.index];
            slots_[b.index] = nullptr;
            released(t);
        }
    }
    for (;;) {
        v4l2_plane pl[VIDEO_MAX_PLANES] = {};
        v4l2_buffer b = {};
        b.type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        b.memory = V4L2_MEMORY_MMAP;
        b.length = 1;
        b.m.planes = pl;
        if (xioctl(fd_, VIDIOC_DQBUF, &b) < 0)
            break;
        if (b.flags & V4L2_BUF_FLAG_LAST)
            last_ = true;
        if (pl[0].bytesused && b.index < caps_.size()) {
            int64_t ts = int64_t(b.timestamp.tv_sec) * 1000000 + b.timestamp.tv_usec;
            unit(static_cast<const uint8_t *>(caps_[b.index].p) + pl[0].data_offset,
                 pl[0].bytesused - pl[0].data_offset, ts, b.flags & V4L2_BUF_FLAG_KEYFRAME);
        }
        if (!last_) {
            pl[0].bytesused = 0;
            xioctl(fd_, VIDIOC_QBUF, &b);
        }
    }
}

void Venc::drain(int ms, const Released &released, const Unit &unit)
{
    if (fd_ < 0)
        return;
    v4l2_encoder_cmd cmd = {};
    cmd.cmd = V4L2_ENC_CMD_STOP;
    bool asked = xioctl(fd_, VIDIOC_ENCODER_CMD, &cmd) == 0;
    uint64_t end = nowMs() + ms;
    while (asked && !last_ && nowMs() < end) {
        pollfd p{fd_, POLLIN, 0};
        poll(&p, 1, int(end - nowMs()));
        service(released, unit);
    }
    service(released, unit);
}

void Venc::close(const Released &released)
{
    if (fd_ < 0)
        return;
    int type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
    xioctl(fd_, VIDIOC_STREAMOFF, &type);
    type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    xioctl(fd_, VIDIOC_STREAMOFF, &type);
    // Off, every frame it still held is ours again.
    for (void *&t : slots_)
        if (t) {
            released(t);
            t = nullptr;
        }
    for (auto &m : caps_)
        munmap(m.p, m.len);
    caps_.clear();
    slots_.clear();
    ::close(fd_);
    fd_ = -1;
}

} // namespace camd
