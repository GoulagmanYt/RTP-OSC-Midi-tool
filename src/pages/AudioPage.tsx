import { AudioConfigCard } from "./audio/AudioConfigCard";
import { VstParametersDialog } from "./audio/VstParametersDialog";
import { AudioStressTestCard } from "./audio/AudioStressTestCard";
import { useAudioPageController } from "./audio/useAudioPageController";
import { VstEqDialog } from "./audio/VstEqDialog";

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
        canOpenVstEq={Boolean(controller.config)}
        config={controller.config}
        currentLatencyMs={controller.currentLatencyMs}
        midiMessagesPerSec={controller.midiMessagesPerSec}
        selectedPluginKindLabel={controller.selectedPluginKindLabel}
        selectedPluginStatusLabel={controller.selectedPluginStatusLabel}
        selectedVstPlugin={controller.selectedVstPlugin}
        vstEqEnabled={controller.selectedVstEq.enabled}
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
        onOpenVstEq={() => controller.setVstEqDialogOpen(true)}
        onOpenVstUi={controller.handleOpenVstUi}
        onPingAudio={controller.handlePingAudio}
        onRefreshVstPluginsList={controller.refreshVstPluginsList}
        onReloadVst={controller.handleReloadVst}
        onSaveConfig={controller.persistConfig}
        onSelectVst={controller.handleSelectVst}
        onRetestVst={controller.handleRetestVst}
        onOpenVstFolder={controller.handleOpenVstFolder}
        onToggleAudio={controller.handleAudioToggle}
        onUpdateAudioConfig={controller.updateAudioConfig}
      />
      <VstParametersDialog
        open={controller.vstParameterDialogOpen}
        params={controller.vstParams}
        loading={controller.vstParamsLoading}
        t={controller.t}
        onOpenChange={(open) => {
          if (!open) controller.setVstParameterDialogOpen(false);
        }}
        onParamChange={controller.handleVstParamChange}
      />
      <VstEqDialog
        open={controller.vstEqDialogOpen}
        pluginName={controller.selectedVstEqName}
        settings={controller.selectedVstEq}
        sampleRate={controller.activeSampleRate ?? 48_000}
        t={controller.t}
        onOpenChange={controller.setVstEqDialogOpen}
        onChange={controller.handleVstEqChange}
        onReset={controller.handleVstEqReset}
      />
    </div>
  );
}

