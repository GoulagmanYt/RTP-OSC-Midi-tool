#include "rack_vst3.h"
#include "realtime_parameter_mailbox.h"
#include "parameter_transfer_queue.h"
#include "realtime_parameter_changes.h"
#include "public.sdk/source/vst/hosting/module.h"
#include "public.sdk/source/vst/hosting/plugprovider.h"
#include "public.sdk/source/vst/hosting/hostclasses.h"
#include "public.sdk/source/vst/hosting/pluginterfacesupport.h"
#include "public.sdk/source/vst/hosting/processdata.h"
#include "public.sdk/source/vst/hosting/parameterchanges.h"
#include "public.sdk/source/vst/hosting/eventlist.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivstcomponent.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/ivstmidicontrollers.h"
#include "pluginterfaces/vst/ivstprocesscontext.h"
#include "pluginterfaces/vst/ivstplugview.h"
#include "pluginterfaces/vst/ivstunits.h"
#include "pluginterfaces/vst/vstspeaker.h"
#include "pluginterfaces/gui/iplugview.h"
#include "pluginterfaces/base/ibstream.h"
#include "pluginterfaces/vst/ivsthostapplication.h"
#include "pluginterfaces/gui/iplugviewcontentscalesupport.h"
#include "public.sdk/source/vst/utility/stringconvert.h"

#if SMTG_OS_WINDOWS
#include <windows.h>
#endif

#include <vector>
#include <string>
#include <cstring>
#include <cstdio>
#include <cstdlib>
#include <mutex>
#include <memory>
#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>

using namespace VST3;
using namespace Steinberg;
using namespace Steinberg::Vst;

// Global mutex for VST3 lifecycle operations
// VST3 module loading/unloading is not guaranteed to be thread-safe
static std::mutex g_vst3_lifecycle_mutex;

static constexpr double kHostTempo = 120.0;

static double project_quarter_notes(TSamples block_start, uint32_t sample_offset,
                                    double sample_rate) noexcept {
    return sample_rate > 0.0
        ? (static_cast<double>(block_start) + static_cast<double>(sample_offset))
            / sample_rate * (kHostTempo / 60.0)
        : 0.0;
}

static constexpr size_t kRealtimeEventCapacity = 512;
static constexpr size_t kRealtimeInputEventCapacity = 2 * kRealtimeEventCapacity;
static constexpr size_t kParameterTransferCapacity = 4096;

struct PendingParameterChange {
    ParamID id = kNoParamId;
    ParamValue value = 0.0;
    int32 sample_offset = 0;
};

using ParameterTransferQueue = rack_realtime::ParameterTransferQueue<PendingParameterChange, kParameterTransferCapacity>;

// Single producer (audio thread), single consumer (UI/control thread).  Unlike
// ParameterTransferQueue this queue deliberately contains no producer mutex.
struct RealtimeParameterQueue {
    std::array<PendingParameterChange, kParameterTransferCapacity> entries{};
    std::atomic<uint64_t> read_index{0};
    std::atomic<uint64_t> write_index{0};
    std::atomic<uint64_t> dropped{0};

    bool push(const PendingParameterChange& change) noexcept {
        const uint64_t write = write_index.load(std::memory_order_relaxed);
        const uint64_t read = read_index.load(std::memory_order_acquire);
        if (write - read >= kParameterTransferCapacity) {
            dropped.fetch_add(1, std::memory_order_relaxed);
            return false;
        }
        entries[write % kParameterTransferCapacity] = change;
        write_index.store(write + 1, std::memory_order_release);
        return true;
    }

    bool pop(PendingParameterChange& change) noexcept {
        const uint64_t read = read_index.load(std::memory_order_relaxed);
        const uint64_t write = write_index.load(std::memory_order_acquire);
        if (read == write) {
            return false;
        }
        change = entries[read % kParameterTransferCapacity];
        read_index.store(read + 1, std::memory_order_release);
        return true;
    }
};

class OscMidiHostApplication final : public HostApplication {
public:
    OscMidiHostApplication() {
        if (auto* support = getPlugInterfaceSupport()) {
            support->removePlugInterfaceSupported(IEditController2::iid);
            support->removePlugInterfaceSupported(IParameterFinder::iid);
            support->removePlugInterfaceSupported(IAudioPresentationLatency::iid);
            support->removePlugInterfaceSupported(IEditControllerHostEditing::iid);
            support->removePlugInterfaceSupported(IUnitData::iid);
            support->removePlugInterfaceSupported(IProgramListData::iid);
        }
    }

    tresult PLUGIN_API getName(String128 name) override {
        return VST3::StringConvert::convert("OSCMidi", name) ? kResultTrue : kInternalError;
    }
};

static void trace_free_step(const char* step) {
    if (std::getenv("RACK_VST3_TRACE_FREE")) {
        std::fprintf(stderr, "rack_vst3_plugin_free: %s\n", step);
        std::fflush(stderr);
    }
}

// Helper: Convert UTF-16 to UTF-8
// VST3 uses char16 (UTF-16) for strings
// Handles surrogate pairs and malformed input safely
static std::string utf16_to_utf8(const char16* utf16_str) {
    if (!utf16_str) {
        return "";
    }

    std::string result;
    result.reserve(128);

    // Safety limit to prevent infinite loops on corrupted data
    // VST3 spec limits plugin names to 256 chars
    const size_t MAX_STRING_LENGTH = 4096;
    size_t processed = 0;

    while (*utf16_str && processed < MAX_STRING_LENGTH) {
        char16 c = *utf16_str++;
        processed++;

        // UTF-16 to UTF-8 conversion with complete error handling
        if (c < 0x80) {
            // U+0000 to U+007F: ASCII range (1 byte in UTF-8)
            result.push_back(static_cast<char>(c));
        }
        else if (c < 0x800) {
            // U+0080 to U+07FF: 2 bytes in UTF-8
            result.push_back(static_cast<char>(0xC0 | (c >> 6)));
            result.push_back(static_cast<char>(0x80 | (c & 0x3F)));
        }
        else if (c >= 0xD800 && c <= 0xDBFF) {
            // High surrogate (U+D800 to U+DBFF): Start of surrogate pair
            // Must be followed by low surrogate to form valid codepoint

            // Safety check: Ensure we haven't reached end of string
            if (*utf16_str == 0) {
                // Malformed: High surrogate at end of string
                result.append("\xEF\xBF\xBD"); // UTF-8 replacement character U+FFFD
                break;
            }

            char16 low = *utf16_str;

            // Validate low surrogate range (U+DC00 to U+DFFF)
            if (low >= 0xDC00 && low <= 0xDFFF) {
                // Valid surrogate pair - decode to codepoint U+10000 to U+10FFFF
                utf16_str++;
                processed++;

                uint32_t high_bits = (c & 0x3FF);      // 10 bits from high surrogate
                uint32_t low_bits = (low & 0x3FF);     // 10 bits from low surrogate
                uint32_t codepoint = 0x10000 + (high_bits << 10) + low_bits;

                // Validate codepoint is within valid Unicode range
                // Max valid codepoint is 0x10FFFF (which is exactly 0x10000 + (0x3FF << 10) + 0x3FF)
                if (codepoint > 0x10FFFF) {
                    // Overflow - invalid codepoint (should never happen with valid surrogates)
                    result.append("\xEF\xBF\xBD"); // UTF-8 replacement character U+FFFD
                } else {
                    // Encode as 4-byte UTF-8 sequence
                    result.push_back(static_cast<char>(0xF0 | (codepoint >> 18)));
                    result.push_back(static_cast<char>(0x80 | ((codepoint >> 12) & 0x3F)));
                    result.push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3F)));
                    result.push_back(static_cast<char>(0x80 | (codepoint & 0x3F)));
                }
            } else {
                // Malformed: High surrogate not followed by low surrogate
                result.append("\xEF\xBF\xBD"); // UTF-8 replacement character U+FFFD
            }
        }
        else if (c >= 0xDC00 && c <= 0xDFFF) {
            // Malformed: Low surrogate (U+DC00 to U+DFFF) without preceding high surrogate
            result.append("\xEF\xBF\xBD"); // UTF-8 replacement character U+FFFD
        }
        else {
            // U+0800 to U+FFFF (excluding surrogates): 3 bytes in UTF-8
            result.push_back(static_cast<char>(0xE0 | (c >> 12)));
            result.push_back(static_cast<char>(0x80 | ((c >> 6) & 0x3F)));
            result.push_back(static_cast<char>(0x80 | (c & 0x3F)));
        }
    }

    return result;
}

// Helper: Convert hex string to UID
static bool string_to_uid(const char* str, VST3::UID& uid) {
    if (!str) {
        return false;
    }
    auto uid_opt = VST3::UID::fromString(std::string(str));
    if (!uid_opt) {
        return false;
    }
    uid = *uid_opt;
    return true;
}

// Memory stream implementation for state serialization
class MemoryStream : public IBStream {
public:
    MemoryStream() : position_(0) { FUNKNOWN_CTOR }
    MemoryStream(const uint8_t* data, size_t size) : position_(0) {
        FUNKNOWN_CTOR
        buffer_.assign(data, data + size);
    }

    virtual ~MemoryStream() = default;

    // IUnknown
    DECLARE_FUNKNOWN_METHODS

    // IBStream
    tresult PLUGIN_API read(void* buffer, int32 numBytes, int32* numBytesRead) override {
        if (!buffer || numBytes < 0) {
            return kInvalidArgument;
        }

        int32 available = static_cast<int32>(buffer_.size()) - position_;
        int32 to_read = std::min(numBytes, available);

        if (to_read > 0) {
            memcpy(buffer, buffer_.data() + position_, to_read);
            position_ += to_read;
        }

        if (numBytesRead) {
            *numBytesRead = to_read;
        }

        return to_read == numBytes ? kResultOk : kResultFalse;
    }

    tresult PLUGIN_API write(void* buffer, int32 numBytes, int32* numBytesWritten) override {
        if (!buffer || numBytes < 0) {
            return kInvalidArgument;
        }

        // Resize buffer if needed
        if (position_ + numBytes > static_cast<int32>(buffer_.size())) {
            buffer_.resize(position_ + numBytes);
        }

        memcpy(buffer_.data() + position_, buffer, numBytes);
        position_ += numBytes;

        if (numBytesWritten) {
            *numBytesWritten = numBytes;
        }

        return kResultOk;
    }

    tresult PLUGIN_API seek(int64 pos, int32 mode, int64* result) override {
        // Calculate new position as int64 to prevent overflow/truncation
        int64 new_position = position_;

        switch (mode) {
            case kIBSeekSet:
                new_position = pos;
                break;
            case kIBSeekCur:
                new_position = static_cast<int64>(position_) + pos;
                break;
            case kIBSeekEnd:
                new_position = static_cast<int64>(buffer_.size()) + pos;
                break;
            default:
                return kInvalidArgument;
        }

        // Clamp to valid range [0, buffer_size]
        if (new_position < 0) {
            new_position = 0;
        }
        int64 buffer_size = static_cast<int64>(buffer_.size());
        if (new_position > buffer_size) {
            new_position = buffer_size;
        }

        // Safe to cast after clamping
        position_ = static_cast<int32>(new_position);

        if (result) {
            *result = position_;
        }

        return kResultOk;
    }

    tresult PLUGIN_API tell(int64* pos) override {
        if (!pos) {
            return kInvalidArgument;
        }
        *pos = position_;
        return kResultOk;
    }

    // Accessors
    const std::vector<uint8_t>& getData() const { return buffer_; }
    size_t getSize() const { return buffer_.size(); }
    void clear() { buffer_.clear(); position_ = 0; }

private:
    std::vector<uint8_t> buffer_;
    int32 position_;
};

IMPLEMENT_REFCOUNT(MemoryStream)

tresult PLUGIN_API MemoryStream::queryInterface(const TUID _iid, void** obj) {
    QUERY_INTERFACE(_iid, obj, FUnknown::iid, IBStream)
    QUERY_INTERFACE(_iid, obj, IBStream::iid, IBStream)
    *obj = nullptr;
    return kNoInterface;
}

