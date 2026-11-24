#include "ring.h"

#include <cstring>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

namespace gh0st {

static const uint32_t MAGIC = 0x47304352; // "G0CR"
static const uint32_t VERSION = 1;
static const size_t HEAD = 64;
static const size_t SLOT_HEAD = 16;

static uint32_t get32(const uint8_t *p)
{
    uint32_t v;
    memcpy(&v, p, 4);
    return v;
}

bool Ring::open(int fd)
{
    struct stat st;
    if (fstat(fd, &st) != 0 || size_t(st.st_size) < HEAD)
        return false;
    void *p = mmap(nullptr, st.st_size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    ::close(fd);
    if (p == MAP_FAILED)
        return false;
    base_ = static_cast<uint8_t *>(p);
    len_ = st.st_size;
    if (get32(base_) != MAGIC || get32(base_ + 4) != VERSION)
        return false;
    slots_ = get32(base_ + 8);
    slotPx_ = get32(base_ + 12);
    return HEAD + slots_ * (SLOT_HEAD + slotPx_ * 2) <= len_;
}

Ring::~Ring()
{
    if (base_)
        munmap(base_, len_);
}

uint32_t Ring::write(uint32_t slot, uint16_t w, uint16_t h,
                     const std::function<void(uint16_t *px)> &fill)
{
    if (slot >= slots_ || size_t(w) * h > slotPx_)
        return 0;
    seq_ += 2;
    uint8_t *s = base_ + HEAD + slot * (SLOT_HEAD + slotPx_ * 2);
    auto *seq = reinterpret_cast<uint32_t *>(s);
    __atomic_store_n(seq, seq_ | 1, __ATOMIC_RELAXED);
    __atomic_thread_fence(__ATOMIC_RELEASE);
    fill(reinterpret_cast<uint16_t *>(s + SLOT_HEAD));
    memcpy(s + 4, &w, 2);
    memcpy(s + 6, &h, 2);
    __atomic_store_n(seq, seq_, __ATOMIC_RELEASE);
    return seq_;
}

} // namespace gh0st
