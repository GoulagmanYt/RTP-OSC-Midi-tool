// Independent diagnostic: no rack, CPAL, worker IPC or production host code.
#define NOMINMAX
#include <juce_audio_utils/juce_audio_utils.h>
#include <windows.h>
#include <psapi.h>
#include <atomic>
#include <chrono>
#include <fstream>
#include <iostream>

struct Clock : juce::AudioPlayHead {
    int64_t samples = 0;
    juce::Optional<PositionInfo> getPosition() const override {
        PositionInfo p;
        p.setTimeInSamples(samples);
        p.setTimeInSeconds(double(samples) / 48000.0);
        p.setPpqPosition(double(samples) / 24000.0);
        p.setBpm(120.0);
        p.setTimeSignature(TimeSignature{4, 4});
        p.setIsPlaying(true);
        return p;
    }
};

struct Render : juce::AudioIODeviceCallback {
    juce::AudioPluginInstance& plugin;
    juce::AudioBuffer<float> buffer;
    juce::MidiBuffer midi;
    Clock clock;
    std::atomic<uint64_t> on{0}, off{0}, callbacks{0}, overBudget{0};
    std::atomic<float> peak{0};
    std::atomic<bool> badBlock{false}, sending{true};
    bool held = false;
    explicit Render(juce::AudioPluginInstance& p) : plugin(p), buffer(2, 512) {
        midi.ensureSize(8192);
        plugin.setPlayHead(&clock);
    }
    ~Render() override { plugin.setPlayHead(nullptr); }
    void audioDeviceAboutToStart(juce::AudioIODevice* d) override {
        if (d->getCurrentBufferSizeSamples() != 512 || d->getCurrentSampleRate() != 48000.0)
            badBlock.store(true);
    }
    void audioDeviceStopped() override {}
    void audioDeviceIOCallbackWithContext(const float* const*, int, float* const* outputs,
                                          int outputCount, int frames,
                                          const juce::AudioIODeviceCallbackContext&) override {
        auto start = std::chrono::steady_clock::now();
        for (int ch = 0; ch < outputCount; ++ch)
            if (outputs[ch]) juce::FloatVectorOperations::clear(outputs[ch], frames);
        if (frames != 512 || badBlock.load()) { badBlock.store(true); return; }
        buffer.clear();
        midi.clear();
        for (int i = 0; i < frames; ++i) {
            const auto phase = (clock.samples + i) % 1536;
            if (phase == 0 && sending.load()) {
                for (int n = 48; n < 64; ++n) midi.addEvent(juce::MidiMessage::noteOn(1, n, (juce::uint8)80), i);
                on.fetch_add(16); held = true;
            } else if (held && (phase == 768 || !sending.load())) {
                for (int n = 48; n < 64; ++n) midi.addEvent(juce::MidiMessage::noteOff(1, n), i);
                off.fetch_add(16); held = false;
            }
        }
        plugin.processBlock(buffer, midi);
        float blockPeak = 0;
        for (int ch = 0; ch < 2; ++ch) {
            blockPeak = std::max(blockPeak, buffer.getMagnitude(ch, 0, frames));
            if (ch < outputCount && outputs[ch])
                for (int i = 0; i < frames; ++i)
                    outputs[ch][i] = juce::jlimit(-0.95f, 0.95f, buffer.getSample(ch, i) * 0.063095734f);
        }
        peak.store(std::max(peak.load(), blockPeak));
        clock.samples += frames;
        callbacks.fetch_add(1);
        if (std::chrono::steady_clock::now() - start > std::chrono::microseconds(10667)) overBudget.fetch_add(1);
    }
};