// Internal plugin state
struct RackVST3Plugin;

class RackVST3ComponentHandler : public IComponentHandler, public IComponentHandler2,
                                 public IComponentHandlerBusActivation, public IUnitHandler {
public:
    explicit RackVST3ComponentHandler(RackVST3Plugin* plugin) : plugin_(plugin) { FUNKNOWN_CTOR }

    DECLARE_FUNKNOWN_METHODS

    tresult PLUGIN_API beginEdit(ParamID /*id*/) override;
    tresult PLUGIN_API performEdit(ParamID id, ParamValue valueNormalized) override;
    tresult PLUGIN_API endEdit(ParamID /*id*/) override;
    tresult PLUGIN_API restartComponent(int32 flags) override;

    tresult PLUGIN_API setDirty(TBool state) override;
    tresult PLUGIN_API requestOpenEditor(FIDString name) override;
    tresult PLUGIN_API startGroupEdit() override { return kResultOk; }
    tresult PLUGIN_API finishGroupEdit() override { return kResultOk; }
    tresult PLUGIN_API requestBusActivation(
        MediaType type, BusDirection dir, int32 index, TBool state) override;
    tresult PLUGIN_API notifyUnitSelection(UnitID /*unitId*/) override { return kResultOk; }
    tresult PLUGIN_API notifyProgramListChange(
        ProgramListID /*listId*/, int32 /*programIndex*/) override;

private:
    RackVST3Plugin* plugin_;
};

IMPLEMENT_REFCOUNT(RackVST3ComponentHandler)

tresult PLUGIN_API RackVST3ComponentHandler::queryInterface(const TUID _iid, void** obj) {
    QUERY_INTERFACE(_iid, obj, FUnknown::iid, IComponentHandler)
    QUERY_INTERFACE(_iid, obj, IComponentHandler::iid, IComponentHandler)
    QUERY_INTERFACE(_iid, obj, IComponentHandler2::iid, IComponentHandler2)
    QUERY_INTERFACE(_iid, obj, IComponentHandlerBusActivation::iid, IComponentHandlerBusActivation)
    QUERY_INTERFACE(_iid, obj, IUnitHandler::iid, IUnitHandler)
    *obj = nullptr;
    return kNoInterface;
}

#if SMTG_OS_WINDOWS
class RackVST3PlugFrame : public IPlugFrame {
public:
    RackVST3PlugFrame(HWND host_window, HWND container_window)
    : host_window_(host_window), container_window_(container_window) { FUNKNOWN_CTOR }

    DECLARE_FUNKNOWN_METHODS

    tresult PLUGIN_API resizeView(IPlugView* view, ViewRect* newSize) override {
        if (!view || !newSize || !container_window_) {
            return kInvalidArgument;
        }

        ViewRect constrained = *newSize;
        const tresult constraint_result = view->checkSizeConstraint(&constrained);
        if (constraint_result != kResultOk && constraint_result != kResultTrue
            && constraint_result != kNotImplemented) {
            return constraint_result;
        }
        const int width = constrained.right - constrained.left;
        const int height = constrained.bottom - constrained.top;
        if (width <= 0 || height <= 0) {
            return kInvalidArgument;
        }
        SetWindowPos(
            container_window_,
            nullptr,
            0,
            0,
            width,
            height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
        );

        if (host_window_) {
            RECT rect{0, 0, width, height};
            const auto style = static_cast<DWORD>(GetWindowLongPtrW(host_window_, GWL_STYLE));
            const auto ex_style = static_cast<DWORD>(GetWindowLongPtrW(host_window_, GWL_EXSTYLE));
            const UINT dpi = std::max<UINT>(GetDpiForWindow(host_window_), 96);
            AdjustWindowRectExForDpi(&rect, style, FALSE, ex_style, dpi);
            SetWindowPos(
                host_window_,
                nullptr,
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
            );
        }

        return view->onSize(&constrained);
    }

private:
    HWND host_window_;
    HWND container_window_;
};

IMPLEMENT_REFCOUNT(RackVST3PlugFrame)

tresult PLUGIN_API RackVST3PlugFrame::queryInterface(const TUID _iid, void** obj) {
    QUERY_INTERFACE(_iid, obj, FUnknown::iid, IPlugFrame)
    QUERY_INTERFACE(_iid, obj, IPlugFrame::iid, IPlugFrame)
    *obj = nullptr;
    return kNoInterface;
}
#endif

struct RackVST3Plugin {
    // Module and factory
    Hosting::Module::Ptr module;
    IPtr<IHostApplication> host_app;

    // Component and controller
    IPtr<IComponent> component;
    IPtr<IAudioProcessor> processor;
    IPtr<IEditController> controller;
    IPtr<IComponentHandler> component_handler;
    IPtr<IMidiMapping> midi_mapping;
    bool separate_controller = false;

    // Connection proxy (if component != controller)
    IPtr<IConnectionPoint> component_cp;
    IPtr<IConnectionPoint> controller_cp;

    // Plugin info
    std::string path;
    VST3::UID uid;

    // Audio configuration
    double sample_rate = 0.0;
    uint32_t max_block_size = 0;
    bool initialized = false;
    bool processing = false;
    bool active = false;
    SymbolicSampleSizes symbolic_sample_size = kSample32;
    uint32 process_context_requirements = 0;
    TSamples sample_position = 0;
    ProcessContext process_context{};
    std::atomic<int32> pending_restart_flags{0};
    std::atomic<bool> dirty{false};
    std::atomic<bool> editor_open_requested{false};

    std::mutex state_mutex;

    // I/O configuration
    int32 num_input_channels = 0;
    int32 num_output_channels = 0;

    // Processing structures
    HostProcessData process_data;
    rack_realtime::ParameterChanges input_param_changes;
    rack_realtime::ParameterChanges output_param_changes;
    EventList input_events;
    EventList output_events;
    // Conservative voice counts survive rejected DSP blocks. Panic releases use
    // a separate fixed backlog so a full MIDI batch cannot discard them.
    std::array<uint32_t, 16 * 128> active_notes{};
    std::array<uint32_t, 16 * 128> notes_after_block{};
    std::array<uint32_t, 16 * 128> reset_notes{};
    std::array<uint32_t, 16 * 128> reset_releases_in_block{};
    uint16_t reset_channels = 0;
    uint16_t reset_pedal_channels = 0;
    size_t reset_events_in_block = 0;
    std::array<RackVST3MidiEvent, kRealtimeEventCapacity> deferred_midi{};
    size_t deferred_midi_count = 0;
    bool notes_staged = false;
    ParameterTransferQueue ui_to_audio_parameters;
    RealtimeParameterQueue audio_to_ui_parameters;
    // One atomic latest value per parameter, allocated before processing starts.
    // Persistence must retain the newest value for an arbitrarily long game
    // session without requiring control-thread calls into the processor.
    std::unique_ptr<rack_realtime::ParameterMailbox[]> audio_persistent_latest;
    std::array<PendingParameterChange, kRealtimeEventCapacity> audio_parameter_scratch{};
    std::array<PendingParameterChange, kRealtimeEventCapacity> direct_audio_parameters{};
    size_t direct_audio_parameter_count = 0;

    // Audio buffers (for pointer arrays)
    std::vector<float*> input_ptrs;
    std::vector<float*> output_ptrs;
    std::vector<std::vector<double>> input_f64;
    std::vector<std::vector<double>> output_f64;

    // Parameter cache
    struct ParameterInfo {
        ParamID id;
        int32 flags;
        UnitID unit_id;
        int32 step_count;
        std::string title;
        std::string units;
        ParamValue min_value;
        ParamValue max_value;
        ParamValue default_value;
    };
    std::vector<ParameterInfo> parameters;
    std::mutex persistent_parameter_mutex;
    std::vector<PendingParameterChange> persistent_parameters;
    std::vector<ParamID> midi_controller_assignments;
    std::array<ParamID, 16> midi_program_assignments{};
    std::array<int32, 16> midi_program_steps{};

    // Preset cache (factory presets from IUnitInfo)
    struct PresetInfo {
        int32 program_list_id;
        int32 program_index;
        ParamID parameter_id = kNoParamId;
        int32 parameter_steps = 0;
        std::string name;
    };
    std::vector<PresetInfo> presets;
};

struct RackVST3Gui {
    RackVST3Plugin* plugin = nullptr;
    IPtr<IPlugView> view;
#if SMTG_OS_WINDOWS
    IPtr<IPlugFrame> frame;
    HWND host_window = nullptr;
    HWND container_window = nullptr;
#endif
    bool attached = false;
};

static size_t rack_vst3_midi_mapping_index(int16 channel, CtrlNumber controller_number);
static bool rack_vst3_try_queue_midi_mapping(
    RackVST3Plugin* plugin,
    int16 channel,
    CtrlNumber controller_number,
    ParamValue value_normalized,
    int32 sample_offset);
static void rack_vst3_begin_midi_block(RackVST3Plugin* plugin);
static void rack_vst3_queue_note_resets(RackVST3Plugin* plugin, uint32_t sample_offset);
static void rack_vst3_queue_emergency_controllers(RackVST3Plugin* plugin, uint32_t sample_offset);
static void rack_vst3_reset_channel(RackVST3Plugin* plugin, uint8_t channel);

static const RackVST3Plugin::ParameterInfo* rack_vst3_find_parameter(
    const RackVST3Plugin* plugin, ParamID id) {
    if (!plugin) return nullptr;
    auto found = std::find_if(
        plugin->parameters.begin(), plugin->parameters.end(),
        [id](const RackVST3Plugin::ParameterInfo& parameter) { return parameter.id == id; });
    return found == plugin->parameters.end() ? nullptr : &*found;
}

static bool rack_vst3_is_midi_proxy(const RackVST3Plugin* plugin, ParamID id) {
    return plugin && std::find(
        plugin->midi_controller_assignments.begin(),
        plugin->midi_controller_assignments.end(), id)
        != plugin->midi_controller_assignments.end();
}

static bool rack_vst3_parameter_is_persistent(
    const RackVST3Plugin* plugin, ParamID id, bool explicit_edit) {
    const auto* parameter = rack_vst3_find_parameter(plugin, id);
    if (!parameter || (parameter->flags & Vst::ParameterInfo::kIsReadOnly) != 0) {
        return false;
    }
    return explicit_edit || !rack_vst3_is_midi_proxy(plugin, id);
}

static void rack_vst3_track_parameter_control(
    RackVST3Plugin* plugin, ParamID id, ParamValue value, bool explicit_edit) {
    if (!rack_vst3_parameter_is_persistent(plugin, id, explicit_edit)
        || !std::isfinite(value)) {
        return;
    }
    const PendingParameterChange change{id, std::clamp<ParamValue>(value, 0.0, 1.0), 0};
    std::lock_guard<std::mutex> lock(plugin->persistent_parameter_mutex);
    auto found = std::find_if(
        plugin->persistent_parameters.begin(), plugin->persistent_parameters.end(),
        [id](const PendingParameterChange& item) { return item.id == id; });
    if (found != plugin->persistent_parameters.end()) {
        *found = change;
    } else if (plugin->persistent_parameters.size() < plugin->parameters.size()) {
        plugin->persistent_parameters.push_back(change);
    }
}

static void rack_vst3_publish_audio_parameter(RackVST3Plugin* plugin, uint32_t index, ParamValue value) {
    plugin->audio_persistent_latest[index].publish(value);
}

static void rack_vst3_drain_persistent_audio_parameters(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->audio_persistent_latest) return;
    for (size_t index = 0; index < plugin->parameters.size(); ++index) {
        ParamValue value = 0;
        if (!plugin->audio_persistent_latest[index].take_latest(value)) continue;
        const ParamID id = plugin->parameters[index].id;
        plugin->controller->setParamNormalized(id, value);
        rack_vst3_track_parameter_control(plugin, id, value, true);
    }
}

