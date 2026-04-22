import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import { Input } from "../../components/ui/Input";
import { Label } from "../../components/ui/Label";
import { Switch } from "../../components/ui/Switch";
import type { AppConfig, DeepPartial, RtpRemoteEntry, RtpSessionInfo } from "../../api-types";
import type { TranslateFn } from "./shared";
import { Plus, RefreshCcw, Trash2, Wifi } from "lucide-react";

type Props = {
  busy: boolean;
  config: AppConfig | null;
  discovering: boolean;
  remoteEnabled: boolean;
  remotes: RtpRemoteEntry[];
  sessions: RtpSessionInfo[];
  t: TranslateFn;
  isRemoteConnected: (remote: RtpRemoteEntry) => boolean;
  isSelectedSession: (session: RtpSessionInfo) => boolean;
  onAddRemote: () => Promise<void>;
  onConnect: (session: RtpSessionInfo) => Promise<void>;
  onRefreshSessions: () => Promise<void>;
  onRemoveRemote: (id: string) => Promise<void>;
  onSave: () => Promise<void>;
  onToggleRtp: (checked: boolean) => Promise<void>;
  onUpdateConfig: (patch: DeepPartial<AppConfig>) => Promise<void>;
  onUpdateRemote: (id: string, patch: Partial<RtpRemoteEntry>) => Promise<void>;
};

