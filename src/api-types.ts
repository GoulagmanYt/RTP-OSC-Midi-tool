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

export type AvatarConfig = {
  path?: string | null;
  offsetX: number;
  offsetY: number;
  scale: number;
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
  sampleRate: number;
  bufferSize: number;
  gainDb: number;
  limiterEnabled: boolean;
  vstPath?: string | null;
};

export type UiConfig = {
  themePreset: string;
  theme: Theme;
  themePalette: string;
  accent: string;
  cornerRadius: number;
  scale: number;
  contentPadding: number;
  sidebarWidth: number;
  surfaceOpacity: number;
  cardOpacity: number;
  auroraIntensity: number;
  grainIntensity: number;
  reduceMotion: boolean;
  autoStart: boolean;
  alwaysOnTop: boolean;
  avatar: AvatarConfig;
};

export type LoggingConfig = {
  enabled: boolean;
  verbose: boolean;
  liveLogs: boolean;
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

export type Config = AppConfig;

export type RuntimeStatus = {
  running: boolean;
  midiInput?: string | null;
  midiOutput?: string | null;
  oscTarget: string;
  rtpActive: boolean;
  rtpBoundPort?: number | null;
  lastError?: string | null;
  vstLoaded?: boolean;
  audioRunning?: boolean;
  audioLatencyMs?: number | null;
  audioBackend?: string | null;
  audioDevice?: string | null;
  audioSampleRate?: number | null;
  audioBufferSize?: number | null;
  audioRequestedBufferSize?: number | null;
  audioStreamBufferSize?: number | null;
  audioBufferMismatch?: boolean | null;
  vstMidiCompatible?: boolean | null;
  audioXruns?: number | null;
  audioLimiterEnabled?: boolean | null;
};

export type BridgeStatus = RuntimeStatus;

export type RuntimeMetrics = {
  audioPeakL?: number | null;
  audioPeakR?: number | null;
  audioLatencyMs?: number | null;
  audioXruns?: number | null;
  audioMidiDrops?: number | null;
  audioLockMisses?: number | null;
  audioEmergencyResets?: number | null;
  midiMessagesPerSec: number;
  oscMessagesPerSec: number;
};

export type BridgeMetrics = RuntimeMetrics;

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

export type MidiActivitySnapshot = {
  source: string;
  messagesPerSec: number;
  lastNote?: number | null;
  lastChannel?: number | null;
  lastSeenMs?: number | null;
};

export type MidiActivityInfo = MidiActivitySnapshot;

export type MidiNoteEvent = {
  source: string;
  note: number;
  channel: number;
  pressed: boolean;
  timestampMs: number;
};

export type AppPaths = {
  configDir: string;
  logFile: string;
  logDir: string;
};

export type VstPluginEntry = {
  name: string;
  path: string;
  format: string;
  kind: string;
  architecture: string;
  supported: boolean;
  unsupportedReason?: string | null;
  midiCompatible?: boolean | null;
  hasEditor: boolean;
  channelLayout?: string | null;
};

export type VstParameter = {
  index: number;
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

export type LogEvent = LogEntry;

export type AudioSettings = AudioConfig;

export type StressTestMode = "audio-vst" | "bridge" | "rtp" | "end-to-end";

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