static void rack_vst3_drain_audio_parameters(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->controller) {
        return;
    }
    PendingParameterChange change;
    while (plugin->audio_to_ui_parameters.pop(change)) {
        plugin->controller->setParamNormalized(change.id, change.value);
        rack_vst3_track_parameter_control(plugin, change.id, change.value, false);
        plugin->dirty.store(true, std::memory_order_release);
    }
    rack_vst3_drain_persistent_audio_parameters(plugin);
}

tresult PLUGIN_API RackVST3ComponentHandler::beginEdit(ParamID /*id*/) {
    return plugin_ ? kResultOk : kInvalidArgument;
}

tresult PLUGIN_API RackVST3ComponentHandler::endEdit(ParamID /*id*/) {
    if (!plugin_) return kInvalidArgument;
    plugin_->dirty.store(true, std::memory_order_release);
    return kResultOk;
}

tresult PLUGIN_API RackVST3ComponentHandler::restartComponent(int32 flags) {
    if (!plugin_) return kInvalidArgument;
    plugin_->pending_restart_flags.fetch_or(flags, std::memory_order_release);
    if (flags & (kParamValuesChanged | kParamTitlesChanged | kMidiCCAssignmentChanged
                 | kReloadComponent | kIoChanged)) {
        plugin_->dirty.store(true, std::memory_order_release);
    }
    return kResultOk;
}

tresult PLUGIN_API RackVST3ComponentHandler::setDirty(TBool state) {
    if (!plugin_) return kInvalidArgument;
    plugin_->dirty.store(state != 0, std::memory_order_release);
    return kResultOk;
}

tresult PLUGIN_API RackVST3ComponentHandler::requestOpenEditor(FIDString name) {
    if (!plugin_ || (name && std::strcmp(name, ViewType::kEditor) != 0)) return kInvalidArgument;
    plugin_->editor_open_requested.store(true, std::memory_order_release);
    return kResultOk;
}

tresult PLUGIN_API RackVST3ComponentHandler::requestBusActivation(
    MediaType type, BusDirection /*dir*/, int32 index, TBool /*state*/) {
    if (!plugin_ || type != kAudio || index != 0) return kResultFalse;
    // The graph can only route the main bus.  Queue a controlled reconfiguration
    // instead of changing buses from inside a plug-in callback.
    plugin_->pending_restart_flags.fetch_or(kIoChanged, std::memory_order_release);
    return kResultOk;
}

tresult PLUGIN_API RackVST3ComponentHandler::notifyProgramListChange(
    ProgramListID /*listId*/, int32 /*programIndex*/) {
    if (!plugin_) return kInvalidArgument;
    plugin_->pending_restart_flags.fetch_or(
        kParamValuesChanged | kMidiCCAssignmentChanged, std::memory_order_release);
    plugin_->dirty.store(true, std::memory_order_release);
    return kResultOk;
}

tresult PLUGIN_API RackVST3ComponentHandler::performEdit(ParamID id, ParamValue valueNormalized) {
    if (!plugin_) {
        return kInvalidArgument;
    }

    if (!plugin_->initialized || !plugin_->controller) {
        return kNotInitialized;
    }
    rack_vst3_track_parameter_control(plugin_, id, valueNormalized, true);
    plugin_->dirty.store(true, std::memory_order_release);
    return plugin_->ui_to_audio_parameters.push({id, valueNormalized, 0})
        ? kResultOk
        : kResultFalse;
}

// ============================================================================
// Plugin Instance Implementation
// ============================================================================

RackVST3Plugin* rack_vst3_plugin_new(const char* path, const char* uid) {
    if (!path || !uid) {
        return nullptr;
    }

    std::lock_guard<std::mutex> lock(g_vst3_lifecycle_mutex);

    auto plugin = new(std::nothrow) RackVST3Plugin();
    if (!plugin) {
        return nullptr;
    }

    plugin->path = path;

    // Parse UID
    if (!string_to_uid(uid, plugin->uid)) {
        delete plugin;
        return nullptr;
    }

    // Load module
    std::string error_description;
    plugin->module = Hosting::Module::create(path, error_description);
    if (!plugin->module) {
        delete plugin;
        return nullptr;
    }

    plugin->host_app = FUnknownPtr<IHostApplication>(new OscMidiHostApplication());
    if (!plugin->host_app) {
        delete plugin;
        return nullptr;
    }

    // Create component
    const auto& factory = plugin->module->getFactory();
    plugin->component = factory.createInstance<IComponent>(plugin->uid);
    if (!plugin->component) {
        delete plugin;
        return nullptr;
    }

    // Get processor interface
    plugin->processor = U::cast<IAudioProcessor>(plugin->component);
    if (!plugin->processor) {
        delete plugin;
        return nullptr;
    }

    // Initialize component
    if (plugin->component->initialize(plugin->host_app) != kResultOk) {
        // Component creation succeeded but initialization failed - no need to terminate
        // IPtr will automatically release when plugin is deleted
        plugin->component = nullptr;
        plugin->processor = nullptr;
        plugin->host_app = nullptr;
        plugin->module = nullptr;
        delete plugin;
        return nullptr;
    }

    // Try to get edit controller
    TUID controllerCID;
    if (plugin->component->getControllerClassId(controllerCID) == kResultTrue) {
        // Controller is separate from component
        VST3::UID controllerUID = VST3::UID::fromTUID(controllerCID);

        plugin->controller = factory.createInstance<IEditController>(controllerUID);
        if (plugin->controller) {
            plugin->separate_controller = true;
            // Initialize controller - if this fails, clean up properly
            if (plugin->controller->initialize(plugin->host_app) != kResultOk) {
                // Controller init failed - terminate component and clean up
                plugin->component->terminate();
                plugin->controller = nullptr;
                plugin->component = nullptr;
                plugin->processor = nullptr;
                plugin->host_app = nullptr;
                plugin->module = nullptr;
                delete plugin;
                return nullptr;
            }
        }
    } else {
        // Component is also the controller (single component architecture)
        plugin->controller = U::cast<IEditController>(plugin->component);
    }

    // Set up connection points if controller is separate
    if (plugin->controller && plugin->separate_controller) {
        plugin->component_cp = U::cast<IConnectionPoint>(plugin->component);
        plugin->controller_cp = U::cast<IConnectionPoint>(plugin->controller);

        if (plugin->component_cp && plugin->controller_cp) {
            plugin->component_cp->connect(plugin->controller_cp);
            plugin->controller_cp->connect(plugin->component_cp);
        }
    }

    if (plugin->controller) {
        plugin->component_handler = FUnknownPtr<IComponentHandler>(
            static_cast<IComponentHandler*>(new RackVST3ComponentHandler(plugin)));
        if (plugin->component_handler) {
            plugin->controller->setComponentHandler(plugin->component_handler);
        }

        plugin->midi_mapping = U::cast<IMidiMapping>(plugin->controller);

        // A separately-created controller must receive the processor's current
        // component state before it is exposed to the host/UI.
        if (plugin->separate_controller) {
            IPtr<MemoryStream> component_state(new MemoryStream(), false);
            if (plugin->component->getState(component_state) == kResultOk) {
                component_state->seek(0, IBStream::kIBSeekSet, nullptr);
                plugin->controller->setComponentState(component_state);
            }
        }
    }

    plugin->input_events.setMaxSize(static_cast<int32>(kRealtimeInputEventCapacity));
    plugin->output_events.setMaxSize(static_cast<int32>(kRealtimeEventCapacity));

    return plugin;
}

void rack_vst3_plugin_free(RackVST3Plugin* plugin) {
    if (!plugin) {
        return;
    }

    std::lock_guard<std::mutex> lock(g_vst3_lifecycle_mutex);
    // Audio processing is stopped by the host before teardown. This mutex
    // serializes the remaining state/controller/UI operations.
    std::unique_lock<std::mutex> state_lock(plugin->state_mutex);
    trace_free_step("begin");

    // Stop processing before deactivating/terminating the plugin.
    // Many VST3 plugins expect the host teardown sequence to mirror the startup sequence:
    // setProcessing(false) -> setActive(false) -> terminate().
    if (plugin->processing && plugin->processor) {
        trace_free_step("setProcessing(false)");
        plugin->processor->setProcessing(false);
        plugin->processing = false;
    }

    if (plugin->active && plugin->component) {
        trace_free_step("setActive(false)");
        plugin->component->setActive(false);
        plugin->active = false;
    }

    // Disconnect connection points
    if (plugin->component_cp && plugin->controller_cp) {
        trace_free_step("disconnect connection points");
        plugin->component_cp->disconnect(plugin->controller_cp);
        plugin->controller_cp->disconnect(plugin->component_cp);
    }

    trace_free_step("clear host-side buffers");
    plugin->input_events.clear();
    plugin->output_events.clear();
    plugin->input_param_changes.clearQueue();
    plugin->output_param_changes.clearQueue();
    plugin->input_ptrs.clear();
    plugin->output_ptrs.clear();
    plugin->initialized = false;

    if (plugin->controller && plugin->component_handler) {
        plugin->controller->setComponentHandler(nullptr);
    }

    // Terminate controller
    if (plugin->controller && plugin->separate_controller) {
        trace_free_step("controller terminate");
        plugin->controller->terminate();
        plugin->controller = nullptr;
    }

    // Terminate component
    if (plugin->component) {
        trace_free_step("component terminate");
        plugin->component->terminate();
        plugin->component = nullptr;
    }

    trace_free_step("release remaining refs");
    // All plug-in interfaces must be released while their module is loaded,
    // including connection points and a combined component/controller.
    plugin->component_cp = nullptr;
    plugin->controller_cp = nullptr;
    plugin->controller = nullptr;
    plugin->component_handler = nullptr;
    plugin->midi_mapping = nullptr;
    plugin->processor = nullptr;
    plugin->host_app = nullptr;
    plugin->module = nullptr;

    trace_free_step("delete plugin");
    // The mutex belongs to plugin: release it before destroying its storage.
    state_lock.unlock();
    delete plugin;
}

