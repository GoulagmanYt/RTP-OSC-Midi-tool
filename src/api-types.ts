export type Theme = "light" | "dark";

export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends Array<infer U>
    ? Array<U>
    : T[K] extends object | undefined
    ? DeepPartial<NonNullable<T[K]>> | T[K]
    : T[K];
};

export type CommandError = {
  code: string;
  domain: string;
  message: string;
  details?: Record<string, string>;
};

export type RtpRemoteEntry = {
  id: string;
  name: string;
  host: string;
  port: number;
  autoConnect: boolean;
};

export type RoutingMapping = {
  from: number;
  to: number;
};

export type RoutingProfile = {
  id: string;
  name: string;
  enabled: boolean;
  channelFilter?: number | null;
  noteMin?: number | null;
  noteMax?: number | null;
  ccMap: RoutingMapping[];
  programMap: RoutingMapping[];
};

export type RoutingAssignment = {
  source: string;
  profileId: string;
};

export type MidiConfig = {
  inputDevice?: string | null;
  outputDevice?: string | null;
  channelFilter?: number | null;
  thruEnabled: boolean;
  hotplug: boolean;
  routingProfiles: RoutingProfile[];
  routingAssignments: RoutingAssignment[];
};

export type OscConfig = {
  enabled: boolean;
  targetIp: string;
  targetPort: number;
  logMessages: boolean;
};

export type RtpConfig = {
  enabled: boolean;
  sessionName: string;
  port: number;
  remoteEnabled: boolean;
  remotes: RtpRemoteEntry[];
  logMessages: boolean;
};

export type AudioConfig = {
  enabled: boolean;
  backend?: string | null;
  device?: string | null;
  deviceId?: string | null;
  sampleRate: number;
  bufferSize: number;
  gainDb: number;
  limiterEnabled: boolean;
  vstPluginId?: string | null;
  vstPath?: string | null;
  vstScanPaths: string[];
};

export type AudioDeviceEntry = {
  id?: string | null;
  name: string;
};

export type UiConfig = {
  theme: Theme;
  themePalette: string;
  cornerRadius: number;
  autoStart: boolean;
  developerMode: boolean;
};

export type LoggingConfig = {
  enabled: boolean;
  verbose: boolean;
  logAllToFile: boolean;
};

export type AppConfig = {
  version: number;
  midi: MidiConfig;
  osc: OscConfig;
  rtp: RtpConfig;
  audio: AudioConfig;
  ui: UiConfig;
  logging: LoggingConfig;
};

export type RuntimeStatus = {
  running: boolean;
  midiInput?: string | null;
  midiOutput?: string | null;
  oscTarget: string;
  rtpActive: boolean;
  rtpBoundPort?: number | null;
  rtpAdvertisedHost?: string | null;
  rtpAdvertisedAddresses: string[];
  rtpNetworkWarning?: string | null;
  lastError?: string | null;
  vstLoaded?: boolean;
  audioRunning?: boolean;
  audioLatencyMs?: number | null;
  audioBufferPeriodMs?: number | null;
  audioBackendLatencyMs?: number | null;
  pluginLatencySamples?: number | null;
  bridgeLatencySamples?: number | null;
  vstHostingMode?: "directX64" | "bridgedX86" | null;
  audioBackend?: string | null;
  audioDevice?: string | null;
  audioDeviceId?: string | null;
  audioSampleRate?: number | null;
  audioBufferSize?: number | null;
  audioRequestedBufferSize?: number | null;
  audioStreamBufferSize?: number | null;
  audioBufferMismatch?: boolean | null;
  vstMidiCompatible?: boolean | null;
  audioXruns?: number | null;
  audioStreamRecoveryRequests?: number | null;
  audioStreamRouteChanges?: number | null;
  audioMidiDrops?: number | null;
  audioLockMisses?: number | null;
  audioEmergencyResets?: number | null;
  audioCallbackMaxUs?: number | null;
  audioCallbackLastUs?: number | null;
  audioCallbackOverBudgetCount?: number | null;
  consecutiveDeadlineMisses?: number | null;
  dspProcessLastUs?: number | null;
  dspProcessP95Us?: number | null;
  dspProcessP99Us?: number | null;
  dspProcessMaxUs?: number | null;
  audioMidiQueueDepth?: number | null;
  audioMidiQueueMaxDepth?: number | null;
  audioMidiOldestUs?: number | null;
  audioLifecycleState?: string;
  vstWorkerState?: string;
  vstWorkerRestarts?: number;
  vstWorkerLastExit?: string | null;
  audioMmcssEnabled?: boolean | null;
  audioPowerThrottlingDisabled?: boolean | null;
  audioLimiterEnabled?: boolean | null;
  x86BridgeUnderruns?: number | null;
  x86BridgeOverruns?: number | null;
  x86WorkerAlive?: boolean | null;
};

