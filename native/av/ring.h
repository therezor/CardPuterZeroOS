// The frame ring the app made: the writer's side of src/ring.rs.
#pragma once

#include <cstddef>
#include <cstdint>
#include <functional>

namespace gh0st {

class Ring {
public:
    // Maps and checks the ring behind an inherited fd. False if it isn't one.
    bool open(int fd);
    ~Ring();
    uint32_t slots() const { return slots_; }
    size_t slotPx() const { return slotPx_; }
    // Writes a w x h frame into `slot` through `fill` and returns the
    // sequence number to announce.
    uint32_t write(uint32_t slot, uint16_t w, uint16_t h,
                   const std::function<void(uint16_t *px)> &fill);

private:
    uint8_t *base_ = nullptr;
    size_t len_ = 0;
    uint32_t slots_ = 0;
    size_t slotPx_ = 0;
    uint32_t seq_ = 0;
};

} // namespace gh0st