int rack_vst3_plugin_initialize(RackVST3Plugin* plugin, double sample_rate, uint32_t max_block_size) {
    if (!plugin || !plugin->component || !plugin->processor) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    std::lock_guard<std::mutex> lock(g_vst3_lifecycle_mutex);

    plugin->sample_rate = sample_rate;
    plugin->max_block_size = max_block_size;

    // Negotiate the symbolic sample size before setupProcessing. OSCMidi's
    // public audio boundary is float32, so float64-only processors use
    // preallocated conversion buffers below.
    if (plugin->processor->canProcessSampleSize(kSample32) == kResultTrue) {
        plugin->symbolic_sample_size = kSample32;
    } else if (plugin->processor->canProcessSampleSize(kSample64) == kResultTrue) {
        plugin->symbolic_sample_size = kSample64;
    } else {
        return RACK_VST3_ERROR_NOT_SUPPORTED;
    }

    // Read the current main-bus topology while inactive. Auxiliary buses are
    // kept inactive; the active main bus must be mono or stereo for OSCMidi's
    // stereo output graph.
    const int32 numInputBuses = plugin->component->getBusCount(kAudio, kInput);
    const int32 numOutputBuses = plugin->component->getBusCount(kAudio, kOutput);
    const int32 numEventInputBuses = plugin->component->getBusCount(kEvent, kInput);
    const int32 numEventOutputBuses = plugin->component->getBusCount(kEvent, kOutput);

    std::vector<SpeakerArrangement> input_arrangements(static_cast<size_t>(numInputBuses));
    std::vector<SpeakerArrangement> output_arrangements(static_cast<size_t>(numOutputBuses));
    for (int32 index = 0; index < numInputBuses; ++index) {
        plugin->processor->getBusArrangement(
            kInput, index, input_arrangements[static_cast<size_t>(index)]);
    }
    bool output_arrangement_changed = false;
    for (int32 index = 0; index < numOutputBuses; ++index) {
        plugin->processor->getBusArrangement(
            kOutput, index, output_arrangements[static_cast<size_t>(index)]);
    }
    if (!output_arrangements.empty()) {
        const int32 current_channels = SpeakerArr::getChannelCount(output_arrangements[0]);
        if (current_channels < 1 || current_channels > 2) {
            output_arrangements[0] = SpeakerArr::kStereo;
            output_arrangement_changed = true;
        }
    }
    if (output_arrangement_changed) {
        const tresult arrangement_result = plugin->processor->setBusArrangements(
            input_arrangements.empty() ? nullptr : input_arrangements.data(),
            numInputBuses,
            output_arrangements.data(),
            numOutputBuses);
        if (arrangement_result != kResultOk && arrangement_result != kResultTrue) {
            return RACK_VST3_ERROR_NOT_SUPPORTED;
        }
    }

    plugin->num_input_channels = 0;
    plugin->num_output_channels = 0;
    if (numInputBuses > 0) {
        BusInfo info{};
        if (plugin->component->getBusInfo(kAudio, kInput, 0, info) == kResultOk) {
            plugin->num_input_channels = info.channelCount;
        }
    }
    if (numOutputBuses > 0) {
        BusInfo info{};
        if (plugin->component->getBusInfo(kAudio, kOutput, 0, info) == kResultOk) {
            plugin->num_output_channels = info.channelCount;
        }
    }
    if (plugin->num_output_channels < 1 || plugin->num_output_channels > 2) {
        return RACK_VST3_ERROR_NOT_SUPPORTED;
    }

    // Setup processing in realtime mode after bus discovery/selection.
    ProcessSetup setup;
    setup.processMode = kRealtime;
    setup.symbolicSampleSize = plugin->symbolic_sample_size;
    setup.maxSamplesPerBlock = max_block_size;
    setup.sampleRate = sample_rate;

    if (plugin->processor->setupProcessing(setup) != kResultOk) {
        return RACK_VST3_ERROR_GENERIC;
    }

    IPtr<IProcessContextRequirements> context_requirements =
        U::cast<IProcessContextRequirements>(plugin->processor);
    plugin->process_context_requirements = context_requirements
        ? context_requirements->getProcessContextRequirements()
        : (IProcessContextRequirements::kNeedSystemTime
           | IProcessContextRequirements::kNeedContinousTimeSamples
           | IProcessContextRequirements::kNeedProjectTimeMusic
           | IProcessContextRequirements::kNeedBarPositionMusic
           | IProcessContextRequirements::kNeedTempo
           | IProcessContextRequirements::kNeedTimeSignature
           | IProcessContextRequirements::kNeedTransportState);
    if (std::getenv("RACK_VST3_TRACE_INIT")) {
        std::fprintf(
            stderr,
            "rack_vst3_plugin_initialize: audio=%d/%d event=%d/%d\n",
            numInputBuses,
            numOutputBuses,
            numEventInputBuses,
            numEventOutputBuses
        );
        std::fflush(stderr);
    }

    // OSCMidi routes only the main audio bus. Explicitly deactivate auxiliary
    // buses so multi-output instruments do not render into unconnected buses.
    for (int32 index = 0; index < numInputBuses; ++index) {
        plugin->component->activateBus(kAudio, kInput, index, index == 0);
        if (index == 0) {
            BusInfo busInfo{};
            if (plugin->component->getBusInfo(kAudio, kInput, index, busInfo) == kResultOk) {
                plugin->num_input_channels = busInfo.channelCount;
            }
        }
    }

    for (int32 index = 0; index < numOutputBuses; ++index) {
        plugin->component->activateBus(kAudio, kOutput, index, index == 0);
        if (index == 0) {
            BusInfo busInfo{};
            if (plugin->component->getBusInfo(kAudio, kOutput, index, busInfo) == kResultOk) {
                plugin->num_output_channels = busInfo.channelCount;
            }
        }
    }

    // VST3 event buses are independent from audio buses. A number of plugins
    // accept events without explicit activation, but conforming instruments
    // such as Upright Piano ignore NoteOn/NoteOff until their main event input
    // is active. Activate the main bus plus any bus advertised as active by
    // default before setActive(true).
    for (int32 index = 0; index < numEventInputBuses; ++index) {
        BusInfo busInfo{};
        const bool hasInfo =
            plugin->component->getBusInfo(kEvent, kInput, index, busInfo) == kResultOk;
        const bool activate = index == 0
            || (hasInfo && (busInfo.flags & BusInfo::kDefaultActive) != 0);
        plugin->component->activateBus(kEvent, kInput, index, activate);
    }
    for (int32 index = 0; index < numEventOutputBuses; ++index) {
        BusInfo busInfo{};
        const bool hasInfo =
            plugin->component->getBusInfo(kEvent, kOutput, index, busInfo) == kResultOk;
        const bool activate = index == 0
            || (hasInfo && (busInfo.flags & BusInfo::kDefaultActive) != 0);
        plugin->component->activateBus(kEvent, kOutput, index, activate);
    }

    // Activate component
    if (plugin->component->setActive(true) != kResultOk) {
        return RACK_VST3_ERROR_GENERIC;
    }
    plugin->active = true;

    // Start processing
    if (plugin->processor->setProcessing(true) != kResultOk) {
        plugin->component->setActive(false);
        plugin->active = false;
        return RACK_VST3_ERROR_GENERIC;
    }
    plugin->processing = true;

    // Prepare process_data once during initialization, but only allocate the bus
    // pointer arrays. The actual sample buffers come from the Rust host on each
    // process() call and must never be owned/freed by HostProcessData.
    plugin->process_data.prepare(*plugin->component, 0, plugin->symbolic_sample_size);
    plugin->process_data.processContext = &plugin->process_context;
    if (plugin->symbolic_sample_size == kSample64) {
        plugin->input_f64.assign(
            static_cast<size_t>(plugin->num_input_channels),
            std::vector<double>(max_block_size, 0.0));
        plugin->output_f64.assign(
            static_cast<size_t>(plugin->num_output_channels),
            std::vector<double>(max_block_size, 0.0));
    }

    // Build parameter cache
    if (plugin->controller) {
        int32 param_count = plugin->controller->getParameterCount();
        plugin->parameters.clear();
        plugin->parameters.reserve(param_count);

        for (int32 i = 0; i < param_count; ++i) {
            ParameterInfo vst3_param_info;
            if (plugin->controller->getParameterInfo(i, vst3_param_info) == kResultOk) {
                RackVST3Plugin::ParameterInfo info;
                info.id = vst3_param_info.id;
                info.flags = vst3_param_info.flags;
                info.unit_id = vst3_param_info.unitId;
                info.step_count = vst3_param_info.stepCount;

                // Convert UTF-16 to UTF-8 (proper conversion for international characters)
                info.title = utf16_to_utf8(vst3_param_info.title);
                info.units = utf16_to_utf8(vst3_param_info.units);

                // VST3 parameters are already normalized 0.0-1.0
                info.min_value = 0.0;
                info.max_value = 1.0;
                info.default_value = vst3_param_info.defaultNormalizedValue;

                plugin->parameters.push_back(info);
            }
        }

        plugin->audio_persistent_latest = std::make_unique<rack_realtime::ParameterMailbox[]>(plugin->parameters.size());
        plugin->midi_controller_assignments.assign(16 * kCountCtrlNumber, kNoParamId);
        if (plugin->midi_mapping) {
            for (int16 channel = 0; channel < 16; ++channel) {
                for (int32 ctrl = 0; ctrl < kCountCtrlNumber; ++ctrl) {
                    ParamID param_id = kNoParamId;
                    if (plugin->midi_mapping->getMidiControllerAssignment(
                            0,
                            channel,
                            static_cast<CtrlNumber>(ctrl),
                            param_id)
                        == kResultTrue)
                    {
                        plugin->midi_controller_assignments[rack_vst3_midi_mapping_index(
                            channel,
                            static_cast<CtrlNumber>(ctrl))] = param_id;
                    }
                }
            }
        }

        const int32 cached_parameter_count = static_cast<int32>(plugin->parameters.size());
        plugin->input_param_changes.setMaxParameters(cached_parameter_count);
        plugin->output_param_changes.setMaxParameters(cached_parameter_count);
        // Both lists now own fixed point arenas; no first-edit warmup or
        // SDK vector growth is needed from process().
    }

    // Enumerate factory presets if available
    IPtr<IUnitInfo> unit_info = U::cast<IUnitInfo>(plugin->controller);
    if (unit_info) {
        plugin->midi_program_assignments.fill(kNoParamId);
        plugin->midi_program_steps.fill(0);
        for (int32 channel = 0; channel < 16; ++channel) {
            UnitID unit_id = kRootUnitId;
            unit_info->getUnitByBus(kEvent, kInput, 0, channel, unit_id);
            for (const auto& parameter : plugin->parameters) {
                if ((parameter.flags & Vst::ParameterInfo::kIsProgramChange) != 0
                    && parameter.unit_id == unit_id && parameter.step_count > 0) {
                    plugin->midi_program_assignments[static_cast<size_t>(channel)] = parameter.id;
                    plugin->midi_program_steps[static_cast<size_t>(channel)] = parameter.step_count;
                    break;
                }
            }
        }
        int32 program_list_count = unit_info->getProgramListCount();
        for (int32 i = 0; i < program_list_count; ++i) {
            ProgramListInfo list_info;
            if (unit_info->getProgramListInfo(i, list_info) == kResultOk) {
                UnitID program_unit_id = kRootUnitId;
                for (int32 unit_index = 0; unit_index < unit_info->getUnitCount(); ++unit_index) {
                    UnitInfo candidate{};
                    if (unit_info->getUnitInfo(unit_index, candidate) == kResultOk
                        && candidate.programListId == list_info.id) {
                        program_unit_id = candidate.id;
                        break;
                    }
                }
                ParamID program_parameter = kNoParamId;
                int32 program_steps = 0;
                for (int32 parameter_index = 0;
                     parameter_index < plugin->controller->getParameterCount();
                     ++parameter_index) {
                    Vst::ParameterInfo candidate{};
                    if (plugin->controller->getParameterInfo(parameter_index, candidate) == kResultOk
                        && (candidate.flags & Vst::ParameterInfo::kIsProgramChange) != 0
                        && candidate.unitId == program_unit_id) {
                        program_parameter = candidate.id;
                        program_steps = candidate.stepCount;
                        break;
                    }
                }
                // Enumerate programs in this list
                for (int32 j = 0; j < list_info.programCount; ++j) {
                    String128 program_name;
                    if (unit_info->getProgramName(list_info.id, j, program_name) == kResultOk) {
                        RackVST3Plugin::PresetInfo preset;
                        preset.program_list_id = list_info.id;
                        preset.program_index = j;
                        preset.parameter_id = program_parameter;
                        preset.parameter_steps = program_steps;
                        preset.name = utf16_to_utf8(program_name);
                        plugin->presets.push_back(preset);
                    }
                }
            }
        }
    }

    plugin->initialized = true;
    return RACK_VST3_OK;
}

int rack_vst3_plugin_is_initialized(RackVST3Plugin* plugin) {
    return (plugin && plugin->initialized) ? 1 : 0;
}

