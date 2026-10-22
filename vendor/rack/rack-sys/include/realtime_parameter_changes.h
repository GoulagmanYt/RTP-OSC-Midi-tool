#pragma once

#include "public.sdk/source/vst/hosting/parameterchanges.h"
#include <array>
#include <algorithm>
#include <memory>
#include <vector>

namespace rack_realtime {

// Fixed point arena shared by all parameters in one block. Queue objects are
// created only at initialization; SDK interfaces never grow storage in process().
class ParameterChanges final : public Steinberg::Vst::ParameterChanges {
    using int32 = Steinberg::int32;
    using ParamID = Steinberg::Vst::ParamID;
    using ParamValue = Steinberg::Vst::ParamValue;
    using tresult = Steinberg::tresult;
    struct Point {
        ParamValue value = 0;
        int32 offset = 0;
        int32 next = -1;
    };
    static constexpr size_t point_capacity = 4096;
    std::array<Point, point_capacity> points{};
    size_t point_count = 0;

    class ValueQueue final : public Steinberg::Vst::ParameterValueQueue {
        ParameterChanges& owner;
        int32 first = -1;
        int32 count = 0;
    public:
        explicit ValueQueue(ParameterChanges& owner)
            : ParameterValueQueue(Steinberg::Vst::kNoParamId), owner(owner) {}
        void reset(ParamID id) { setParamID(id); first = -1; count = 0; }
        int32 PLUGIN_API getPointCount() override { return count; }
        tresult PLUGIN_API getPoint(int32 index, int32& offset, ParamValue& value) override {
            if (index < 0 || index >= count) return Steinberg::kResultFalse;
            int32 cursor = first;
            while (index-- > 0) cursor = owner.points[cursor].next;
            const auto& point = owner.points[cursor];
            offset = point.offset;
            value = point.value;
            return Steinberg::kResultTrue;
        }
        tresult PLUGIN_API addPoint(int32 offset, ParamValue value, int32& index) override {
            int32 previous = -1;
            int32 cursor = first;
            index = 0;
            while (cursor >= 0 && owner.points[cursor].offset < offset) {
                previous = cursor;
                cursor = owner.points[cursor].next;
                ++index;
            }
            if (cursor >= 0 && owner.points[cursor].offset == offset) {
                owner.points[cursor].value = value;
                return Steinberg::kResultTrue;
            }
            if (owner.point_count == point_capacity) return Steinberg::kResultFalse;
            const auto added = static_cast<int32>(owner.point_count++);
            owner.points[added] = {value, offset, cursor};
            if (previous < 0) first = added;
            else owner.points[previous].next = added;
            ++count;
            return Steinberg::kResultTrue;
        }
    };
    std::vector<std::unique_ptr<ValueQueue>> queues;
    int32 used = 0;
public:
    void setMaxParameters(int32 maximum) {
        queues.clear();
        queues.reserve(static_cast<size_t>(std::max(0, maximum)));
        for (int32 i = 0; i < maximum; ++i) queues.emplace_back(std::make_unique<ValueQueue>(*this));
        clearQueue();
    }
    void clearQueue() { used = 0; point_count = 0; }
    int32 PLUGIN_API getParameterCount() override { return used; }
    Steinberg::Vst::IParamValueQueue* PLUGIN_API getParameterData(int32 index) override {
        return index >= 0 && index < used ? queues[index].get() : nullptr;
    }
    Steinberg::Vst::IParamValueQueue* PLUGIN_API addParameterData(const ParamID& id, int32& index) override {
        for (index = 0; index < used; ++index) {
            if (queues[index]->getParameterId() == id) return queues[index].get();
        }
        if (static_cast<size_t>(used) == queues.size()) return nullptr;
        auto* queue = queues[used++].get();
        queue->reset(id);
        return queue;
    }
};

} // namespace rack_realtime
