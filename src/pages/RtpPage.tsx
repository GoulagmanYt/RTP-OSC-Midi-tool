import { useEffect, useState, useCallback, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { refreshRtpSessions, RtpParticipantInfo, RtpSessionInfo } from "../api";
import { RTP_VIRTUAL_INPUT } from "../constants";
import { useBridge } from "../providers/BridgeProvider";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "../components/ui/Card";
import { Label } from "../components/ui/Label";
import { Input } from "../components/ui/Input";
import { Switch } from "../components/ui/Switch";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Wifi, Server, RefreshCcw, Plus, Trash2 } from "lucide-react";
import { useI18n } from "../providers/LanguageProvider";

export default function RtpPage() {
  const { config, updateConfig, saveConfig, status } = useBridge();
  const [busy, setBusy] = useState(false);
  const [discovering, setDiscovering] = useState(false);
  const [sessions, setSessions] = useState<RtpSessionInfo[]>([]);
  const [participants, setParticipants] = useState<RtpParticipantInfo[]>([]);
  const { t } = useI18n();
  const remoteEnabled = config?.rtpRemoteEnabled ?? false;
  const remotes = config?.rtpRemotes ?? [];
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
    const unlistenSessions = listen<RtpSessionInfo[]>("rtp_sessions", (event) => {
      setSessions(event.payload);
      setDiscovering(false);
    });
    const unlistenParticipants = listen<RtpParticipantInfo[]>("rtp_participants", (event) => {
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
  }, [remoteEnabled]);

  useEffect(() => {
    if (!status?.rtpActive && !remoteEnabled && config?.midiIn !== RTP_VIRTUAL_INPUT) {
      setParticipants([]);
    }
  }, [status?.rtpActive, remoteEnabled, config?.midiIn]);

  useEffect(() => {
    if (!remoteEnabled || sessions.length === 0) return;
    const currentRemotes = remotesRef.current;
    if (currentRemotes.length === 0) return;
    let changed = false;
    const next = currentRemotes.map((remote) => {
      if (!remote.autoConnect || !remote.name) return remote;
      const match = sessions.find(
        (session) => session.name.toLowerCase() === remote.name.toLowerCase()
      );
      if (!match) return remote;
      const host = match.addresses[0] ?? match.host;
      if (remote.host !== host || remote.port !== match.port) {
        changed = true;
        return { ...remote, host, port: match.port };
      }
      return remote;
    });
    if (changed) {
      updateConfig({ rtpRemotes: next });
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
    await updateConfig({ rtpEnabled: checked });
  };

  const makeRemoteId = () =>
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `rtp-${Date.now()}-${Math.random().toString(16).slice(2)}`;

  const updateRemote = async (id: string, patch: Partial<(typeof remotes)[number]>) => {
    const next = remotes.map((remote) => (remote.id === id ? { ...remote, ...patch } : remote));
    await updateConfig({ rtpRemotes: next });
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
    await updateConfig({ rtpRemotes: next });
  };

  const removeRemote = async (id: string) => {
    await updateConfig({ rtpRemotes: remotes.filter((remote) => remote.id !== id) });
  };

  const handleConnect = async (session: RtpSessionInfo) => {
    const host = session.addresses[0] ?? session.host;
    const existing =
      remotes.find((remote) => remote.name === session.name) ??
      remotes.find((remote) => remote.host === host && remote.port === session.port);
    const next = existing
      ? remotes.map((remote) =>
          remote.id === existing.id
            ? { ...remote, name: session.name, host, port: session.port, autoConnect: true }
            : remote
        )
      : [
          ...remotes,
          {
            id: makeRemoteId(),
            name: session.name,
            host,
            port: session.port,
            autoConnect: true,
          },
        ];
    await updateConfig({ rtpRemoteEnabled: true, rtpRemotes: next });
  };

  const isSelectedSession = (session: RtpSessionInfo) =>
    remotes.some((remote) => {
      if (remote.port !== session.port) return false;
      if (remote.name && remote.name === session.name) return true;
      if (remote.host === session.host) return true;
      return session.addresses.includes(remote.host);
    });

  const connectedAddrs = new Set(participants.map((participant) => participant.addr));
  const isRemoteConnected = (remote: (typeof remotes)[number]) => {
    const addr = `${remote.host}:${remote.port}`;
    if (connectedAddrs.has(addr)) return true;
    if (!remote.name) return false;
    return participants.some((participant) => participant.name === remote.name);
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Wifi className="h-5 w-5 text-blue-500" />
            {t("rtp.title")}
          </CardTitle>
          <CardDescription>{t("rtp.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="flex items-center justify-between p-4 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
            <div className="flex flex-col gap-1">
              <Label className="font-medium">{t("rtp.enable")}</Label>
              <p className="text-xs text-muted-foreground">{t("rtp.enableHint")}</p>
            </div>
            <Switch checked={config?.rtpEnabled ?? false} onCheckedChange={handleToggleRtp} />
          </div>

          <div className="space-y-4 p-4 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
            <h3 className="font-semibold text-sm">{t("rtp.sessionConfig")}</h3>
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="rtp-session" className="text-sm">
                  {t("rtp.sessionName")}
                </Label>
                <Input
                  id="rtp-session"
                  type="text"
                  placeholder="OSCMidi"
                  value={config?.rtpSessionName ?? ""}
                  onChange={(e) => updateConfig({ rtpSessionName: e.target.value })}
                />
                <p className="text-xs text-muted-foreground">{t("rtp.sessionNameHint")}</p>
              </div>

              <div className="space-y-2">
                <Label htmlFor="rtp-port" className="text-sm">
                  {t("rtp.portLabel")}
                </Label>
                <Input
                  id="rtp-port"
                  type="number"
                  placeholder="5004"
                  value={config?.rtpPort ?? ""}
                  onChange={(e) => updateConfig({ rtpPort: Number(e.target.value || 0) })}
                />
                <p className="text-xs text-muted-foreground">{t("rtp.portHint")}</p>
              </div>
            </div>
          </div>

          <div className="space-y-4 p-4 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
            <h3 className="font-semibold text-sm">{t("rtp.remoteConfig")}</h3>
            <div className="flex items-center justify-between">
              <div className="flex flex-col gap-1">
                <Label className="font-medium">{t("rtp.remoteEnable")}</Label>
                <p className="text-xs text-muted-foreground">{t("rtp.remoteEnableHint")}</p>
              </div>
              <Switch
                checked={remoteEnabled}
                onCheckedChange={(checked) => updateConfig({ rtpRemoteEnabled: checked })}
              />
            </div>
            {remoteEnabled ? (
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <div>
                    <Label className="text-sm">{t("rtp.favoritesTitle")}</Label>
                    <p className="text-xs text-muted-foreground">{t("rtp.favoritesHint")}</p>
                  </div>
                  <Button variant="outline" size="sm" onClick={addRemote}>
                    <Plus className="h-3.5 w-3.5 mr-2" />
                    {t("rtp.addFavorite")}
                  </Button>
                </div>
                {remotes.length === 0 ? (
                  <div className="text-xs text-muted-foreground">{t("rtp.favoritesEmpty")}</div>
                ) : (
                  <div className="space-y-3">
                    {remotes.map((remote) => (
                      <div
                        key={remote.id}
                        className="rounded-md border border-slate-100 dark:border-slate-800 bg-white/70 dark:bg-slate-950/40 p-3 space-y-3"
                      >
                        <div className="grid gap-3 md:grid-cols-3">
                          <div className="space-y-1">
                            <Label className="text-xs">{t("rtp.favoriteName")}</Label>
                            <Input
                              type="text"
                              placeholder="pianoledvisualizer"
                              value={remote.name}
                              onChange={(e) => updateRemote(remote.id, { name: e.target.value })}
                            />
                          </div>
                          <div className="space-y-1">
                            <Label className="text-xs">{t("rtp.favoriteHost")}</Label>
                            <Input
                              type="text"
                              placeholder="192.168.0.50"
                              value={remote.host}
                              onChange={(e) => updateRemote(remote.id, { host: e.target.value })}
                            />
                          </div>
                          <div className="space-y-1">
                            <Label className="text-xs">{t("rtp.favoritePort")}</Label>
                            <Input
                              type="number"
                              placeholder="5004"
                              value={remote.port}
                              onChange={(e) =>
                                updateRemote(remote.id, { port: Number(e.target.value || 0) })
                              }
                            />
                          </div>
                        </div>
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-3">
                            <Switch
                              checked={remote.autoConnect}
                              onCheckedChange={(checked) =>
                                updateRemote(remote.id, { autoConnect: checked })
                              }
                            />
                            <span className="text-xs text-muted-foreground">
                              {t("rtp.autoConnect")}
                            </span>
                            <Badge variant={isRemoteConnected(remote) ? "success" : "secondary"}>
                              {isRemoteConnected(remote)
                                ? t("rtp.connected")
                                : t("rtp.disconnected")}
                            </Badge>
                          </div>
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => removeRemote(remote.id)}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ) : null}
          </div>

          <div className="space-y-4 p-4 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
            <h3 className="font-semibold text-sm">{t("rtp.advanced")}</h3>
            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <Label className="text-sm">{t("rtp.logRtp")}</Label>
                <Switch
                  checked={config?.logRtp ?? false}
                  onCheckedChange={(c) => updateConfig({ logRtp: c })}
                />
              </div>
              <div className="flex items-center justify-between">
                <Label className="text-sm">{t("rtp.verbose")}</Label>
                <Switch
                  checked={config?.verbose ?? false}
                  onCheckedChange={(c) => updateConfig({ verbose: c })}
                />
              </div>
            </div>
          </div>

          {remoteEnabled ? (
            <div className="space-y-4 p-4 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="font-semibold text-sm">{t("rtp.discoveredTitle")}</h3>
                  <p className="text-xs text-muted-foreground">{t("rtp.discoveredHint")}</p>
                </div>
                <Button variant="outline" size="sm" onClick={refreshSessions} disabled={discovering}>
                  <RefreshCcw className="h-3.5 w-3.5 mr-2" />
                  {t("common.refresh")}
                </Button>
              </div>
              <div className="space-y-2">
                {sessions.length === 0 ? (
                  <div className="text-xs text-muted-foreground">{t("rtp.discoveredEmpty")}</div>
                ) : (
                  sessions.map((session) => (
                    <div
                      key={`${session.name}-${session.host}-${session.port}`}
                      className="flex items-center justify-between rounded-md border border-slate-100 dark:border-slate-800 bg-white/70 dark:bg-slate-950/40 px-3 py-2"
                    >
                      <div className="space-y-0.5">
                        <div className="text-sm font-medium">{session.name}</div>
                        <div className="text-xs text-muted-foreground">
                          {session.addresses[0] ?? session.host}:{session.port}
                        </div>
                      </div>
                      <Button
                        variant={isSelectedSession(session) ? "secondary" : "default"}
                        size="sm"
                        onClick={() => handleConnect(session)}
                      >
                        {isSelectedSession(session) ? t("common.active") : t("rtp.connect")}
                      </Button>
                    </div>
                  ))
                )}
              </div>
            </div>
          ) : null}

          <div className="flex justify-end gap-2 pt-4 border-t">
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => updateConfig({ rtpSessionName: "OSCMidi", rtpPort: 5004 })}
            >
              <RefreshCcw className="h-4 w-4 mr-2" />
              {t("common.default")}
            </Button>
            <Button onClick={handleSave} disabled={busy}>
              {t("common.save")}
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm flex items-center gap-2">
            <Server className="h-4 w-4" />
            {t("rtp.connectedTitle")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          {participants.length === 0 ? (
            <div className="text-xs text-muted-foreground">{t("rtp.connectedEmpty")}</div>
          ) : (
            participants.map((participant) => (
              <div
                key={`${participant.addr}-${participant.name}`}
                className="flex items-center justify-between rounded-md border border-slate-100 dark:border-slate-800 bg-white/70 dark:bg-slate-950/40 px-3 py-2"
              >
                <div className="text-sm font-medium">{participant.name}</div>
                <code className="text-xs font-mono bg-slate-200 dark:bg-slate-700 px-1.5 py-0.5 rounded">
                  {participant.addr}
                </code>
              </div>
            ))
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm flex items-center gap-2">
            <Server className="h-4 w-4" />
            {t("rtp.statusTitle")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-3 md:grid-cols-2">
            <div className="p-3 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
              <div className="flex items-center justify-between mb-2">
                <span className="text-sm text-muted-foreground">{t("rtp.server")}</span>
                <Badge variant={status?.rtpActive ? "success" : "secondary"}>
                  {status?.rtpActive ? t("common.active") : t("common.inactive")}
                </Badge>
              </div>
              <div className="text-xs text-muted-foreground">
                {config?.rtpSessionName || "OSCMidi"} - {t("dashboard.rtpPort")} {config?.rtpPort || 5004}
              </div>
            </div>

            <div className="p-3 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
              <div className="flex items-center justify-between mb-2">
                <span className="text-sm text-muted-foreground">{t("rtp.boundPorts")}</span>
                <code className="text-xs font-mono bg-slate-200 dark:bg-slate-700 px-1.5 py-0.5 rounded">
                  {status?.rtpBoundPort ? `${status.rtpBoundPort}/${status.rtpBoundPort + 1}` : "-"}
                </code>
              </div>
              <div className="text-xs text-muted-foreground">
                {status?.rtpActive ? (
                  <div className="flex items-center gap-1">
                    <div className="w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse" />
                    {t("rtp.ready")}
                  </div>
                ) : (
                  t("rtp.waiting")
                )}
              </div>
            </div>
          </div>

          <div className="border-t pt-4 space-y-2">
            <h4 className="font-semibold text-sm">{t("rtp.details")}</h4>
            <div className="space-y-2">
              <div className="flex items-center justify-between p-2 rounded text-sm">
                <span className="text-muted-foreground">{t("rtp.sessionNameLabel")}</span>
                <span className="font-medium">{config?.rtpSessionName || "-"}</span>
              </div>
              <div className="flex items-center justify-between p-2 rounded text-sm">
                <span className="text-muted-foreground">{t("rtp.portConfigured")}</span>
                <span className="font-medium">{config?.rtpPort || "-"}</span>
              </div>
              <div className="flex items-center justify-between p-2 rounded text-sm">
                <span className="text-muted-foreground">{t("rtp.portData")}</span>
                <span className="font-medium">{config?.rtpPort ? config.rtpPort + 1 : "-"}</span>
              </div>
              <div className="flex items-center justify-between p-2 rounded text-sm">
                <span className="text-muted-foreground">{t("rtp.statusLabel")}</span>
                <span className="font-medium">
                  {status?.rtpActive ? t("common.active") : t("common.inactive")}
                </span>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm">{t("rtp.about")}</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground space-y-3">
          <p>{t("rtp.aboutText")}</p>
          <div className="bg-slate-50 dark:bg-slate-900/50 p-3 rounded-lg border border-slate-100 dark:border-slate-800">
            <p className="font-semibold text-xs mb-2">{t("rtp.defaultPorts")}</p>
            <ul className="text-xs space-y-1">
              <li>
                <code className="bg-slate-100 dark:bg-slate-800 px-1.5 py-0.5 rounded">5004</code>{" "}
                - {t("rtp.controlPort")}
              </li>
              <li>
                <code className="bg-slate-100 dark:bg-slate-800 px-1.5 py-0.5 rounded">5005</code>{" "}
                - {t("rtp.dataPort")}
              </li>
            </ul>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