static void rack_vst3_service_restart_notifications(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->initialized) return;
    const int32 flags = plugin->pending_restart_flags.exchange(0, std::memory_order_acq_rel);
    if (flags == 0) return;

    if ((flags & kParamTitlesChanged) && plugin->controller) {
        const int32 count = std::min<int32>(
            plugin->controller->getParameterCount(),
            static_cast<int32>(plugin->parameters.size()));
        for (int32 index = 0; index < count; ++index) {
            Vst::ParameterInfo source{};
            if (plugin->controller->getParameterInfo(index, source) != kResultOk) continue;
            auto& target = plugin->parameters[static_cast<size_t>(index)];
            target.id = source.id;
            target.flags = source.flags;
            target.unit_id = source.unitId;
            target.step_count = source.stepCount;
            target.title = utf16_to_utf8(source.title);
            target.units = utf16_to_utf8(source.units);
            target.default_value = source.defaultNormalizedValue;
        }
    }
    if ((flags & kMidiCCAssignmentChanged) && plugin->midi_mapping) {
        for (int16 channel = 0; channel < 16; ++channel) {
            for (int32 ctrl = 0; ctrl < kCountCtrlNumber; ++ctrl) {
                ParamID param_id = kNoParamId;
                if (plugin->midi_mapping->getMidiControllerAssignment(
                        0, channel, static_cast<CtrlNumber>(ctrl), param_id) != kResultTrue) {
                    param_id = kNoParamId;
                }
                plugin->midi_controller_assignments[rack_vst3_midi_mapping_index(
                    channel, static_cast<CtrlNumber>(ctrl))] = param_id;
            }
        }
    }
    if ((flags & kLatencyChanged) && plugin->processor && plugin->component) {
        if (plugin->processing) {
            plugin->processor->setProcessing(false);
            plugin->processing = false;
        }
        if (plugin->active) {
            plugin->component->setActive(false);
            plugin->active = false;
        }
        if (plugin->component->setActive(true) == kResultOk) {
            plugin->active = true;
            if (plugin->processor->setProcessing(true) == kResultOk) {
                plugin->processing = true;
            }
        }
    }
    // I/O graph and full component reload requests cannot be applied inside a
    // plug-in callback.  They are deliberately consumed here at a host safe
    // point; the outer worker supervisor will perform a state-preserving reload
    // if subsequent processing reports an incompatible layout.
}

int rack_vst3_plugin_reset(RackVST3Plugin* plugin) {
    // Check null pointer BEFORE acquiring mutex (no lock needed if null)
    if (!plugin) {
        return RACK_VST3_ERROR_NOT_INITIALIZED;
    }

    // Acquire mutex BEFORE checking state to prevent TOCTOU race condition
    // Without this, another thread could change initialized between check and lock
    std::lock_guard<std::mutex> lock(g_vst3_lifecycle_mutex);
    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);

    if (!plugin->initialized || !plugin->component) {
        return RACK_VST3_ERROR_NOT_INITIALIZED;
    }

    if (plugin->processing && plugin->processor->setProcessing(false) != kResultOk) {
        return RACK_VST3_ERROR_GENERIC;
    }
    plugin->processing = false;
    if (plugin->active && plugin->component->setActive(false) != kResultOk) {
        return RACK_VST3_ERROR_GENERIC;
    }
    plugin->active = false;
    if (plugin->component->setActive(true) != kResultOk) {
        return RACK_VST3_ERROR_GENERIC;
    }
    plugin->active = true;
    if (plugin->processor->setProcessing(true) != kResultOk) {
        plugin->component->setActive(false);
        plugin->active = false;
        return RACK_VST3_ERROR_GENERIC;
    }
    plugin->processing = true;
    plugin->sample_position = 0;

    return RACK_VST3_OK;
}

int rack_vst3_plugin_get_input_channels(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->initialized) {
        return 0;
    }
    return plugin->num_input_channels;
}

int rack_vst3_plugin_get_output_channels(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->initialized) {
        return 0;
    }
    return plugin->num_output_channels;
}

uint32_t rack_vst3_plugin_get_latency_samples(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->initialized || !plugin->processor) {
        return 0;
    }
    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    rack_vst3_service_restart_notifications(plugin);
    return plugin->processor->getLatencySamples();
}

int rack_vst3_plugin_process(
    RackVST3Plugin* plugin,
    const float* const* inputs,
    uint32_t num_input_channels,
    float* const* outputs,
    uint32_t num_output_channels,
    uint32_t frames)
{
    if (!plugin || !plugin->initialized || !plugin->processor) {
        return RACK_VST3_ERROR_NOT_INITIALIZED;
    }

    // Validate input parameters to prevent buffer overruns
    // Channel counts must match what was configured during initialization
    if (num_input_channels != static_cast<uint32_t>(plugin->num_input_channels)) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    if (num_output_channels != static_cast<uint32_t>(plugin->num_output_channels)) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    // Frame count must not exceed the max block size configured during initialization
    if (frames > plugin->max_block_size) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    // Validate buffer pointers when channel counts > 0
    if (num_input_channels > 0 && !inputs) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    if (num_output_channels > 0 && !outputs) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    if (!plugin->notes_staged) {
        rack_vst3_begin_midi_block(plugin);
        rack_vst3_queue_note_resets(plugin, 0);
    }

    // Update dynamic fields only (prepare() was called during initialization)
    plugin->process_data.numSamples = frames;

    auto& context = plugin->process_context;
    context = {};
    context.sampleRate = plugin->sample_rate;
    context.projectTimeSamples = plugin->sample_position;
    context.state = 0;
    const uint32 requirements = plugin->process_context_requirements;
    if (requirements & IProcessContextRequirements::kNeedTransportState) {
        context.state |= ProcessContext::kPlaying;
    }
    if (requirements & IProcessContextRequirements::kNeedSystemTime) {
        context.systemTime = std::chrono::duration_cast<std::chrono::nanoseconds>(
            std::chrono::steady_clock::now().time_since_epoch()).count();
        context.state |= ProcessContext::kSystemTimeValid;
    }
    if (requirements & IProcessContextRequirements::kNeedContinousTimeSamples) {
        context.continousTimeSamples = plugin->sample_position;
        context.state |= ProcessContext::kContTimeValid;
    }
    const double quarter_notes =
        project_quarter_notes(plugin->sample_position, 0, plugin->sample_rate);
    if (requirements & IProcessContextRequirements::kNeedProjectTimeMusic) {
        context.projectTimeMusic = quarter_notes;
        context.state |= ProcessContext::kProjectTimeMusicValid;
    }
    if (requirements & IProcessContextRequirements::kNeedBarPositionMusic) {
        context.barPositionMusic = std::floor(quarter_notes / 4.0) * 4.0;
        context.state |= ProcessContext::kBarPositionValid;
    }
    if (requirements & IProcessContextRequirements::kNeedTempo) {
        context.tempo = kHostTempo;
        context.state |= ProcessContext::kTempoValid;
    }
    if (requirements & IProcessContextRequirements::kNeedTimeSignature) {
        context.timeSigNumerator = 4;
        context.timeSigDenominator = 4;
        context.state |= ProcessContext::kTimeSigValid;
    }

    // Set input buffers on the pre-allocated HostProcessData bus arrays.
    // Do not replace channelBuffers32 itself: HostProcessData owns those arrays
    // and will free them during teardown.
    if (num_input_channels > 0) {
        AudioBusBuffers& bus = plugin->process_data.inputs[0];
        bus.numChannels = num_input_channels;
        for (uint32_t ch = 0; ch < num_input_channels; ++ch) {
            if (plugin->symbolic_sample_size == kSample32) {
                bus.channelBuffers32[ch] = const_cast<float*>(inputs[ch]);
            } else {
                auto& converted = plugin->input_f64[ch];
                for (uint32_t frame = 0; frame < frames; ++frame) {
                    converted[frame] = static_cast<double>(inputs[ch][frame]);
                }
                bus.channelBuffers64[ch] = converted.data();
            }
        }
    }

    // Set output buffers on the pre-allocated HostProcessData bus arrays.
    if (num_output_channels > 0) {
        AudioBusBuffers& bus = plugin->process_data.outputs[0];
        bus.numChannels = num_output_channels;
        for (uint32_t ch = 0; ch < num_output_channels; ++ch) {
            if (plugin->symbolic_sample_size == kSample32) {
                bus.channelBuffers32[ch] = outputs[ch];
            } else {
                bus.channelBuffers64[ch] = plugin->output_f64[ch].data();
            }
        }
    }

    // Set parameter and event interfaces
    plugin->process_data.inputParameterChanges = &plugin->input_param_changes;
    plugin->process_data.outputParameterChanges = &plugin->output_param_changes;
    plugin->process_data.inputEvents = &plugin->input_events;
    plugin->process_data.outputEvents = &plugin->output_events;

    // UI/controller changes cross into the processor through a bounded SPSC
    // queue. Coalescing guarantees at most one point per parameter and therefore
    // reuses the preallocated ParameterValueQueue storage.
    size_t pending_count = plugin->direct_audio_parameter_count;
    for (size_t index = 0; index < pending_count; ++index) {
        plugin->audio_parameter_scratch[index] = plugin->direct_audio_parameters[index];
    }
    plugin->direct_audio_parameter_count = 0;
    PendingParameterChange pending;
    size_t parameter_reads = 0;
    while (parameter_reads++ < kRealtimeEventCapacity
           && pending_count < plugin->audio_parameter_scratch.size()
           && plugin->ui_to_audio_parameters.pop(pending)) {
        size_t existing = 0;
        for (; existing < pending_count; ++existing) {
            if (plugin->audio_parameter_scratch[existing].id == pending.id) {
                plugin->audio_parameter_scratch[existing] = pending;
                break;
            }
        }
        if (existing == pending_count) {
            plugin->audio_parameter_scratch[pending_count++] = pending;
        }
    }
    for (size_t index = 0; index < pending_count; ++index) {
        const auto& change = plugin->audio_parameter_scratch[index];
        int32 queue_index = 0;
        IParamValueQueue* queue =
            plugin->input_param_changes.addParameterData(change.id, queue_index);
        if (!queue) {
            continue;
        }
        int32 point_index = 0;
        queue->addPoint(change.sample_offset, change.value, point_index);
    }

    // Process
    // UI/direct edits are merged at offset zero. Emergency pedal release must
    // take precedence over an edit of the same MIDI proxy in this block.
    rack_vst3_queue_emergency_controllers(plugin, 0);
    tresult result = plugin->processor->process(plugin->process_data);

    if (result == kResultOk) {
        plugin->active_notes = plugin->notes_after_block;
        for (size_t channel = 0; channel < 16; ++channel) {
            bool remaining = false;
            for (size_t pitch = 0; pitch < 128; ++pitch) {
                const size_t index = channel * 128 + pitch;
                plugin->reset_notes[index] -= plugin->reset_releases_in_block[index];
                remaining |= plugin->reset_notes[index] != 0;
            }
            if (!remaining) {
                plugin->reset_channels &= static_cast<uint16_t>(~(1u << channel));
                plugin->reset_pedal_channels &= static_cast<uint16_t>(~(1u << channel));
            }
        }
    } else {
        // The processor may have consumed only part of a failed block. Retain
        // every possible Note-On and retry releases before accepting retriggers.
        for (uint8_t channel = 0; channel < 16; ++channel) {
            for (size_t pitch = 0; pitch < 128; ++pitch) {
                const size_t index = channel * 128 + pitch;
                plugin->reset_notes[index] = std::max(plugin->reset_notes[index], plugin->active_notes[index]);
            }
            plugin->reset_channels |= static_cast<uint16_t>(1u << channel);
            plugin->reset_pedal_channels |= static_cast<uint16_t>(1u << channel);
        }
    }
    plugin->notes_staged = false;

    if (result == kResultOk && plugin->symbolic_sample_size == kSample64) {
        for (uint32_t ch = 0; ch < num_output_channels; ++ch) {
            const auto& converted = plugin->output_f64[ch];
            for (uint32_t frame = 0; frame < frames; ++frame) {
                outputs[ch][frame] = static_cast<float>(converted[frame]);
            }
        }
    }

    // Parameter changes emitted by the processor may only be reflected into
    // the edit controller from the control/UI thread.
    const int32 output_parameter_count = plugin->output_param_changes.getParameterCount();
    for (int32 parameter_index = 0; parameter_index < output_parameter_count; ++parameter_index) {
        IParamValueQueue* queue =
            plugin->output_param_changes.getParameterData(parameter_index);
        if (!queue || queue->getPointCount() <= 0) continue;
        int32 sample_offset = 0;
        ParamValue value = 0.0;
        if (queue->getPoint(queue->getPointCount() - 1, sample_offset, value) == kResultTrue) {
            plugin->audio_to_ui_parameters.push({queue->getParameterId(), value, sample_offset});
        }
    }

    // Clear input/output events and parameter changes for next call
    plugin->input_events.clear();
    plugin->input_param_changes.clearQueue();
    plugin->output_events.clear();
    plugin->output_param_changes.clearQueue();

    if (num_input_channels > 0) {
        AudioBusBuffers& bus = plugin->process_data.inputs[0];
        for (uint32_t ch = 0; ch < num_input_channels; ++ch) {
            if (plugin->symbolic_sample_size == kSample32) bus.channelBuffers32[ch] = nullptr;
            else bus.channelBuffers64[ch] = nullptr;
        }
    }
    if (num_output_channels > 0) {
        AudioBusBuffers& bus = plugin->process_data.outputs[0];
        for (uint32_t ch = 0; ch < num_output_channels; ++ch) {
            if (plugin->symbolic_sample_size == kSample32) bus.channelBuffers32[ch] = nullptr;
            else bus.channelBuffers64[ch] = nullptr;
        }
    }

    plugin->sample_position += frames;

    return (result == kResultOk) ? RACK_VST3_OK : RACK_VST3_ERROR_GENERIC;
}

