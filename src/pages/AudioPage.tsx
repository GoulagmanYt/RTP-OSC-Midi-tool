import { AudioConfigCard } from "./audio/AudioConfigCard";
import { VstParametersDialog } from "./audio/VstParametersDialog";
import { AudioStressTestCard } from "./audio/AudioStressTestCard";
import { useAudioPageController } from "./audio/useAudioPageController";

export default function AudioPage() {
  const controller = useAudioPageController();

  return (
    <div className="space-y-6">
      {controller.config?.ui.developerMode && (
        <AudioStressTestCard
          bridgeRunning={controller.bridgeRunning}
          audioRunning={controller.status?.audioRunning ?? false}
          t={controller.t}
        />
      )}
      <AudioConfigCard
        audioBackends={controller.audioBackends}
        audioDevices={controller.audioDevices}
        audioReloading={controller.audioReloading}
        bridgeRunning={controller.bridgeRunning}
        bufferMismatch={controller.bufferMismatch}
        canOpenSelectedVstUi={controller.canOpenSelectedVstUi}
        canOpenVstParameterFallback={controller.canOpenVstParameterFallback}
        config={controller.config}
        currentLatencyMs={controller.currentLatencyMs}
        isVst3={controller.isVst3}
        midiMessagesPerSec={controller.midiMessagesPerSec}
        selectedPluginKindLabel={controller.selectedPluginKindLabel}
        selectedPluginStatusLabel={controller.selectedPluginStatusLabel}
        selectedVstPlugin={controller.selectedVstPlugin}
        status={controller.status}
        streamBufferSize={controller.streamBufferSize}
        t={controller.t}
        vstMidiCompatible={controller.vstMidiCompatible}
        vstPlugins={controller.vstPlugins}
        vstPluginsLoading={controller.vstPluginsLoading}
        vstUiOpen={controller.vstUiOpen}
        activeBufferSize={controller.activeBufferSize}
        activeSampleRate={controller.activeSampleRate}
        requestedBufferSize={controller.requestedBufferSize}
        onBackendChange={controller.handleBackendChange}
        onCloseVstUi={controller.handleCloseVstUi}
        onGainChange={controller.handleGainChange}
        onLimiterToggle={controller.handleLimiterToggle}
        onOpenVstParameters={controller.handleOpenVstParameters}
        onOpenVstUi={controller.handleOpenVstUi}
        onPingAudio={controller.handlePingAudio}
        onRefreshVstPluginsList={controller.refreshVstPluginsList}
        onReloadVst={controller.handleReloadVst}
        onSaveConfig={controller.persistConfig}
        onSelectVst={controller.handleSelectVst}
        onToggleAudio={controller.handleAudioToggle}
        onUpdateAudioConfig={controller.updateAudioConfig}
      />
      <VstParametersDialog
        isVst3={controller.isVst3}
        open={controller.vstParameterDialogOpen}
        params={controller.vstParams}
        loading={controller.vstParamsLoading}
        t={controller.t}
        onOpenChange={(open) => {
          if (!open) controller.setVstParameterDialogOpen(false);
        }}
        onParamChange={controller.handleVstParamChange}
      />
    </div>
  );
}

