#include "realtime_parameter_mailbox.h"
#include "parameter_transfer_queue.h"

#include <array>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <thread>

static void require(bool condition, const char* message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(EXIT_FAILURE);
    }
}

int main() {
    std::array<rack_realtime::ParameterMailbox, 3> parameters;
    double value = -1;
    require(!parameters[0].take_latest(value), "new mailbox must be empty");
    require(value == -1, "empty read must preserve the output");
    for (unsigned update = 0; update < 25000; ++update) {
        const double normalized = double(update) / 24999.0;
        require(parameters[0].publish(normalized), "valid value must publish");
        require(parameters[1].publish(1.0 - normalized), "second parameter must publish");
        require(parameters[2].publish(double(update % 2)), "third parameter must publish");
    }
    for (unsigned index = 0; index < parameters.size(); ++index) {
        require(parameters[index].take_latest(value), "latest update must remain available");
        require(value == (index == 1 ? 0.0 : 1.0), "latest values must survive 25000 updates per parameter");
        require(!parameters[index].take_latest(value), "read must acknowledge the update");
    }
    require(parameters[0].publish(-0.0), "negative zero must publish");
    require(parameters[0].take_latest(value) && value == 0.0 && !std::signbit(value), "zero must decode with no dirty/sign bit");
    require(!parameters[0].publish(std::numeric_limits<double>::quiet_NaN()), "NaN must be rejected");
    require(!parameters[0].publish(-1) && !parameters[0].publish(2), "out of range values must be rejected");
    require(!parameters[0].take_latest(value), "invalid values must not dirty the mailbox");

    std::atomic<bool> done{false};
    std::thread producer([&] {
        for (unsigned update = 0; update <= 50000; ++update) parameters[0].publish(double(update) / 50000.0);
        done.store(true, std::memory_order_release);
    });
    double observed = -1;
    while (!done.load(std::memory_order_acquire)) {
        if (parameters[0].take_latest(value)) {
            require(value >= observed, "concurrent consumption must not move backwards");
            observed = value;
        }
    }
    producer.join();
    if (parameters[0].take_latest(value)) observed = value;
    require(observed == 1.0, "concurrent handoff must retain the final publication");
    struct Change { unsigned id; double value; };
    rack_realtime::ParameterTransferQueue<Change, 8> queue;
    for (unsigned index = 0; index < 8; ++index) require(queue.push({index, double(index)}), "queue must fill");
    Change change{};
    for (unsigned index = 0; index < 4; ++index) require(queue.pop(change) && change.id == index, "initial FIFO must hold");
    for (unsigned index = 8; index < 12; ++index) require(queue.push({index, double(index)}), "ring must wrap");
    queue.discard_if([](const Change& entry) { return entry.id % 2 == 0; });
    for (unsigned index : {5u, 7u, 9u, 11u}) require(queue.pop(change) && change.id == index, "selective discard must preserve FIFO across wrap");
    require(!queue.pop(change), "discarded entries must be absent");
    require(queue.push({1, 0.1}) && queue.push({2, 0.2}) && queue.push({1, 0.3}), "native restore entries must queue");
    queue.discard_if([](const Change& entry) { return entry.id == 1; });
    require(queue.pop(change) && change.id == 2 && change.value == 0.2, "unrelated native restore values must remain");
    require(!queue.pop(change), "older native values for live-restored parameter must be removed");
    std::puts("PASS: 25000 updates across three parameters, zero/one, invalid values, concurrent latest publication");
}
