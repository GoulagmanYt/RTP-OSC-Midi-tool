#pragma once
#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <mutex>

namespace rack_realtime {
template<class Change, size_t Capacity>
struct ParameterTransferQueue {
    std::array<Change, Capacity> entries{};
    std::atomic<uint64_t> read_index{0};
    std::atomic<uint64_t> write_index{0};
    std::atomic<uint64_t> dropped{0};
    std::mutex producer_mutex;

    bool push(const Change& change) noexcept {
        // Multiple non-realtime producers (plugin UI and Tauri commands) are
        // serialized here. The audio consumer never takes this mutex.
        std::lock_guard<std::mutex> producer_lock(producer_mutex);
        const uint64_t write = write_index.load(std::memory_order_relaxed);
        const uint64_t read = read_index.load(std::memory_order_acquire);
        if (write - read >= Capacity) {
            dropped.fetch_add(1, std::memory_order_relaxed);
            return false;
        }
        entries[write % Capacity] = change;
        write_index.store(write + 1, std::memory_order_release);
        return true;
    }

    bool pop(Change& change) noexcept {
        const uint64_t read = read_index.load(std::memory_order_relaxed);
        const uint64_t write = write_index.load(std::memory_order_acquire);
        if (read == write) {
            return false;
        }
        change = entries[read % Capacity];
        read_index.store(read + 1, std::memory_order_release);
        return true;
    }
    // Lifecycle thread only, with the audio consumer stopped. Preserve FIFO
    // for every entry that is not superseded by the restored live snapshot.
    template<class Predicate>
    void discard_if(Predicate superseded) {
        std::lock_guard<std::mutex> producer_lock(producer_mutex);
        const uint64_t read = read_index.load(std::memory_order_relaxed);
        const uint64_t write = write_index.load(std::memory_order_acquire);
        uint64_t kept = read;
        for (uint64_t cursor = read; cursor < write; ++cursor) {
            const Change change = entries[cursor % Capacity];
            if (!superseded(change)) entries[kept++ % Capacity] = change;
        }
        write_index.store(kept, std::memory_order_release);
    }
};


} // namespace rack_realtime