export type RuntimeMetrics = {
  audioPeakL?: number | null;
  audioPeakR?: number | null;
  audioLatencyMs?: number | null;
  audioBufferPeriodMs?: number | null;
  audioBackendLatencyMs?: number | null;
  pluginLatencySamples?: number | null;
  bridgeLatencySamples?: number | null;
  vstHostingMode?: "directX64" | "bridgedX86" | null;
  audioXruns?: number | null;
  audioStreamRecoveryRequests?: number | null;
  audioStreamRouteChanges?: number | null;
  audioMidiDrops?: number | null;
  audioLockMisses?: number | null;
  audioEmergencyResets?: number | null;
  audioCallbackMaxUs?: number | null;
  audioCallbackLastUs?: number | null;
  audioCallbackOverBudgetCount?: number | null;
  consecutiveDeadlineMisses?: number | null;
  dspProcessLastUs?: number | null;
  dspProcessP95Us?: number | null;
  dspProcessP99Us?: number | null;
  dspProcessMaxUs?: number | null;
  audioMidiQueueDepth?: number | null;
  audioMidiQueueMaxDepth?: number | null;
  audioMidiOldestUs?: number | null;
  vstWorkerState?: string;
  vstWorkerRestarts?: number;
  vstWorkerLastExit?: string | null;
  audioMmcssEnabled?: boolean | null;
  audioPowerThrottlingDisabled?: boolean | null;
  x86BridgeUnderruns?: number | null;
  x86BridgeOverruns?: number | null;
  x86WorkerAlive?: boolean | null;
  midiMessagesPerSec: number;
  oscMessagesPerSec: number;
  bridgeQueueDepth: number;
  bridgeQueueMaxDepth: number;
  bridgeMessagesIn: number;
  bridgeMessagesOut: number;
  bridgeMessagesDropped: number;
  rtpMidiDrops: number;
  reliablePlaybackMessagesIn: number;
  reliablePlaybackMessagesOut: number;
  reliablePlaybackDropped: number;
  reliablePlaybackMaxLateUs: number;
  reliablePlaybackActiveSession?: string | null;
};

export type PreflightReport = {
  midiInOk: boolean;
  midiOutOk: boolean;
  audioBackendOk: boolean;
  rtpPortOk: boolean;
  messages: string[];
};

export type RtpSessionInfo = {
  name: string;
  host: string;
  port: number;
  addresses: string[];
};

export type RtpParticipantInfo = {
  name: string;
  addr: string;
};

export type MidiActivityInfo = {
  source: string;
  messagesPerSec: number;
  lastNote?: number | null;
  lastChannel?: number | null;
  lastSeenMs?: number | null;
};

export type AppPaths = {
  configDir: string;
  logFile: string;
  logDir: string;
};

export type VstPluginEntry = {
  id: string;
  name: string;
  path: string;
  format: string;
  kind: string;
  architecture: string;
  availableArchitectures: string[];
  vendor?: string | null;
  pluginVersion?: string | null;
  status: "probing" | "compatible" | "unverified" | "failed" | "quarantined" | "unsupported" | "outOfScope";
  supported: boolean;
  unsupportedReason?: string | null;
  failureStage?: string | null;
  lastError?: string | null;
  lastProbedMs?: number | null;
  midiCompatible?: boolean | null;
  hasEditor: boolean;
  channelLayout?: string | null;
  classUid?: string | null;
  subPluginId?: number | null;
  hostingMode?: "directX64" | "bridgedX86" | null;
  fileModifiedMs?: number | null;
  fileSize?: number | null;
  hostAbiVersion?: number;
};

export type VstParameter = {
  index: number;
  id: number;
  flags: number;
  stepCount: number;
  unitId: number;
  name: string;
  min: number;
  max: number;
  default: number;
  unit: string;
  value: number;
};

export type LogEntry = {
  level: "info" | "warn" | "error" | "debug";
  message: string;
  timestamp: string;
};

export type StressTestMode = "audio-vst" | "bridge";

export interface SegmentMetrics {
  dropped: number;
  xruns: number;
  latencyMs: number | null;
}

export interface StressTestResult {
  sentNotes: number;
  elapsedMs: number;
  // Global totals
  droppedNotes: number;
  xruns: number;
  // Per-segment metrics (optional based on test mode)
  segmentRtp?: SegmentMetrics;
  segmentBridge?: SegmentMetrics;
  segmentAudio?: SegmentMetrics;
  // End-to-end tracking
  receivedNotes?: number;
}