export function RtpConfigCard({
  busy,
  config,
  discovering,
  remoteEnabled,
  remotes,
  sessions,
  t,
  isRemoteConnected,
  isSelectedSession,
  onAddRemote,
  onConnect,
  onRefreshSessions,
  onRemoveRemote,
  onSave,
  onToggleRtp,
  onUpdateConfig,
  onUpdateRemote,
}: Props) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Wifi className="h-5 w-5 text-blue-500" />
          {t("rtp.title")}
        </CardTitle>
        <CardDescription>{t("rtp.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="flex items-center justify-between rounded-lg border border-slate-100 bg-slate-50 p-4 dark:border-slate-800 dark:bg-slate-900/50">
          <div className="flex flex-col gap-1">
            <Label className="font-medium">{t("rtp.enable")}</Label>
            <p className="text-xs text-muted-foreground">{t("rtp.enableHint")}</p>
          </div>
          <Switch checked={config?.rtp.enabled ?? false} onCheckedChange={onToggleRtp} />
        </div>

        <div className="space-y-4 rounded-lg border border-slate-100 bg-slate-50 p-4 dark:border-slate-800 dark:bg-slate-900/50">
          <h3 className="text-sm font-semibold">{t("rtp.sessionConfig")}</h3>
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="rtp-session" className="text-sm">
                {t("rtp.sessionName")}
              </Label>
              <Input
                id="rtp-session"
                type="text"
                placeholder="OSCMidi"
                value={config?.rtp.sessionName ?? ""}
                onChange={(e) => onUpdateConfig({ rtp: { sessionName: e.target.value } })}
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
                value={config?.rtp.port ?? ""}
                onChange={(e) => onUpdateConfig({ rtp: { port: Number(e.target.value || 0) } })}
              />
              <p className="text-xs text-muted-foreground">{t("rtp.portHint")}</p>
            </div>
          </div>
        </div>

        <div className="space-y-4 rounded-lg border border-slate-100 bg-slate-50 p-4 dark:border-slate-800 dark:bg-slate-900/50">
          <h3 className="text-sm font-semibold">{t("rtp.remoteConfig")}</h3>
          <div className="flex items-center justify-between">
            <div className="flex flex-col gap-1">
              <Label className="font-medium">{t("rtp.remoteEnable")}</Label>
              <p className="text-xs text-muted-foreground">{t("rtp.remoteEnableHint")}</p>
            </div>
            <Switch
              checked={remoteEnabled}
              onCheckedChange={(checked) => onUpdateConfig({ rtp: { remoteEnabled: checked } })}
            />
          </div>
          {remoteEnabled ? (
            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <div>
                  <Label className="text-sm">{t("rtp.favoritesTitle")}</Label>
                  <p className="text-xs text-muted-foreground">{t("rtp.favoritesHint")}</p>
                </div>
                <Button variant="outline" size="sm" onClick={onAddRemote}>
                  <Plus className="mr-2 h-3.5 w-3.5" />
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
                      className="space-y-3 rounded-md border border-slate-100 bg-white/70 p-3 dark:border-slate-800 dark:bg-slate-950/40"
                    >
                      <div className="grid gap-3 md:grid-cols-3">
                        <div className="space-y-1">
                          <Label className="text-xs">{t("rtp.favoriteName")}</Label>
                          <Input
                            type="text"
                            placeholder="pianoledvisualizer"
                            value={remote.name}
                            onChange={(e) => onUpdateRemote(remote.id, { name: e.target.value })}
                          />
                        </div>
                        <div className="space-y-1">
                          <Label className="text-xs">{t("rtp.favoriteHost")}</Label>
                          <Input
                            type="text"
                            placeholder="192.168.0.50"
                            value={remote.host}
                            onChange={(e) => onUpdateRemote(remote.id, { host: e.target.value })}
                          />
                        </div>
                        <div className="space-y-1">
                          <Label className="text-xs">{t("rtp.favoritePort")}</Label>
                          <Input
                            type="number"
                            placeholder="5004"
                            value={remote.port}
                            onChange={(e) =>
                              onUpdateRemote(remote.id, { port: Number(e.target.value || 0) })
                            }
                          />
                        </div>
                      </div>
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                          <Switch
                            checked={remote.autoConnect}
                            onCheckedChange={(checked) =>
                              onUpdateRemote(remote.id, { autoConnect: checked })
                            }
                          />
                          <span className="text-xs text-muted-foreground">{t("rtp.autoConnect")}</span>
                          <Badge variant={isRemoteConnected(remote) ? "success" : "secondary"}>
                            {isRemoteConnected(remote) ? t("rtp.connected") : t("rtp.disconnected")}
                          </Badge>
                        </div>
                        <Button variant="ghost" size="sm" onClick={() => onRemoveRemote(remote.id)}>
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

        <div className="space-y-4 rounded-lg border border-slate-100 bg-slate-50 p-4 dark:border-slate-800 dark:bg-slate-900/50">
          <h3 className="text-sm font-semibold">{t("rtp.advanced")}</h3>
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <Label className="text-sm">{t("rtp.logRtp")}</Label>
              <Switch
                checked={config?.rtp.logMessages ?? false}
                onCheckedChange={(checked) => onUpdateConfig({ rtp: { logMessages: checked } })}
              />
            </div>
            <div className="flex items-center justify-between">
              <Label className="text-sm">{t("rtp.verbose")}</Label>
              <Switch
                checked={config?.logging.verbose ?? false}
                onCheckedChange={(checked) => onUpdateConfig({ logging: { verbose: checked } })}
              />
            </div>
          </div>
        </div>

        {remoteEnabled ? (
          <div className="space-y-4 rounded-lg border border-slate-100 bg-slate-50 p-4 dark:border-slate-800 dark:bg-slate-900/50">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-semibold">{t("rtp.discoveredTitle")}</h3>
                <p className="text-xs text-muted-foreground">{t("rtp.discoveredHint")}</p>
              </div>
              <Button variant="outline" size="sm" onClick={onRefreshSessions} disabled={discovering}>
                <RefreshCcw className="mr-2 h-3.5 w-3.5" />
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
                    className="flex items-center justify-between rounded-md border border-slate-100 bg-white/70 px-3 py-2 dark:border-slate-800 dark:bg-slate-950/40"
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
                      onClick={() => onConnect(session)}
                    >
                      {isSelectedSession(session) ? t("common.active") : t("rtp.connect")}
                    </Button>
                  </div>
                ))
              )}
            </div>
          </div>
        ) : null}

        <div className="flex justify-end gap-2 border-t pt-4">
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => onUpdateConfig({ rtp: { sessionName: "OSCMidi", port: 5004 } })}
          >
            <RefreshCcw className="mr-2 h-4 w-4" />
            {t("common.default")}
          </Button>
          <Button onClick={onSave} disabled={busy}>
            {t("common.save")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
