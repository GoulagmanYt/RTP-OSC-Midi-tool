#pragma once

#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>

namespace rack_realtime {

// One audio producer and one control consumer. Allocation belongs to the
// parameter table at initialization; publication never allocates or locks.
// Coalescing retains the latest value even across arbitrarily long sessions.
class ParameterMailbox {
public:
    bool publish(double value) noexcept {
        if (!std::isfinite(value) || value < 0.0 || value > 1.0) return false;
        uint64_t bits = 0;
        std::memcpy(&bits, &value, sizeof(bits));
        // Normalized values have no sign bit; -0.0 is restored as +0.0.
        latest_.store(bits | dirty_bit, std::memory_order_release);
        return true;
    }

    bool take_latest(double& value) noexcept {
        uint64_t bits = latest_.exchange(0, std::memory_order_acq_rel);
        if ((bits & dirty_bit) == 0) return false;
        bits &= ~dirty_bit;
        std::memcpy(&value, &bits, sizeof(value));
        return true;
    }

private:
    static_assert(sizeof(double) == sizeof(uint64_t));
    static_assert(std::atomic<uint64_t>::is_always_lock_free);
    static constexpr uint64_t dirty_bit = uint64_t{1} << 63;
    std::atomic<uint64_t> latest_{0};
};

} // namespace rack_realtime