// ============================================================================
// Parameter API
// ============================================================================

int rack_vst3_plugin_parameter_count(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->controller) {
        return 0;
    }
    return static_cast<int>(plugin->parameters.size());
}

int rack_vst3_plugin_get_parameter(RackVST3Plugin* plugin, uint32_t index, float* value) {
    if (!plugin || !plugin->controller || !value) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    rack_vst3_drain_audio_parameters(plugin);
    rack_vst3_service_restart_notifications(plugin);

    if (index >= plugin->parameters.size()) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    ParamID param_id = plugin->parameters[index].id;
    ParamValue normalized = plugin->controller->getParamNormalized(param_id);
    *value = static_cast<float>(normalized);

    return RACK_VST3_OK;
}

int rack_vst3_plugin_set_parameter(RackVST3Plugin* plugin, uint32_t index, float value) {
    if (!plugin || !plugin->controller) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);

    if (index >= plugin->parameters.size()) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    // Clamp normalized value to 0.0-1.0
    // VST3 uses normalized parameter values (0.0-1.0 range)
    // This matches AudioUnit behavior for consistency across plugin formats
    if (value < 0.0f) value = 0.0f;
    if (value > 1.0f) value = 1.0f;

    ParamID param_id = plugin->parameters[index].id;

    // Set parameter value on controller (for UI reflection)
    plugin->controller->setParamNormalized(param_id, value);
    rack_vst3_track_parameter_control(plugin, param_id, value, true);
    plugin->dirty.store(true, std::memory_order_release);

    return plugin->ui_to_audio_parameters.push({param_id, value, 0})
        ? RACK_VST3_OK
        : RACK_VST3_ERROR_GENERIC;
}

int rack_vst3_plugin_set_parameter_audio(RackVST3Plugin* plugin, uint32_t index, float value) {
    if (!plugin || !plugin->initialized || index >= plugin->parameters.size() || !std::isfinite(value)) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    const PendingParameterChange change{
        plugin->parameters[index].id,
        std::clamp<ParamValue>(value, 0.0, 1.0),
        0,
    };
    for (size_t existing = 0; existing < plugin->direct_audio_parameter_count; ++existing) {
        if (plugin->direct_audio_parameters[existing].id == change.id) {
            plugin->direct_audio_parameters[existing] = change;
            rack_vst3_publish_audio_parameter(plugin, index, change.value);
            return RACK_VST3_OK;
        }
    }
    if (plugin->direct_audio_parameter_count >= plugin->direct_audio_parameters.size()) {
        return RACK_VST3_ERROR_GENERIC;
    }
    plugin->direct_audio_parameters[plugin->direct_audio_parameter_count++] = change;
    rack_vst3_publish_audio_parameter(plugin, index, change.value);
    return RACK_VST3_OK;
}

int rack_vst3_plugin_prepare_live_parameter_restore(RackVST3Plugin* plugin, const uint32_t* indices, uint32_t count) {
    if (!plugin || (!indices && count != 0)) return RACK_VST3_ERROR_INVALID_PARAM;
    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    std::vector<ParamID> superseded;
    superseded.reserve(count);
    for (uint32_t offset = 0; offset < count; ++offset) {
        if (indices[offset] >= plugin->parameters.size()) return RACK_VST3_ERROR_INVALID_PARAM;
        superseded.push_back(plugin->parameters[indices[offset]].id);
    }
    std::sort(superseded.begin(), superseded.end());
    plugin->ui_to_audio_parameters.discard_if([&](const PendingParameterChange& change) {
        return std::binary_search(superseded.begin(), superseded.end(), change.id);
    });
    return RACK_VST3_OK;
}

int rack_vst3_plugin_tracked_parameter_count(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->controller) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    rack_vst3_drain_audio_parameters(plugin);
    std::lock_guard<std::mutex> persistent_lock(plugin->persistent_parameter_mutex);
    return static_cast<int>(plugin->persistent_parameters.size());
}

int rack_vst3_plugin_get_tracked_parameter(
    RackVST3Plugin* plugin,
    uint32_t index,
    uint32_t* parameter_id,
    float* value) {
    if (!plugin || !parameter_id || !value) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    rack_vst3_drain_audio_parameters(plugin);
    std::lock_guard<std::mutex> persistent_lock(plugin->persistent_parameter_mutex);
    if (index >= plugin->persistent_parameters.size()) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    const auto& parameter = plugin->persistent_parameters[index];
    *parameter_id = static_cast<uint32_t>(parameter.id);
    *value = static_cast<float>(parameter.value);
    return RACK_VST3_OK;
}

int rack_vst3_plugin_parameter_info(
    RackVST3Plugin* plugin,
    uint32_t index,
    char* name,
    size_t name_size,
    float* min,
    float* max,
    float* default_value,
    uint32_t* parameter_id,
    uint32_t* flags,
    int32_t* step_count,
    int32_t* unit_id,
    char* unit,
    size_t unit_size)
{
    if (!plugin || !plugin->controller || !name || name_size == 0) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    if (index >= plugin->parameters.size()) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    const auto& param_info = plugin->parameters[index];

    // Name
    strncpy(name, param_info.title.c_str(), name_size - 1);
    name[name_size - 1] = '\0';

    // Min/max/default
    if (min) *min = static_cast<float>(param_info.min_value);
    if (max) *max = static_cast<float>(param_info.max_value);
    if (default_value) *default_value = static_cast<float>(param_info.default_value);
    if (parameter_id) *parameter_id = static_cast<uint32_t>(param_info.id);
    if (flags) *flags = static_cast<uint32_t>(param_info.flags);
    if (step_count) *step_count = param_info.step_count;
    if (unit_id) *unit_id = param_info.unit_id;

    // Unit
    if (unit && unit_size > 0) {
        strncpy(unit, param_info.units.c_str(), unit_size - 1);
        unit[unit_size - 1] = '\0';
    }

    return RACK_VST3_OK;
}

// ============================================================================
// Preset Management (Stub - TODO: Implement)
// ============================================================================

int rack_vst3_plugin_get_preset_count(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->initialized) {
        return 0;
    }
    return static_cast<int>(plugin->presets.size());
}

int rack_vst3_plugin_get_preset_info(
    RackVST3Plugin* plugin,
    uint32_t index,
    char* name,
    size_t name_size,
    int32_t* preset_number)
{
    if (!plugin || !plugin->initialized || !name || name_size == 0) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    if (index >= plugin->presets.size()) {
        return RACK_VST3_ERROR_NOT_FOUND;
    }

    const auto& preset = plugin->presets[index];

    // Copy name
    strncpy(name, preset.name.c_str(), name_size - 1);
    name[name_size - 1] = '\0';

    // Preset number is just the index
    if (preset_number) {
        *preset_number = static_cast<int32_t>(index);
    }

    return RACK_VST3_OK;
}

int rack_vst3_plugin_load_preset(RackVST3Plugin* plugin, int32_t preset_number) {
    if (!plugin || !plugin->initialized || !plugin->controller) {
        return RACK_VST3_ERROR_NOT_INITIALIZED;
    }

    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);

    if (preset_number < 0 || preset_number >= static_cast<int32_t>(plugin->presets.size())) {
        return RACK_VST3_ERROR_NOT_FOUND;
    }

    const auto& preset = plugin->presets[preset_number];
    if (preset.parameter_id == kNoParamId || preset.parameter_steps <= 0
        || preset.program_index > preset.parameter_steps) {
        return RACK_VST3_ERROR_NOT_SUPPORTED;
    }
    const ParamValue normalized = static_cast<ParamValue>(preset.program_index)
        / static_cast<ParamValue>(preset.parameter_steps);
    const tresult result =
        plugin->controller->setParamNormalized(preset.parameter_id, normalized);
    if (result != kResultOk && result != kResultTrue) {
        return RACK_VST3_ERROR_GENERIC;
    }
    rack_vst3_track_parameter_control(plugin, preset.parameter_id, normalized, true);
    plugin->dirty.store(true, std::memory_order_release);
    return plugin->ui_to_audio_parameters.push({preset.parameter_id, normalized, 0})
        ? RACK_VST3_OK
        : RACK_VST3_ERROR_GENERIC;
}

// Each plugin callback owns a zero-based stream: seek(0) must never reach
// the host envelope or the other component's state.
static int rack_vst3_serialize_state(RackVST3Plugin* plugin, std::vector<uint8_t>& data) {
    IPtr<MemoryStream> component(new MemoryStream(), false);
    const tresult component_result = plugin->component->getState(component);
    if (component_result != kResultOk) {
        return (component_result == kResultFalse || component_result == kNotImplemented)
            ? RACK_VST3_ERROR_NOT_SUPPORTED
            : RACK_VST3_ERROR_GENERIC;
    }

    IPtr<MemoryStream> controller(new MemoryStream(), false);
    if (plugin->controller && plugin->separate_controller) {
        const tresult result = plugin->controller->getState(controller);
        if (result != kResultOk && result != kResultFalse && result != kNotImplemented) {
            return RACK_VST3_ERROR_GENERIC;
        }
    }
    if (component->getSize() == 0 && controller->getSize() == 0) {
        return RACK_VST3_ERROR_NOT_SUPPORTED;
    }

    const uint32_t component_size = static_cast<uint32_t>(component->getSize());
    data.resize(sizeof(component_size));
    memcpy(data.data(), &component_size, sizeof(component_size));
    data.insert(data.end(), component->getData().begin(), component->getData().end());
    data.insert(data.end(), controller->getData().begin(), controller->getData().end());
    return RACK_VST3_OK;
}

int rack_vst3_plugin_get_state_size(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->component) {
        return 0;
    }
    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    rack_vst3_drain_audio_parameters(plugin);
    std::vector<uint8_t> data;
    const int result = rack_vst3_serialize_state(plugin, data);
    return result == RACK_VST3_OK ? static_cast<int>(data.size()) : result;
}

