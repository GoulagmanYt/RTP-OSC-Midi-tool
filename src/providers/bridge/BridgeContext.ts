import { createContext, useContext } from "react";
import type * as api from "../../api";

export interface BridgeContextType {
  config: api.AppConfig | null;
  status: api.RuntimeStatus | null;
  metrics: api.RuntimeMetrics | null;
  logs: api.LogEntry[];
  audioBackends: string[];
  audioDevices: string[];
  midiInputs: string[];
  midiOutputs: string[];
  isLoading: boolean;
  preflight: api.PreflightReport | null;
  refreshStatus: () => Promise<void>;
  updateConfig: (patch: api.DeepPartial<api.AppConfig>) => Promise<void>;
  saveConfig: () => Promise<void>;
  toggleBridge: () => Promise<void>;
  clearLogs: () => void;
  refreshAudioDevices: (backend?: string | null) => Promise<void>;
  refreshLists: () => Promise<void>;
  runPreflight: () => Promise<api.PreflightReport | null>;
  reloadConfig: () => Promise<void>;
}

export const BridgeContext = createContext<BridgeContextType | undefined>(undefined);

export function useBridge() {
  const context = useContext(BridgeContext);
  if (context === undefined) {
    throw new Error("useBridge must be used within a BridgeProvider");
  }
  return context;
}