int main(int argc, char** argv) {
    if (argc != 5 && argc != 6) {
        std::cerr << "Usage: juce_reference <vst3-path> <seconds> <memory-cap-MiB> <new-report.jsonl> [rack-native-state]\n";
        return 2;
    }
    const int seconds = juce::String(argv[2]).getIntValue(), cap = juce::String(argv[3]).getIntValue();
    if (seconds < 1 || seconds > 43200 || cap < 1) return 2;
    if (juce::File(argv[4]).exists()) { std::cerr << "Report already exists\n"; return 2; }
    std::ofstream report(argv[4]);
    if (!report) return 2;
    juce::ScopedJuceInitialiser_GUI gui;
    juce::VST3PluginFormat format;
    juce::OwnedArray<juce::PluginDescription> descriptions;
    format.findAllTypesForFile(descriptions, argv[1]);
    if (descriptions.size() != 1) { std::cerr << "Expected exactly one VST3 instrument\n"; return 2; }
    juce::String error;
    auto plugin = format.createInstanceFromDescription(*descriptions[0], 48000.0, 512, error);
    if (!plugin) { std::cerr << error << '\n'; return 2; }
    plugin->setPlayConfigDetails(0, 2, 48000.0, 512);
    plugin->setRateAndBufferSizeDetails(48000.0, 512);
    plugin->prepareToPlay(48000.0, 512);
    if (argc == 6) {
        juce::MemoryBlock state;
        if (!juce::File(argv[5]).loadFileAsData(state) || state.getSize() < 4) return 2;
        const auto componentSize = juce::ByteOrder::littleEndianInt(state.getData());
        if (componentSize > state.getSize() - 4) return 2;
        const auto* data = static_cast<const char*>(state.getData()) + 4;
        juce::XmlElement xml("VST3PluginState");
        xml.createNewChildElement("IComponent")->addTextElement(juce::MemoryBlock(data, componentSize).toBase64Encoding());
        const auto controllerSize = state.getSize() - 4 - componentSize;
        if (controllerSize != 0)
            xml.createNewChildElement("IEditController")->addTextElement(juce::MemoryBlock(data + componentSize, controllerSize).toBase64Encoding());
        juce::MemoryBlock wrapped;
        juce::AudioProcessor::copyXmlToBinary(xml, wrapped);
        plugin->setStateInformation(wrapped.getData(), static_cast<int>(wrapped.getSize()));
        juce::MemoryBlock readBack;
        plugin->getStateInformation(readBack);
        // Keep the round-trip state beside the private input, not in public reports.
        if (!juce::File(juce::String(argv[5]) + ".juce-roundtrip").replaceWithData(readBack.getData(), readBack.getSize())) return 2;
        if (auto saved = juce::AudioProcessor::getXmlFromBinary(readBack.getData(), static_cast<int>(readBack.getSize()))) {
            if (auto* component = saved->getChildByName("IComponent")) {
                juce::MemoryBlock native;
                if (!native.fromBase64Encoding(component->getAllSubText())) return 2;
                if (!juce::File(juce::String(argv[5]) + ".juce-component").replaceWithData(native.getData(), native.getSize())) return 2;
            } else return 2;
        } else return 2;
    }
    Render render(*plugin);
    juce::AudioDeviceManager device;
    device.initialise(0, 2, nullptr, false, {}, nullptr);
    device.setCurrentAudioDeviceType("ASIO", true);
    juce::AudioDeviceManager::AudioDeviceSetup setup;
    device.getAudioDeviceSetup(setup);
    setup.outputDeviceName = "Voicemeeter AUX Virtual ASIO";
    setup.inputDeviceName = {};
    setup.sampleRate = 48000;
    setup.bufferSize = 512;
    error = device.setAudioDeviceSetup(setup, true);
    if (error.isNotEmpty() || device.getCurrentAudioDeviceType() != "ASIO") {
        std::cerr << "ASIO setup failed: " << error << '\n'; plugin->releaseResources(); return 2;
    }
    device.addAudioCallback(&render);
    const auto start = std::chrono::steady_clock::now();
    int previous = -1;
    bool memoryExceeded = false, externalStop = false;
    for (;;) {
        juce::MessageManager::getInstance()->runDispatchLoopUntil(50);
        const auto elapsed = std::chrono::duration_cast<std::chrono::seconds>(std::chrono::steady_clock::now() - start).count();
        if (elapsed != previous) {
            previous = int(elapsed);
            PROCESS_MEMORY_COUNTERS_EX memory{};
            if (!GetProcessMemoryInfo(GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&memory), sizeof(memory))) return 2;
            report << "{\"seconds\":" << elapsed << ",\"privateBytes\":" << memory.PrivateUsage
                   << ",\"noteOns\":" << render.on.load() << ",\"noteOffs\":" << render.off.load()
                   << ",\"peak\":" << render.peak.load() << ",\"overBudget\":" << render.overBudget.load() << "}\n" << std::flush;
            memoryExceeded = memory.PrivateUsage > uint64_t(cap) * 1024 * 1024;
        }
        externalStop = juce::File(juce::String(argv[4]) + ".stop").existsAsFile();
        if (elapsed >= seconds || memoryExceeded || render.badBlock.load() || externalStop) break;
    }
    render.sending.store(false);
    for (int i = 0; i < 60; ++i) juce::MessageManager::getInstance()->runDispatchLoopUntil(50);
    device.removeAudioCallback(&render);
    device.closeAudioDevice();
    plugin->releaseResources();
    const bool completed = !externalStop && !memoryExceeded && !render.badBlock.load() && render.on.load() == render.off.load() && render.peak.load() > 0.00001f;
    report << "{\"completed\":" << (completed ? "true" : "false")
           << ",\"externalStop\":" << (externalStop ? "true" : "false")
           << ",\"memoryExceeded\":" << (memoryExceeded ? "true" : "false")
           << ",\"badBlock\":" << (render.badBlock.load() ? "true" : "false")
           << ",\"noteOns\":" << render.on.load() << ",\"noteOffs\":" << render.off.load()
           << ",\"peak\":" << render.peak.load() << ",\"overBudget\":" << render.overBudget.load() << "}\n";
    return completed ? 0 : 1;
}