int rack_vst3_plugin_get_state(RackVST3Plugin* plugin, uint8_t* data, size_t* size) {
    if (!plugin || !data || !size || *size == 0 || !plugin->component) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    rack_vst3_drain_audio_parameters(plugin);
    std::vector<uint8_t> state;
    const int result = rack_vst3_serialize_state(plugin, state);
    if (result != RACK_VST3_OK) return result;
    if (state.size() > *size) {
        *size = state.size();
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    memcpy(data, state.data(), state.size());
    *size = state.size();
    return RACK_VST3_OK;
}

int rack_vst3_plugin_set_state(RackVST3Plugin* plugin, const uint8_t* data, size_t size) {
    if (!plugin || !data || size == 0 || !plugin->component) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);

    const bool has_separate_controller = plugin->controller && plugin->separate_controller;

    auto apply_legacy_sequence = [&]() -> int {
        IPtr<MemoryStream> legacy_stream(new MemoryStream(data, size), false);
        tresult result = plugin->component->setState(legacy_stream);
        if (result != kResultOk) {
            return (result == kResultFalse || result == kNotImplemented)
                ? RACK_VST3_ERROR_NOT_SUPPORTED
                : RACK_VST3_ERROR_GENERIC;
        }

        if (has_separate_controller) {
            IPtr<MemoryStream> component_for_controller(new MemoryStream(data, size), false);
            result = plugin->controller->setComponentState(component_for_controller);
            if (result != kResultOk && result != kResultFalse && result != kNotImplemented) {
                return RACK_VST3_ERROR_GENERIC;
            }
        }
        return RACK_VST3_OK;
    };

    // Serialized layout (current host format):
    // [u32 component_size][component_state][controller_state]
    // Keep a legacy fallback for older/raw states without a valid size marker.
    IPtr<MemoryStream> stream(new MemoryStream(data, size), false);
    uint32_t component_state_size = 0;
    int32 bytes_read = 0;
    tresult result =
        stream->read(&component_state_size, sizeof(component_state_size), &bytes_read);
    if (result != kResultOk || bytes_read != sizeof(component_state_size)) {
        return apply_legacy_sequence();
    }

    const size_t payload_size = size - sizeof(uint32_t);
    if (static_cast<size_t>(component_state_size) > payload_size) {
        // Not in host format: try legacy/raw restore behavior.
        return apply_legacy_sequence();
    }

    const uint8_t* component_data = data + sizeof(uint32_t);
    const size_t component_size = static_cast<size_t>(component_state_size);
    const uint8_t* controller_data = component_data + component_size;
    const size_t controller_size = payload_size - component_size;

    // 1) component->setState(component_state)
    IPtr<MemoryStream> component_stream(new MemoryStream(component_data, component_size), false);
    result = plugin->component->setState(component_stream);
    if (result != kResultOk) {
        return (result == kResultFalse || result == kNotImplemented)
            ? RACK_VST3_ERROR_NOT_SUPPORTED
            : RACK_VST3_ERROR_GENERIC;
    }

    if (has_separate_controller) {
        // 2) controller->setComponentState(component_state)
        // This synchronization step is required by the VST3 persistence sequence.
        IPtr<MemoryStream> component_for_controller(
            new MemoryStream(component_data, component_size),
            false);
        result = plugin->controller->setComponentState(component_for_controller);
        if (result != kResultOk && result != kResultFalse && result != kNotImplemented) {
            return RACK_VST3_ERROR_GENERIC;
        }

        // 3) controller->setState(controller_state)
        if (controller_size > 0) {
            IPtr<MemoryStream> controller_stream(
                new MemoryStream(controller_data, controller_size),
                false);
            result = plugin->controller->setState(controller_stream);
            if (result != kResultOk && result != kResultFalse && result != kNotImplemented) {
                return RACK_VST3_ERROR_GENERIC;
            }
        }
    }

    rack_vst3_drain_audio_parameters(plugin);
    plugin->dirty.store(false, std::memory_order_release);
    return RACK_VST3_OK;
}

// ============================================================================
// GUI API
// ============================================================================

static bool rack_vst3_get_view_size(IPlugView* view, float* width, float* height) {
    if (!view || !width || !height) {
        return false;
    }

    ViewRect rect;
    if (view->getSize(&rect) != kResultOk) {
        return false;
    }

    *width = static_cast<float>(rect.right - rect.left);
    *height = static_cast<float>(rect.bottom - rect.top);
    return true;
}

static size_t rack_vst3_midi_mapping_index(int16 channel, CtrlNumber controller_number) {
    return static_cast<size_t>(channel) * static_cast<size_t>(kCountCtrlNumber)
        + static_cast<size_t>(controller_number);
}

static bool rack_vst3_try_queue_midi_mapping(
    RackVST3Plugin* plugin,
    int16 channel,
    CtrlNumber controller_number,
    ParamValue value_normalized,
    int32 sample_offset)
{
    if (!plugin || channel < 0 || channel >= 16 || controller_number < 0
        || controller_number >= kCountCtrlNumber)
    {
        return false;
    }

    if (plugin->midi_controller_assignments.empty()) {
        return false;
    }

    ParamID param_id =
        plugin->midi_controller_assignments[rack_vst3_midi_mapping_index(channel, controller_number)];
    if (param_id == kNoParamId) {
        return false;
    }

    int32 queue_index = 0;
    IParamValueQueue* queue = plugin->input_param_changes.addParameterData(param_id, queue_index);
    if (!queue) {
        return false;
    }

    int32 point_index = 0;
    tresult result = queue->addPoint(sample_offset, value_normalized, point_index);
    return result == kResultTrue || result == kResultOk;
}

int rack_vst3_plugin_editor_size(RackVST3Plugin* plugin, float* width, float* height) {
    if (!plugin || !plugin->controller || !width || !height) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    IPtr<IPlugView> view(plugin->controller->createView(ViewType::kEditor), false);
    if (!view) {
        return RACK_VST3_ERROR_NOT_FOUND;
    }

#if SMTG_OS_WINDOWS
    if (view->isPlatformTypeSupported(kPlatformTypeHWND) != kResultTrue) {
        return RACK_VST3_ERROR_NOT_SUPPORTED;
    }
#endif

    if (!rack_vst3_get_view_size(view, width, height)) {
        return RACK_VST3_ERROR_GENERIC;
    }

    return RACK_VST3_OK;
}

RackVST3Gui* rack_vst3_plugin_create_gui(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->controller) {
        return nullptr;
    }

    std::lock_guard<std::mutex> state_lock(plugin->state_mutex);
    IPtr<IPlugView> view(plugin->controller->createView(ViewType::kEditor), false);
    if (!view) {
        return nullptr;
    }

#if SMTG_OS_WINDOWS
    if (view->isPlatformTypeSupported(kPlatformTypeHWND) != kResultTrue) {
        return nullptr;
    }
#else
    return nullptr;
#endif

    auto gui = new (std::nothrow) RackVST3Gui();
    if (!gui) {
        return nullptr;
    }
    gui->plugin = plugin;
    gui->view = view;
    return gui;
}

void rack_vst3_gui_free(RackVST3Gui* gui) {
    if (!gui) {
        return;
    }

    if (gui->attached) {
        rack_vst3_gui_detach(gui);
    }

    gui->view = nullptr;
#if SMTG_OS_WINDOWS
    gui->frame = nullptr;
    gui->host_window = nullptr;
    gui->container_window = nullptr;
#endif
    delete gui;
}

int rack_vst3_gui_attach(RackVST3Gui* gui, void* parent) {
    if (!gui || !gui->plugin || !gui->view || !parent) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    if (gui->attached) {
        return RACK_VST3_OK;
    }

    std::lock_guard<std::mutex> state_lock(gui->plugin->state_mutex);

#if SMTG_OS_WINDOWS
    HWND parent_hwnd = static_cast<HWND>(parent);
    const UINT dpi = std::max<UINT>(GetDpiForWindow(parent_hwnd), 96);
    FUnknownPtr<IPlugViewContentScaleSupport> scale_support(gui->view);
    if (scale_support) {
        scale_support->setContentScaleFactor(static_cast<float>(dpi) / 96.0f);
    }
    float width = 800.0f;
    float height = 600.0f;
    rack_vst3_get_view_size(gui->view, &width, &height);

    HWND container = CreateWindowExW(
        0,
        L"STATIC",
        L"",
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
        0,
        0,
        static_cast<int>(width),
        static_cast<int>(height),
        parent_hwnd,
        nullptr,
        GetModuleHandleW(nullptr),
        nullptr
    );
    if (!container) {
        return RACK_VST3_ERROR_GENERIC;
    }

    gui->frame = FUnknownPtr<IPlugFrame>(new RackVST3PlugFrame(parent_hwnd, container));
    gui->view->setFrame(gui->frame);

    const tresult result = gui->view->attached(container, kPlatformTypeHWND);
    if (result != kResultOk && result != kResultTrue) {
        gui->view->setFrame(nullptr);
        gui->frame = nullptr;
        DestroyWindow(container);
        return RACK_VST3_ERROR_GENERIC;
    }

    ViewRect attached_size{};
    if (gui->view->getSize(&attached_size) == kResultOk) {
        gui->frame->resizeView(gui->view, &attached_size);
    }

    gui->host_window = parent_hwnd;
    gui->container_window = container;
    gui->attached = true;
    return RACK_VST3_OK;
#else
    (void)parent;
    return RACK_VST3_ERROR_NOT_SUPPORTED;
#endif
}

int rack_vst3_gui_detach(RackVST3Gui* gui) {
    if (!gui || !gui->plugin || !gui->view) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    if (!gui->attached) {
        return RACK_VST3_OK;
    }

    std::lock_guard<std::mutex> state_lock(gui->plugin->state_mutex);

    gui->view->removed();
    gui->view->setFrame(nullptr);

#if SMTG_OS_WINDOWS
    if (gui->container_window) {
        DestroyWindow(gui->container_window);
        gui->container_window = nullptr;
    }
    gui->host_window = nullptr;
    gui->frame = nullptr;
#endif
    gui->attached = false;
    return RACK_VST3_OK;
}

int rack_vst3_gui_get_size(RackVST3Gui* gui, float* width, float* height) {
    if (!gui || !gui->view || !width || !height) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    std::lock_guard<std::mutex> state_lock(gui->plugin->state_mutex);
    return rack_vst3_get_view_size(gui->view, width, height) ? RACK_VST3_OK
                                                               : RACK_VST3_ERROR_GENERIC;
}

int rack_vst3_gui_set_content_scale_factor(RackVST3Gui* gui, float factor) {
    if (!gui || !gui->plugin || !gui->view || !std::isfinite(factor) || factor <= 0.0f) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    std::lock_guard<std::mutex> state_lock(gui->plugin->state_mutex);
    FUnknownPtr<IPlugViewContentScaleSupport> scale_support(gui->view);
    if (!scale_support) {
        return RACK_VST3_OK;
    }
    const tresult result = scale_support->setContentScaleFactor(factor);
    return (result == kResultOk || result == kResultTrue || result == kNotImplemented)
        ? RACK_VST3_OK
        : RACK_VST3_ERROR_GENERIC;
}

// ============================================================================
// MIDI API
// ============================================================================

static void rack_vst3_begin_midi_block(RackVST3Plugin* plugin) {
    if (plugin->notes_staged) return;
    plugin->notes_after_block = plugin->active_notes;
    plugin->reset_releases_in_block.fill(0);
    plugin->reset_events_in_block = 0;
    plugin->notes_staged = true;
}

static void rack_vst3_reset_channel(RackVST3Plugin* plugin, uint8_t channel) {
    if (channel >= 16) return;
    plugin->reset_channels |= static_cast<uint16_t>(1u << channel);
    for (size_t pitch = 0; pitch < 128; ++pitch) {
        const size_t index = size_t(channel) * 128 + pitch;
        const uint32_t unsent = plugin->reset_notes[index] - plugin->reset_releases_in_block[index];
        plugin->reset_notes[index] = plugin->reset_releases_in_block[index]
            + std::max(unsent, plugin->notes_after_block[index]);
    }
}

static void rack_vst3_queue_emergency_controllers(RackVST3Plugin* plugin, uint32_t sample_offset) {
    // Pedal release must reach every channel even when the first channel alone
    // consumes this block's entire explicit Note-Off budget.
    for (uint8_t channel = 0; channel < 16; ++channel) {
        if (plugin->reset_pedal_channels & (1u << channel)) {
            // Keep the original CC64/120/121/123 panic contract, and also
            // release sostenuto/hold so explicit Note-Offs cannot stay latched.
            for (CtrlNumber controller : {CtrlNumber(64), CtrlNumber(66), CtrlNumber(69), CtrlNumber(120), CtrlNumber(121), CtrlNumber(123)}) {
                rack_vst3_try_queue_midi_mapping(plugin, channel, controller, 0.0, sample_offset);
            }
        }
    }
}

