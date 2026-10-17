// Exercise the actual host implementation without a driver or third-party DLL.
#include "../src/vst3_instance.cpp"
#include <new>

static bool count_allocations = false;
static size_t allocations = 0;
void* operator new(std::size_t bytes) {
    if (count_allocations) ++allocations;
    if (void* p = std::malloc(bytes ? bytes : 1)) return p;
    throw std::bad_alloc();
}
void operator delete(void* p) noexcept { std::free(p); }
void operator delete(void* p, std::size_t) noexcept { std::free(p); }

static void require(bool condition, const char* message) {
    if (!condition) { std::fprintf(stderr, "FAIL: %s\n", message); std::exit(1); }
}

class NoteProcessor final : public IAudioProcessor {
public:
    std::array<int, 16 * 128> notes{};
    bool reject = false;
    double last_parameter_value = -1;
    int32 parameter_points = 0;
    std::array<float, 2048> voices{};
    size_t voice_count = 0;
    int32 last_parameter_points = 0;
    int32 last_note_offset = -1;
    std::array<double, 512> parameter_values{};
    std::array<int32, 512> parameter_offsets{};
    tresult PLUGIN_API queryInterface(const TUID, void** obj) override { *obj = nullptr; return kNoInterface; }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }
    tresult PLUGIN_API setBusArrangements(SpeakerArrangement*, int32, SpeakerArrangement*, int32) override { return kResultOk; }
    tresult PLUGIN_API getBusArrangement(BusDirection, int32, SpeakerArrangement&) override { return kResultOk; }
    tresult PLUGIN_API canProcessSampleSize(int32) override { return kResultOk; }
    uint32 PLUGIN_API getLatencySamples() override { return 0; }
    tresult PLUGIN_API setupProcessing(ProcessSetup&) override { return kResultOk; }
    tresult PLUGIN_API setProcessing(TBool) override { return kResultOk; }
    uint32 PLUGIN_API getTailSamples() override { return 0; }
    tresult PLUGIN_API process(ProcessData& data) override {
        if (reject) return kResultFalse;
        for (int32 i = 0; i < data.inputParameterChanges->getParameterCount(); ++i) {
            auto* queue = data.inputParameterChanges->getParameterData(i);
            parameter_points += queue->getPointCount();
            last_parameter_points = queue->getPointCount();
            for (int32 point = 0; point < std::min<int32>(512, last_parameter_points); ++point) {
                queue->getPoint(point, parameter_offsets[point], parameter_values[point]);
            }
            int32 offset = 0;
            if (queue->getPointCount()) queue->getPoint(queue->getPointCount() - 1, offset, last_parameter_value);
        }
        for (int32 i = 0; i < data.inputEvents->getEventCount(); ++i) {
            Event event{};
            data.inputEvents->getEvent(i, event);
            if (event.type == Event::kNoteOnEvent) {
                last_note_offset = event.sampleOffset;
                ++notes[event.noteOn.channel * 128 + event.noteOn.pitch];
                if (event.noteOn.channel == 0 && event.noteOn.pitch == 60) voices[voice_count++] = event.noteOn.velocity;
            }
            if (event.type == Event::kNoteOffEvent) {
                auto& note = notes[event.noteOff.channel * 128 + event.noteOff.pitch];
                if (note > 0) --note;
                if (event.noteOff.channel == 0 && event.noteOff.pitch == 60 && voice_count) --voice_count;
            }
        }
        return kResultOk;
    }
};

static int render(RackVST3Plugin& plugin, const RackVST3MidiEvent* events, uint32_t count) {
    return rack_vst3_plugin_process_with_midi(&plugin, events, count, nullptr, 0, nullptr, 0, 48000);
}

