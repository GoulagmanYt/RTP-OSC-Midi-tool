import { useEffect, useState, useCallback, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { refreshRtpSessions } from "../api";
import type { RtpParticipantInfo, RtpSessionInfo } from "../api";
import { RTP_VIRTUAL_INPUT } from "../constants";
import { useBridge } from "../providers/BridgeProvider";
import { useI18n } from "../providers/LanguageProvider";
import { RtpAboutCard } from "./rtp/RtpAboutCard";
import { RtpConfigCard } from "./rtp/RtpConfigCard";
import { RtpParticipantsCard } from "./rtp/RtpParticipantsCard";
import { RtpStatusCard } from "./rtp/RtpStatusCard";
import {
  buildRemoteConnectionChecker,
  isSelectedSession,
  mergeSessionIntoRemotes,
  syncAutoConnectRemotes,
} from "./rtp/utils";

export default function RtpPage() {
  const { config, updateConfig, saveConfig, status } = useBridge();
  const [busy, setBusy] = useState(false);
  const [discovering, setDiscovering] = useState(false);
  const [sessions, setSessions] = useState<RtpSessionInfo[]>([]);
  const [participants, setParticipants] = useState<RtpParticipantInfo[]>([]);
  const { t } = useI18n();
  const remoteEnabled = config?.rtp.remoteEnabled ?? false;
  const remotes = config?.rtp.remotes ?? [];
  const remotesRef = useRef(remotes);
  remotesRef.current = remotes;

  const refreshSessions = useCallback(async () => {
    setDiscovering(true);
    try {
      await refreshRtpSessions();
    } catch (e) {
      console.error("Failed to list RTP sessions", e);
      setDiscovering(false);
    }
  }, []);
  
  useEffect(() => {
    const unlistenSessions = listen<RtpSessionInfo[]>("rtp:sessions", (event) => {
      setSessions(event.payload);
      setDiscovering(false);
    });
    const unlistenParticipants = listen<RtpParticipantInfo[]>("rtp:participants", (event) => {
      setParticipants(event.payload);
    });
    return () => {
      unlistenSessions.then((fn) => fn());
      unlistenParticipants.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (remoteEnabled) {
      refreshSessions();
    } else {
      setSessions([]);
      setDiscovering(false);
    }
  }, [remoteEnabled, refreshSessions]);

  useEffect(() => {
    if (!status?.rtpActive && !remoteEnabled && config?.midi.inputDevice !== RTP_VIRTUAL_INPUT) {
      setParticipants([]);
    }
  }, [status?.rtpActive, remoteEnabled, config?.midi.inputDevice]);

  useEffect(() => {
    if (!remoteEnabled || sessions.length === 0) return;
    const currentRemotes = remotesRef.current;
    if (currentRemotes.length === 0) return;
    const result = syncAutoConnectRemotes(currentRemotes, sessions);
    if (result.changed) {
      updateConfig({ rtp: { remotes: result.remotes } });
    }
  }, [remoteEnabled, sessions, updateConfig]);

  const handleSave = async () => {
    setBusy(true);
    try {
      await saveConfig();
    } finally {
      setBusy(false);
    }
  };

  const handleToggleRtp = async (checked: boolean) => {
    await updateConfig({ rtp: { enabled: checked } });
  };

  const makeRemoteId = () =>
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `rtp-${Date.now()}-${Math.random().toString(16).slice(2)}`;

  const updateRemote = async (id: string, patch: Partial<(typeof remotes)[number]>) => {
    const next = remotes.map((remote) => (remote.id === id ? { ...remote, ...patch } : remote));
    await updateConfig({ rtp: { remotes: next } });
  };

  const addRemote = async () => {
    const next = [
      ...remotes,
      {
        id: makeRemoteId(),
        name: "",
        host: "",
        port: 5004,
        autoConnect: true,
      },
    ];
    await updateConfig({ rtp: { remotes: next } });
  };

  const removeRemote = async (id: string) => {
    await updateConfig({ rtp: { remotes: remotes.filter((remote) => remote.id !== id) } });
  };

  const handleConnect = async (session: RtpSessionInfo) => {
    const next = mergeSessionIntoRemotes(remotes, session, makeRemoteId);
    await updateConfig({ rtp: { remoteEnabled: true, remotes: next } });
  };

  const isRemoteConnected = buildRemoteConnectionChecker(participants);

  return (
    <div className="space-y-6">
      <RtpConfigCard
        busy={busy}
        config={config}
        discovering={discovering}
        remoteEnabled={remoteEnabled}
        remotes={remotes}
        sessions={sessions}
        t={t}
        isRemoteConnected={isRemoteConnected}
        isSelectedSession={(session) => isSelectedSession(remotes, session)}
        onAddRemote={addRemote}
        onConnect={handleConnect}
        onRefreshSessions={refreshSessions}
        onRemoveRemote={removeRemote}
        onSave={handleSave}
        onToggleRtp={handleToggleRtp}
        onUpdateConfig={updateConfig}
        onUpdateRemote={updateRemote}
      />
      <RtpParticipantsCard participants={participants} t={t} />
      <RtpStatusCard config={config} status={status} t={t} />
      <RtpAboutCard t={t} />
    </div>
  );
}
