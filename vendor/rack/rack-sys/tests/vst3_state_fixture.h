// Minimal ABI fixtures for exercising the real host persistence boundary.
class StateComponent final : public IComponent {
public:
    bool seek_on_save = false;
    std::array<uint8_t, 3> state{0x11, 0x22, 0x33};
    tresult PLUGIN_API queryInterface(const TUID, void** obj) override { *obj = nullptr; return kNoInterface; }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }
    tresult PLUGIN_API initialize(FUnknown*) override { return kResultOk; }
    tresult PLUGIN_API terminate() override { return kResultOk; }
    tresult PLUGIN_API getControllerClassId(TUID) override { return kResultFalse; }
    tresult PLUGIN_API setIoMode(IoMode) override { return kResultOk; }
    int32 PLUGIN_API getBusCount(MediaType, BusDirection) override { return 0; }
    tresult PLUGIN_API getBusInfo(MediaType, BusDirection, int32, BusInfo&) override { return kResultFalse; }
    tresult PLUGIN_API getRoutingInfo(RoutingInfo&, RoutingInfo&) override { return kNotImplemented; }
    tresult PLUGIN_API activateBus(MediaType, BusDirection, int32, TBool) override { return kResultOk; }
    tresult PLUGIN_API setActive(TBool) override { return kResultOk; }
    tresult PLUGIN_API getState(IBStream* stream) override {
        if (seek_on_save) stream->seek(0, IBStream::kIBSeekSet, nullptr);
        return stream->write(state.data(), static_cast<int32>(state.size()), nullptr);
    }
    tresult PLUGIN_API setState(IBStream* stream) override {
        return stream->read(state.data(), static_cast<int32>(state.size()), nullptr);
    }
};

class StateController final : public IEditController {
public:
    bool seek_on_save = false;
    std::array<uint8_t, 2> state{0x44, 0x55};
    std::array<uint8_t, 3> component_state{};
    tresult PLUGIN_API queryInterface(const TUID, void** obj) override { *obj = nullptr; return kNoInterface; }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }
    tresult PLUGIN_API initialize(FUnknown*) override { return kResultOk; }
    tresult PLUGIN_API terminate() override { return kResultOk; }
    tresult PLUGIN_API getState(IBStream* stream) override {
        if (seek_on_save) stream->seek(0, IBStream::kIBSeekSet, nullptr);
        return stream->write(state.data(), static_cast<int32>(state.size()), nullptr);
    }
    tresult PLUGIN_API setState(IBStream* stream) override {
        return stream->read(state.data(), static_cast<int32>(state.size()), nullptr);
    }
    tresult PLUGIN_API setComponentState(IBStream* stream) override {
        return stream->read(component_state.data(), static_cast<int32>(component_state.size()), nullptr);
    }
    int32 PLUGIN_API getParameterCount() override { return 0; }
    tresult PLUGIN_API getParameterInfo(int32, Vst::ParameterInfo&) override { return kResultFalse; }
    tresult PLUGIN_API getParamStringByValue(ParamID, ParamValue, String128) override { return kNotImplemented; }
    tresult PLUGIN_API getParamValueByString(ParamID, TChar*, ParamValue&) override { return kNotImplemented; }
    ParamValue PLUGIN_API normalizedParamToPlain(ParamID, ParamValue value) override { return value; }
    ParamValue PLUGIN_API plainParamToNormalized(ParamID, ParamValue value) override { return value; }
    ParamValue PLUGIN_API getParamNormalized(ParamID) override { return 0; }
    tresult PLUGIN_API setParamNormalized(ParamID, ParamValue) override { return kResultOk; }
    tresult PLUGIN_API setComponentHandler(IComponentHandler*) override { return kResultOk; }
    IPlugView* PLUGIN_API createView(FIDString) override { return nullptr; }
};

static void test_native_state_streams(bool component_seeks, bool controller_seeks) {
    StateComponent component;
    StateController controller;
    component.seek_on_save = component_seeks;
    controller.seek_on_save = controller_seeks;
    RackVST3Plugin plugin;
    plugin.component = &component;
    plugin.controller = &controller;
    plugin.separate_controller = true;

    require(rack_vst3_plugin_get_state_size(&plugin) == 9, "native state size must include independent component and controller payloads");
    std::array<uint8_t, 9> bytes{};
    size_t size = bytes.size();
    require(rack_vst3_plugin_get_state(&plugin, bytes.data(), &size) == RACK_VST3_OK, "native state capture");
    require(bytes == std::array<uint8_t, 9>{3, 0, 0, 0, 0x11, 0x22, 0x33, 0x44, 0x55}, "seeking within a plugin state stream must not overwrite host framing or another state");
    component.state.fill(0);
    controller.state.fill(0);
    require(rack_vst3_plugin_set_state(&plugin, bytes.data(), size) == RACK_VST3_OK, "native state restore");
    require(component.state == std::array<uint8_t, 3>{0x11, 0x22, 0x33}, "component state round trip");
    require(controller.state == std::array<uint8_t, 2>{0x44, 0x55}, "controller state round trip");
    require(controller.component_state == component.state, "controller receives the restored component state");
}