int main(int argc, char** argv) {
    if (argc == 2 && std::strcmp(argv[1], "host-refcounts") == 0) {
        auto verify = [](auto* object) {
            require(object->addRef() == 2, "host reference count must start at one regardless of heap contents");
            require(object->release() == 1, "temporary reference must preserve the host owner");
            require(object->release() == 0, "final owner must destroy the object");
        };
        auto poisoned = [](size_t size) {
            void* storage = ::operator new(size);
            std::memset(storage, 0xA5, size);
            return storage;
        };
        verify(new (poisoned(sizeof(MemoryStream))) MemoryStream());
        const uint8_t state[]{1, 2, 3};
        verify(new (poisoned(sizeof(MemoryStream))) MemoryStream(state, sizeof(state)));
        verify(new (poisoned(sizeof(RackVST3ComponentHandler))) RackVST3ComponentHandler(nullptr));
#if SMTG_OS_WINDOWS
        verify(new (poisoned(sizeof(RackVST3PlugFrame))) RackVST3PlugFrame(nullptr, nullptr));
#endif
        std::puts("PASS");
        return 0;
    }
    require(argc == 2, "select panic, allocations or parameters");
    NoteProcessor processor;
    auto plugin = std::make_unique<RackVST3Plugin>();
    plugin->processor = &processor;
    plugin->initialized = true;
    plugin->sample_rate = 48000;
    plugin->max_block_size = 48000;
    plugin->input_events.setMaxSize(kRealtimeInputEventCapacity);
    plugin->output_events.setMaxSize(512);
    if (std::strcmp(argv[1], "panic") == 0 || std::strcmp(argv[1], "panic-retry") == 0 || std::strcmp(argv[1], "panic-dense") == 0) {
        const RackVST3MidiEvent note{0, 0x90, 60, 100, 0};
        require(render(*plugin, &note, 1) == RACK_VST3_OK, "note processing");
        require(processor.notes[60] == 1, "fixture note is active");
        if (std::strcmp(argv[1], "panic-dense") == 0) {
            for (int voice = 1; voice < 1024; ++voice) require(render(*plugin, &note, 1) == RACK_VST3_OK, "dense note processing");
        }
        std::array<RackVST3MidiEvent, 64> panic{};
        size_t index = 0;
        for (uint8_t channel = 0; channel < 16; ++channel) {
            for (uint8_t controller : {64, 120, 121, 123}) panic[index++] = {0, 0xB0, controller, 0, channel};
        }
        if (std::strcmp(argv[1], "panic-retry") == 0) {
            processor.reject = true;
            require(render(*plugin, panic.data(), 64) != RACK_VST3_OK, "fixture rejects DSP block");
            processor.reject = false;
        } else {
            require(render(*plugin, panic.data(), 64) == RACK_VST3_OK, "panic processing");
        }
        for (int retry = 0; retry < 8; ++retry) require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "panic retry processing");
        require(processor.notes[60] == 0, "panic must stop notes without IMidiMapping");
    } else if (std::strcmp(argv[1], "deferred-pedals") == 0 || std::strcmp(argv[1], "emergency-tail") == 0 || std::strcmp(argv[1], "deferred-fresh") == 0) {
        plugin->midi_controller_assignments.assign(16 * kCountCtrlNumber, kNoParamId);
        plugin->midi_controller_assignments[64] = 42;
        plugin->input_param_changes.setMaxParameters(1);
        const RackVST3MidiEvent note{0, 0x90, 60, 100, 0};
        const RackVST3MidiEvent pedal{0, 0xB0, 64, 127, 0};
        require(render(*plugin, &pedal, 1) == RACK_VST3_OK, "pedal down");
        for (int i = 0; i < 1024; ++i) require(render(*plugin, &note, 1) == RACK_VST3_OK, "dense voices");
        const RackVST3MidiEvent events[]{{0, 0xB0, 123, 0, 0}, {100, 0xB0, 64, 0, 0}, {200, 0xB0, 64, 127, 0}};
        processor.reject = std::strcmp(argv[1], "emergency-tail") == 0;
        const int result = render(*plugin, events, 3);
        require(processor.reject ? result != RACK_VST3_OK : result == RACK_VST3_OK, "initial reset");
        if (processor.reject) {
            processor.reject = false;
            require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "first recovery block");
            require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "final recovery block");
            require(processor.last_parameter_value == 0.0, "emergency pedal release needs a successful block before deferred pedal-on");
        }
        if (std::strcmp(argv[1], "deferred-fresh") == 0) {
            const RackVST3MidiEvent fresh{0, 0xB0, 64, 64, 0};
            require(render(*plugin, &fresh, 1) == RACK_VST3_OK, "mix a fresh direct-API event with the deferred tail");
            require(processor.last_parameter_value == 64.0 / 127.0, "fresh events follow the deferred tail without timestamp regression");
            require(processor.last_parameter_points == 3, "all deferred and fresh pedal points survive");
        } else {
            // One-frame variable buffers cannot encode both transitions in one block.
            require(rack_vst3_plugin_process_with_midi(plugin.get(), nullptr, 0, nullptr, 0, nullptr, 0, 1) == RACK_VST3_OK, "short deferred block");
            require(processor.last_parameter_value == 0.0, "deferred pedal-off must not be coalesced away");
            require(rack_vst3_plugin_process_with_midi(plugin.get(), nullptr, 0, nullptr, 0, nullptr, 0, 1) == RACK_VST3_OK, "next deferred block");
            require(processor.last_parameter_value == 1.0, "deferred pedal-on follows the release");
        }
    } else if (std::strcmp(argv[1], "emergency-pedals") == 0) {
        plugin->midi_controller_assignments.assign(16 * kCountCtrlNumber, kNoParamId);
        plugin->midi_controller_assignments[kCountCtrlNumber + 64] = 42;
        plugin->input_param_changes.setMaxParameters(1);
        const RackVST3MidiEvent pedal{0, 0xB0, 64, 127, 1};
        const RackVST3MidiEvent note{0, 0x90, 60, 100, 0};
        require(render(*plugin, &pedal, 1) == RACK_VST3_OK, "pedal on another channel");
        for (int i = 0; i < 1024; ++i) require(render(*plugin, &note, 1) == RACK_VST3_OK, "dense voices");
        require(rack_vst3_plugin_request_midi_reset(plugin.get()) == RACK_VST3_OK, "emergency admission");
        require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "first reset block");
        require(processor.last_parameter_value == 0.0, "release all channel pedals before the note-off budget is exhausted");
        require(rack_vst3_plugin_midi_reset_pending(plugin.get()), "dense reset stays pending");
        processor.reject = true;
        require(render(*plugin, nullptr, 0) != RACK_VST3_OK, "rejected reset block");
        require(rack_vst3_plugin_midi_reset_pending(plugin.get()), "rejected reset is not acknowledged");
        processor.reject = false;
        for (int i = 0; i < 8; ++i) require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "reset retry");
        require(!rack_vst3_plugin_midi_reset_pending(plugin.get()) && processor.notes[60] == 0, "reset completed");
    } else if (std::strcmp(argv[1], "same-frame-pedals") == 0 || std::strcmp(argv[1], "emergency-ui") == 0) {
        plugin->midi_controller_assignments.assign(16 * kCountCtrlNumber, kNoParamId);
        plugin->midi_controller_assignments[64] = 42;
        plugin->input_param_changes.setMaxParameters(1);
        const RackVST3MidiEvent pedal{0, 0xB0, 64, 127, 0};
        require(render(*plugin, &pedal, 1) == RACK_VST3_OK, "initial pedal-on");
        if (std::strcmp(argv[1], "emergency-ui") == 0) {
            require(plugin->ui_to_audio_parameters.push({42, 1.0, 0}), "queued UI pedal edit");
            require(rack_vst3_plugin_request_midi_reset(plugin.get()) == RACK_VST3_OK, "panic admission");
            require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "panic processing");
            require(processor.last_parameter_value == 0.0, "UI proxy edits must not overwrite an emergency pedal release");
        } else {
            const RackVST3MidiEvent events[]{{0, 0xB0, 64, 0, 0}, {0, 0xB0, 64, 127, 0}, {0, 0x90, 60, 100, 0}};
            require(rack_vst3_plugin_process_with_midi(plugin.get(), events, 3, nullptr, 0, nullptr, 0, 1) == RACK_VST3_OK, "same-frame pedal transitions");
            require(processor.last_parameter_value == 0.0, "a same-frame pedal release must reach DSP before the next pedal-on");
            require(rack_vst3_plugin_process_with_midi(plugin.get(), nullptr, 0, nullptr, 0, nullptr, 0, 1) == RACK_VST3_OK, "deferred pedal-on");
            require(processor.last_parameter_value == 1.0, "same-frame pedal-on is preserved for the next block");
            require(processor.notes[60] == 0, "a deferred note must stay after the pedal-on");
            require(rack_vst3_plugin_process_with_midi(plugin.get(), nullptr, 0, nullptr, 0, nullptr, 0, 1) == RACK_VST3_OK, "deferred note");
            require(processor.notes[60] == 1, "deferred note is retained");
            require(rack_vst3_plugin_process_with_midi(plugin.get(), events, 3, nullptr, 0, nullptr, 0, 2) == RACK_VST3_OK, "two-frame pedal transitions");
            require(processor.last_parameter_points == 2 && processor.parameter_offsets[0] == 0 && processor.parameter_offsets[1] == 1, "pedal transitions occupy distinct frames");
            require(processor.last_note_offset == 1, "retiming a pedal-on keeps subsequent notes in FIFO order");
        }
    } else if (std::strcmp(argv[1], "emergency-controllers") == 0) {
        plugin->midi_controller_assignments.assign(16 * kCountCtrlNumber, kNoParamId);
        plugin->midi_controller_assignments[121] = 43;
        plugin->input_param_changes.setMaxParameters(1);
        require(plugin->ui_to_audio_parameters.push({43, 1.0, 0}), "initial controller state");
        require(render(*plugin, nullptr, 0) == RACK_VST3_OK && processor.last_parameter_value == 1.0, "controller fixture is nonzero");
        require(rack_vst3_plugin_request_midi_reset(plugin.get()) == RACK_VST3_OK, "emergency reset");
        require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "controller reset processing");
        require(processor.last_parameter_value == 0.0, "emergency path preserves the existing Reset All Controllers message");
    } else if (std::strcmp(argv[1], "panic-full") == 0) {
        const RackVST3MidiEvent note{0, 0x90, 60, 100, 0};
        require(render(*plugin, &note, 1) == RACK_VST3_OK, "initial note");
        require(render(*plugin, &note, 1) == RACK_VST3_OK, "second initial note");
        std::array<RackVST3MidiEvent, 512> events{};
        events.fill(note);
        events[0] = {0, 0xB0, 123, 0, 0};
        require(render(*plugin, events.data(), 512) == RACK_VST3_OK, "panic expansion must not reject a full admitted MIDI batch");
        require(processor.notes[60] == 511, "all post-panic retriggers must be delivered");
    } else if (std::strcmp(argv[1], "panic-dense-retrigger") == 0) {
        const RackVST3MidiEvent old_note{0, 0x90, 60, 100, 0};
        for (int i = 0; i < 1024; ++i) require(render(*plugin, &old_note, 1) == RACK_VST3_OK, "old voices");
        const RackVST3MidiEvent events[]{{0, 0xB0, 123, 0, 0}, {0, 0x90, 60, 127, 0}};
        require(render(*plugin, events, 2) == RACK_VST3_OK, "dense reset and retrigger");
        for (int i = 0; i < 8; ++i) require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "finish dense reset");
        require(processor.voice_count == 1 && processor.voices[0] == 1.0f, "reset must complete before retrigger reaches a LIFO processor");
    } else if (std::strcmp(argv[1], "panic-sustain") == 0) {
        plugin->midi_controller_assignments.assign(16 * kCountCtrlNumber, kNoParamId);
        plugin->midi_controller_assignments[64] = 42;
        plugin->input_param_changes.setMaxParameters(1);
        const RackVST3MidiEvent pedal{0, 0xB0, 64, 127, 0};
        const RackVST3MidiEvent notes_off{0, 0xB0, 123, 0, 0};
        require(render(*plugin, &pedal, 1) == RACK_VST3_OK, "pedal on");
        require(render(*plugin, &notes_off, 1) == RACK_VST3_OK, "ordinary All Notes Off");
        require(processor.last_parameter_value == 1.0, "ordinary All Notes Off must preserve pedal state");
    } else if (std::strcmp(argv[1], "panic-retrigger") == 0) {
        plugin->midi_controller_assignments.assign(16 * kCountCtrlNumber, kNoParamId);
        plugin->midi_controller_assignments[64] = 42;
        plugin->input_param_changes.setMaxParameters(1);
        const RackVST3MidiEvent note{0, 0x90, 60, 100, 0};
        require(render(*plugin, &note, 1) == RACK_VST3_OK, "initial note");
        const RackVST3MidiEvent events[]{{0, 0xB0, 123, 0, 0}, {0, 0x90, 60, 100, 0}, {0, 0xB0, 64, 127, 0}};
        require(render(*plugin, events, 3) == RACK_VST3_OK, "reset then retrigger");
        require(processor.notes[60] == 1, "post-reset retrigger must survive");
        require(processor.last_parameter_value == 1.0, "post-reset pedal-on must survive");
    } else if (std::strcmp(argv[1], "allocations") == 0) {
        plugin->midi_controller_assignments.assign(16 * kCountCtrlNumber, kNoParamId);
        plugin->midi_controller_assignments[1] = 42;
        plugin->input_param_changes.setMaxParameters(1);
        int32 index = 0;
        plugin->input_param_changes.addParameterData(42, index)->addPoint(0, 0.5, index);
        plugin->input_param_changes.clearQueue();
        std::array<RackVST3MidiEvent, 200> events{};
        for (uint32_t i = 0; i < events.size(); ++i) events[i] = {i * 240, 0xB0, 1, uint8_t(i % 128), 0};
        count_allocations = true;
        const int result = render(*plugin, events.data(), uint32_t(events.size()));
        count_allocations = false;
        std::printf("allocations for 200 mapped CC/s: %zu\n", allocations);
        require(result == RACK_VST3_OK, "CC processing");
        require(processor.parameter_points == 200, "retain all sample-accurate CC points");
        require(processor.last_parameter_value == double(199 % 128) / 127.0, "retain final CC value");
        require(allocations == 0, "mapped MIDI must not allocate in process");
    } else if (std::strcmp(argv[1], "parameters") == 0) {
        plugin->input_param_changes.setMaxParameters(1);
        for (int edit = 0; edit < 4096; ++edit) require(plugin->ui_to_audio_parameters.push({42, 0.5, 0}), "parameter fixture admission");
        require(render(*plugin, nullptr, 0) == RACK_VST3_OK, "parameter processing");
        require(plugin->ui_to_audio_parameters.read_index.load() == 512, "duplicate UI edits must count toward the callback work budget");
    } else if (std::strcmp(argv[1], "parameter-capacity") == 0) {
        auto& changes = plugin->input_param_changes;
        changes.setMaxParameters(1);
        int32 index = 0;
        auto* queue = changes.addParameterData(42, index);
        count_allocations = true;
        for (int32 point = 0; point < 4096; ++point) require(queue->addPoint(point, 0.5, index) == kResultTrue, "admit bounded points");
        require(queue->addPoint(4096, 0.75, index) == kResultFalse, "point capacity is a hard bound");
        require(changes.addParameterData(43, index) == nullptr, "parameter capacity is a hard bound");
        require(queue->addPoint(4095, 0.75, index) == kResultTrue, "existing point can still change at capacity");
        changes.clearQueue();
        queue = changes.addParameterData(43, index);
        require(queue && queue->getPointCount() == 0 && queue->addPoint(0, 1.0, index) == kResultTrue, "arena is reusable next block");
        count_allocations = false;
        require(allocations == 0, "capacity exhaustion and recycling must not allocate");
    } else {
        require(false, "unknown case");
    }
    std::puts("PASS");
}