static void rack_vst3_queue_note_resets(RackVST3Plugin* plugin, uint32_t sample_offset) {
    rack_vst3_queue_emergency_controllers(plugin, sample_offset);
    for (uint8_t channel = 0; channel < 16; ++channel) {
        if ((plugin->reset_channels & (1u << channel)) == 0) continue;
        for (uint8_t pitch = 0; pitch < 128; ++pitch) {
            const size_t index = size_t(channel) * 128 + pitch;
            while (plugin->reset_releases_in_block[index] < plugin->reset_notes[index]) {
                if (plugin->reset_events_in_block >= kRealtimeEventCapacity) return;
                Event event{};
                event.type = Event::kNoteOffEvent;
                event.sampleOffset = sample_offset;
                event.ppqPosition = project_quarter_notes(plugin->sample_position, sample_offset, plugin->sample_rate);
                event.noteOff.channel = channel;
                event.noteOff.pitch = pitch;
                event.noteOff.noteId = -1;
                if (plugin->input_events.addEvent(event) != kResultOk) return;
                ++plugin->reset_releases_in_block[index];
                ++plugin->reset_events_in_block;
                if (plugin->notes_after_block[index] > 0) --plugin->notes_after_block[index];
            }
        }
    }
}

int rack_vst3_plugin_request_midi_reset(RackVST3Plugin* plugin) {
    if (!plugin || !plugin->initialized) return RACK_VST3_ERROR_NOT_INITIALIZED;
    plugin->input_events.clear();
    plugin->deferred_midi_count = 0;
    plugin->notes_staged = false;
    rack_vst3_begin_midi_block(plugin);
    plugin->reset_pedal_channels = UINT16_MAX;
    for (uint8_t channel = 0; channel < 16; ++channel) rack_vst3_reset_channel(plugin, channel);
    return RACK_VST3_OK;
}

int rack_vst3_plugin_midi_reset_pending(RackVST3Plugin* plugin) {
    return plugin && (plugin->reset_channels != 0 || plugin->deferred_midi_count != 0);
}

static bool rack_vst3_unsent_note_resets(const RackVST3Plugin* plugin) {
    if (!plugin->reset_channels) return false;
    for (size_t index = 0; index < plugin->reset_notes.size(); ++index) {
        if (plugin->reset_notes[index] > plugin->reset_releases_in_block[index]) return true;
    }
    return false;
}

static bool rack_vst3_would_erase_pedal_release(RackVST3Plugin* plugin, const RackVST3MidiEvent& event) {
    if ((event.status & 0xF0) != 0xB0 || event.channel >= 16 || event.data2 < 64
        || (event.data1 != 64 && event.data1 != 66 && event.data1 != 69)
        || plugin->midi_controller_assignments.empty()) return false;
    const ParamID id = plugin->midi_controller_assignments[
        rack_vst3_midi_mapping_index(event.channel, CtrlNumber(event.data1))];
    if (id == kNoParamId) return false;
    for (int32 index = 0; index < plugin->input_param_changes.getParameterCount(); ++index) {
        auto* queue = plugin->input_param_changes.getParameterData(index);
        if (queue->getParameterId() != id || queue->getPointCount() == 0) continue;
        int32 offset = 0;
        ParamValue value = 0.0;
        if (queue->getPoint(queue->getPointCount() - 1, offset, value) == kResultTrue) {
            return offset == int32(event.sample_offset) && value < 0.5;
        }
    }
    return false;
}

static int rack_vst3_queue_midi_for_block(
    RackVST3Plugin* plugin,
    const RackVST3MidiEvent* events,
    uint32_t event_count,
    uint32_t frames)
{
    if (!plugin || !plugin->initialized) {
        return RACK_VST3_ERROR_NOT_INITIALIZED;
    }

    // Null events array is only valid if event_count is 0
    if (!events && event_count > 0) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    if (event_count > kRealtimeEventCapacity - plugin->deferred_midi_count) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }

    rack_vst3_begin_midi_block(plugin);
    rack_vst3_queue_note_resets(plugin, 0);

    if (rack_vst3_unsent_note_resets(plugin) || plugin->reset_pedal_channels != 0) {
        // Rust sees zero admission capacity until this panic and its FIFO tail
        // complete. No retrigger can reach a processor before the old releases.
        for (uint32_t i = 0; i < event_count; ++i) {
            auto& deferred = plugin->deferred_midi[plugin->deferred_midi_count++];
            deferred = events[i];
        }
        return RACK_VST3_OK;
    }

    std::array<RackVST3MidiEvent, kRealtimeEventCapacity> batch{};
    const size_t deferred_count = plugin->deferred_midi_count;
    size_t batch_count = deferred_count;
    std::copy_n(plugin->deferred_midi.begin(), batch_count, batch.begin());
    for (uint32_t i = 0; i < event_count; ++i) batch[batch_count++] = events[i];
    plugin->deferred_midi_count = 0;

    // Convert MIDI events to VST3 events
    bool event_overflow = false;
    size_t consumed = 0;
    uint32_t last_offset = 0;
    for (size_t i = 0; i < batch_count; ++i) {
        if (deferred_count != 0) {
            if (i >= frames) break;
            // Distinct points preserve pedal-off/on transitions. If a variable
            // block is too short, retain the FIFO tail for the following block.
            batch[i].sample_offset = static_cast<uint32_t>(i);
        }
        auto midi_event = batch[i];
        midi_event.sample_offset = std::max(last_offset, midi_event.sample_offset);
        if (rack_vst3_would_erase_pedal_release(plugin, midi_event)) {
            if (frames == 0 || midi_event.sample_offset >= frames - 1) break;
            ++midi_event.sample_offset;
        }
        last_offset = midi_event.sample_offset;
        consumed = i + 1;

        Event vst3_event;
        memset(&vst3_event, 0, sizeof(Event));
        vst3_event.sampleOffset = midi_event.sample_offset;
        // Event and ProcessContext positions must describe the same timeline.
        // Leaving ppqPosition at zero timestamps every note at project start.
        vst3_event.ppqPosition = project_quarter_notes(
            plugin->sample_position, midi_event.sample_offset, plugin->sample_rate);
        vst3_event.busIndex = 0;

        uint8_t status = midi_event.status & 0xF0;

        switch (status) {
            case 0x90:  // Note On
                vst3_event.type = Event::kNoteOnEvent;
                vst3_event.noteOn.channel = midi_event.channel;
                vst3_event.noteOn.pitch = midi_event.data1;
                vst3_event.noteOn.velocity = static_cast<float>(midi_event.data2) / 127.0f;
                vst3_event.noteOn.noteId = -1;  // Not specified
                if (plugin->input_events.addEvent(vst3_event) == kResultOk) {
                    if (midi_event.channel < 16 && midi_event.data1 < 128) {
                        const size_t index = size_t(midi_event.channel) * 128 + midi_event.data1;
                        if (plugin->active_notes[index] != UINT32_MAX) ++plugin->active_notes[index];
                        if (plugin->notes_after_block[index] != UINT32_MAX) ++plugin->notes_after_block[index];
                    }
                } else event_overflow = true;
                break;

            case 0x80:  // Note Off
                vst3_event.type = Event::kNoteOffEvent;
                vst3_event.noteOff.channel = midi_event.channel;
                vst3_event.noteOff.pitch = midi_event.data1;
                vst3_event.noteOff.velocity = static_cast<float>(midi_event.data2) / 127.0f;
                vst3_event.noteOff.noteId = -1;  // Not specified
                if (plugin->input_events.addEvent(vst3_event) == kResultOk) {
                    if (midi_event.channel < 16 && midi_event.data1 < 128) {
                        auto& count = plugin->notes_after_block[size_t(midi_event.channel) * 128 + midi_event.data1];
                        if (count > 0) --count;
                    }
                } else event_overflow = true;
                break;

            case 0xA0:  // Polyphonic Key Pressure (Aftertouch)
                vst3_event.type = Event::kPolyPressureEvent;
                vst3_event.polyPressure.channel = midi_event.channel;
                vst3_event.polyPressure.pitch = midi_event.data1;
                vst3_event.polyPressure.pressure = static_cast<float>(midi_event.data2) / 127.0f;
                event_overflow |= plugin->input_events.addEvent(vst3_event) != kResultOk;
                break;

            case 0xB0:  // Control Change
                rack_vst3_try_queue_midi_mapping(
                    plugin,
                    midi_event.channel,
                    static_cast<CtrlNumber>(midi_event.data1),
                    static_cast<ParamValue>(midi_event.data2) / 127.0,
                    midi_event.sample_offset);
                if (midi_event.data1 == 120 || midi_event.data1 == 123) {
                    rack_vst3_reset_channel(plugin, midi_event.channel);
                    rack_vst3_queue_note_resets(plugin, midi_event.sample_offset);
                }
                break;

            case 0xC0:  // Program Change
                if (midi_event.channel < plugin->midi_program_assignments.size()) {
                    const ParamID program_id =
                        plugin->midi_program_assignments[midi_event.channel];
                    const int32 steps = plugin->midi_program_steps[midi_event.channel];
                    if (program_id != kNoParamId && steps > 0
                        && static_cast<int32>(midi_event.data1) <= steps) {
                        int32 queue_index = 0;
                        if (IParamValueQueue* queue =
                                plugin->input_param_changes.addParameterData(program_id, queue_index)) {
                            int32 point_index = 0;
                            const ParamValue value = static_cast<ParamValue>(midi_event.data1)
                                / static_cast<ParamValue>(steps);
                            event_overflow |= queue->addPoint(
                                midi_event.sample_offset, value, point_index) != kResultTrue;
                        }
                    }
                }
                break;

            case 0xD0:  // Channel Pressure (Aftertouch)
                rack_vst3_try_queue_midi_mapping(
                    plugin,
                    midi_event.channel,
                    kAfterTouch,
                    static_cast<ParamValue>(midi_event.data1) / 127.0,
                    midi_event.sample_offset);
                break;

            case 0xE0: {  // Pitch Bend
                const uint16_t pitch_bend = static_cast<uint16_t>(midi_event.data1)
                    | (static_cast<uint16_t>(midi_event.data2) << 7);
                rack_vst3_try_queue_midi_mapping(
                    plugin,
                    midi_event.channel,
                    kPitchBend,
                    static_cast<ParamValue>(pitch_bend) / 16383.0,
                    midi_event.sample_offset);
                break;
            }

            default:
                // Unknown MIDI event - skip it
                continue;
        }
        if (rack_vst3_unsent_note_resets(plugin)) {
            break;
        }
    }
    for (size_t retained = consumed; retained < batch_count; ++retained) {
        plugin->deferred_midi[plugin->deferred_midi_count++] = batch[retained];
    }

    return event_overflow ? RACK_VST3_ERROR_GENERIC : RACK_VST3_OK;
}

int rack_vst3_plugin_send_midi(
    RackVST3Plugin* plugin, const RackVST3MidiEvent* events, uint32_t event_count)
{
    return rack_vst3_queue_midi_for_block(
        plugin, events, event_count, plugin ? plugin->max_block_size : 0);
}

int rack_vst3_plugin_process_with_midi(
    RackVST3Plugin* plugin,
    const RackVST3MidiEvent* events,
    uint32_t event_count,
    const float* const* inputs,
    uint32_t num_input_channels,
    float* const* outputs,
    uint32_t num_output_channels,
    uint32_t frames)
{
    if (event_count > kRealtimeEventCapacity) {
        return RACK_VST3_ERROR_INVALID_PARAM;
    }
    const int midi_result = rack_vst3_queue_midi_for_block(plugin, events, event_count, frames);
    if (midi_result != RACK_VST3_OK) {
        if (plugin) {
            plugin->input_events.clear();
            plugin->input_param_changes.clearQueue();
            rack_vst3_plugin_request_midi_reset(plugin);
        }
        return midi_result;
    }
    return rack_vst3_plugin_process(
        plugin,
        inputs,
        num_input_channels,
        outputs,
        num_output_channels,
        frames);
}
