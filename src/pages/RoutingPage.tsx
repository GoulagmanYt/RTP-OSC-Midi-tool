import { useMemo, useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useBridge } from "../providers/BridgeProvider";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "../components/ui/Card";
import { Label } from "../components/ui/Label";
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from "../components/ui/Select";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Switch } from "../components/ui/Switch";
import { Badge } from "../components/ui/Badge";
import { Cable, Music, Zap, RefreshCcw, Activity, Plus, Trash2, SlidersHorizontal } from "lucide-react";
import { resetKeys, sendTestMidi, MidiActivityInfo } from "../api";
import { useI18n } from "../providers/LanguageProvider";
import { RTP_VIRTUAL_INPUT, VST_OUTPUT } from "../constants";

export default function RoutingPage() {
  const { config, status, updateConfig, midiInputs, midiOutputs, refreshLists } = useBridge();
  const { t } = useI18n();
  const [activity, setActivity] = useState<MidiActivityInfo[]>([]);
  const [testChannel, setTestChannel] = useState(1);
  const [testNote, setTestNote] = useState(60);
  const [testVelocity, setTestVelocity] = useState(100);
  const [testCc, setTestCc] = useState(1);
  const [testCcValue, setTestCcValue] = useState(64);

  const inputsList = useMemo(() => (Array.isArray(midiInputs) ? midiInputs : []), [midiInputs]);
  const outputsList = useMemo(() => (Array.isArray(midiOutputs) ? midiOutputs : []), [midiOutputs]);
  const profiles = config?.routingProfiles ?? [];
  const assignments = config?.routingAssignments ?? [];
  const sourceOptions = useMemo(() => {
    const rtpSource = config?.rtpSessionName ? `RTP:${config.rtpSessionName}` : "RTP:OSCMidi";
    const merged = [rtpSource, ...inputsList];
    return Array.from(new Set(merged)).filter(Boolean);
  }, [config?.rtpSessionName, inputsList]);

  useEffect(() => {
    const unlisten = listen<MidiActivityInfo[]>("midi_activity", (event) => {
      setActivity(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handleMidiInChange = useCallback(
    (value: string) => {
      updateConfig({ midiIn: value || null });
    },
    [updateConfig]
  );

  const handleMidiOutChange = useCallback(
    (value: string) => {
      updateConfig({ midiOut: value || null });
    },
    [updateConfig]
  );

  const handleChannelFilterChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const val = e.target.value;
      updateConfig({
        channelFilter: val ? Number(val) : null,
      });
    },
    [updateConfig]
  );

  const makeId = () =>
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `profile-${Date.now()}-${Math.random().toString(16).slice(2)}`;

  const updateProfile = (id: string, patch: Partial<(typeof profiles)[number]>) => {
    const next = profiles.map((profile) => (profile.id === id ? { ...profile, ...patch } : profile));
    updateConfig({ routingProfiles: next });
  };

  const addProfile = () => {
    const next = [
      ...profiles,
      {
        id: makeId(),
        name: `Profile ${profiles.length + 1}`,
        enabled: true,
        channelFilter: null,
        noteMin: null,
        noteMax: null,
        ccMap: [],
        programMap: [],
      },
    ];
    updateConfig({ routingProfiles: next });
  };

  const removeProfile = (id: string) => {
    const nextProfiles = profiles.filter((profile) => profile.id !== id);
    const nextAssignments = assignments.filter((assignment) => assignment.profileId !== id);
    updateConfig({ routingProfiles: nextProfiles, routingAssignments: nextAssignments });
  };

  const updateProfileMap = (
    id: string,
    mapKey: "ccMap" | "programMap",
    index: number,
    patch: Partial<(typeof profiles)[number]["ccMap"][number]>
  ) => {
    const next = profiles.map((profile) => {
      if (profile.id !== id) return profile;
      const map = profile[mapKey].map((entry, idx) => (idx === index ? { ...entry, ...patch } : entry));
      return { ...profile, [mapKey]: map };
    });
    updateConfig({ routingProfiles: next });
  };

  const addProfileMap = (id: string, mapKey: "ccMap" | "programMap") => {
    const next = profiles.map((profile) => {
      if (profile.id !== id) return profile;
      return { ...profile, [mapKey]: [...profile[mapKey], { from: 0, to: 0 }] };
    });
    updateConfig({ routingProfiles: next });
  };

  const removeProfileMap = (id: string, mapKey: "ccMap" | "programMap", index: number) => {
    const next = profiles.map((profile) => {
      if (profile.id !== id) return profile;
      return { ...profile, [mapKey]: profile[mapKey].filter((_, idx) => idx !== index) };
    });
    updateConfig({ routingProfiles: next });
  };

  const addAssignment = () => {
    if (profiles.length === 0) return;
    const source = sourceOptions[0] ?? "";
    const next = [...assignments, { source, profileId: profiles[0].id }];
    updateConfig({ routingAssignments: next });
  };

  const updateAssignment = (index: number, patch: Partial<(typeof assignments)[number]>) => {
    const next = assignments.map((assignment, idx) => (idx === index ? { ...assignment, ...patch } : assignment));
    updateConfig({ routingAssignments: next });
  };

  const removeAssignment = (index: number) => {
    updateConfig({ routingAssignments: assignments.filter((_, idx) => idx !== index) });
  };

  const handleTestNote = async () => {
    try {
      await sendTestMidi({
        kind: "note",
        channel: testChannel,
        note: testNote,
        velocity: testVelocity,
        cc: 0,
        value: 0,
      });
    } catch (e) {
      console.error("Failed to send test note", e);
    }
  };

  const handleTestCc = async () => {
    try {
      await sendTestMidi({
        kind: "cc",
        channel: testChannel,
        note: 0,
        velocity: 0,
        cc: testCc,
        value: testCcValue,
      });
    } catch (e) {
      console.error("Failed to send test CC", e);
    }
  };

  const formatNote = (note?: number | null) => {
    if (note === null || note === undefined) return "--";
    const names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    const octave = Math.floor(note / 12) - 1;
    return `${names[note % 12]}${octave}`;
  };

  const handleReset = () => {
    updateConfig({
      midiIn: null,
      midiOut: null,
      channelFilter: null,
      midiThru: true,
      audioGainDb: 0,
    });
  };

  const handlePanic = async () => {
    try {
      await resetKeys();
    } catch (e) {
      console.error("Failed to send MIDI panic", e);
    }
  };

  const displayPortName = (name: string) => {
    if (name === RTP_VIRTUAL_INPUT) return t("devices.rtpVirtualInput");
    if (name === VST_OUTPUT) return t("devices.vstInternalOutput");
    return name;
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Cable className="h-5 w-5 text-blue-500" />
            {t("routing.title")}
          </CardTitle>
          <CardDescription>{t("routing.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="grid gap-6 md:grid-cols-3 items-center">
            <div className="space-y-2">
              <Label htmlFor="midi-input" className="text-sm font-medium">
                {t("routing.midiInput")}
              </Label>
              <Select value={config?.midiIn ?? ""} onValueChange={handleMidiInChange}>
                <SelectTrigger>
                  <SelectValue placeholder={t("routing.noneSelected")} />
                </SelectTrigger>
                <SelectContent>
                  {inputsList.map((input) => (
                    <SelectItem key={input} value={input}>
                      {displayPortName(input)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="flex flex-col items-center justify-center gap-2 text-sm text-muted-foreground">
              <Music className="h-5 w-5 text-muted-foreground" />
              <span>{t("routing.midiRelay")}</span>
            </div>

            <div className="space-y-2">
              <Label htmlFor="midi-output" className="text-sm font-medium">
                {t("routing.midiOutput")}
              </Label>
              <Select value={config?.midiOut ?? ""} onValueChange={handleMidiOutChange}>
                <SelectTrigger>
                  <SelectValue placeholder={t("routing.noneSelected")} />
                </SelectTrigger>
                <SelectContent>
                  {outputsList.map((output) => (
                    <SelectItem key={output} value={output}>
                      {displayPortName(output)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex flex-col gap-1">
                  <Label>{t("routing.channelFilter")}</Label>
                  <p className="text-xs text-muted-foreground">{t("routing.channelFilterHint")}</p>
                </div>
                <Input
                  className="w-28"
                  type="number"
                  min="1"
                  max="16"
                  placeholder={t("routing.all")}
                  value={config?.channelFilter ? String(config.channelFilter) : ""}
                  onChange={handleChannelFilterChange}
                />
              </div>

              <div className="flex items-center justify-between">
                <div className="flex flex-col gap-1">
                  <Label>{t("routing.midiThru")}</Label>
                  <p className="text-xs text-muted-foreground">{t("routing.midiThruHint")}</p>
                </div>
                <Switch checked={config?.midiThru ?? false} onCheckedChange={(c) => updateConfig({ midiThru: c })} />
              </div>

              <div className="flex items-center justify-between">
                <div className="flex flex-col gap-1">
                  <Label>{t("routing.hotplug")}</Label>
                  <p className="text-xs text-muted-foreground">{t("routing.hotplugHint")}</p>
                </div>
                <Switch checked={config?.hotplug ?? false} onCheckedChange={(c) => updateConfig({ hotplug: c })} />
              </div>
            </div>

            <div className="flex flex-col gap-3">
              <div className="flex items-center justify-between">
                <div className="flex flex-col gap-1">
                  <Label>{t("routing.audioGain")}</Label>
                  <p className="text-xs text-muted-foreground">{t("routing.audioGainHint")}</p>
                </div>
                <Input
                  className="w-28"
                  type="number"
                  min="-24"
                  max="12"
                  step="1"
                  value={config?.audioGainDb ?? 0}
                  onChange={(e) => updateConfig({ audioGainDb: Number(e.target.value || 0) })}
                />
              </div>

              <div className="flex gap-2 flex-wrap">
                <Button variant="outline" size="sm" onClick={refreshLists}>
                  <RefreshCcw className="h-4 w-4 mr-2" />
                  {t("routing.refreshPorts")}
                </Button>
                <Button variant="outline" size="sm" onClick={handleReset}>
                  {t("common.reset")}
                </Button>
                <Button variant="destructive" size="sm" onClick={handlePanic}>
                  <Zap className="h-4 w-4 mr-2" />
                  {t("routing.panic")}
                </Button>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>


      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <SlidersHorizontal className="h-5 w-5 text-blue-500" />
            {t("routing.advancedTitle")}
          </CardTitle>
          <CardDescription>{t("routing.advancedDescription")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <Label>{t("routing.profilesTitle")}</Label>
                <p className="text-xs text-muted-foreground">{t("routing.profilesHint")}</p>
              </div>
              <Button variant="outline" size="sm" onClick={addProfile}>
                <Plus className="h-4 w-4 mr-2" />
                {t("routing.addProfile")}
              </Button>
            </div>
            {profiles.length === 0 ? (
              <div className="text-xs text-muted-foreground">{t("routing.profilesEmpty")}</div>
            ) : (
              <div className="space-y-4">
                {profiles.map((profile) => (
                  <div
                    key={profile.id}
                    className="rounded-md border border-slate-100 dark:border-slate-800 bg-white/70 dark:bg-slate-950/40 p-4 space-y-4"
                  >
                    <div className="flex flex-wrap items-center justify-between gap-3">
                      <Input
                        className="min-w-[180px] flex-1"
                        type="text"
                        value={profile.name}
                        onChange={(e) => updateProfile(profile.id, { name: e.target.value })}
                      />
                      <div className="flex items-center gap-2">
                        <Switch
                          checked={profile.enabled}
                          onCheckedChange={(checked) => updateProfile(profile.id, { enabled: checked })}
                        />
                        <span className="text-xs text-muted-foreground">{t("routing.profileEnabled")}</span>
                      </div>
                      <Button variant="ghost" size="sm" onClick={() => removeProfile(profile.id)}>
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                    <div className="grid gap-3 md:grid-cols-3">
                      <div className="space-y-1">
                        <Label className="text-xs">{t("routing.profileChannel")}</Label>
                        <Input
                          type="number"
                          min="1"
                          max="16"
                          placeholder={t("routing.all")}
                          value={profile.channelFilter ?? ""}
                          onChange={(e) =>
                            updateProfile(profile.id, {
                              channelFilter: e.target.value ? Number(e.target.value) : null,
                            })
                          }
                        />
                      </div>
                      <div className="space-y-1">
                        <Label className="text-xs">{t("routing.profileNoteMin")}</Label>
                        <Input
                          type="number"
                          min="0"
                          max="127"
                          placeholder="0"
                          value={profile.noteMin ?? ""}
                          onChange={(e) =>
                            updateProfile(profile.id, {
                              noteMin: e.target.value ? Number(e.target.value) : null,
                            })
                          }
                        />
                      </div>
                      <div className="space-y-1">
                        <Label className="text-xs">{t("routing.profileNoteMax")}</Label>
                        <Input
                          type="number"
                          min="0"
                          max="127"
                          placeholder="127"
                          value={profile.noteMax ?? ""}
                          onChange={(e) =>
                            updateProfile(profile.id, {
                              noteMax: e.target.value ? Number(e.target.value) : null,
                            })
                          }
                        />
                      </div>
                    </div>
                    <div className="grid gap-4 md:grid-cols-2">
                      <div className="space-y-2">
                        <div className="flex items-center justify-between">
                          <Label className="text-xs">{t("routing.ccMapTitle")}</Label>
                          <Button variant="outline" size="sm" onClick={() => addProfileMap(profile.id, "ccMap")}>
                            <Plus className="h-3.5 w-3.5 mr-1" />
                            {t("routing.mapAdd")}
                          </Button>
                        </div>
                        {profile.ccMap.length === 0 ? (
                          <div className="text-xs text-muted-foreground">{t("routing.mapEmpty")}</div>
                        ) : (
                          <div className="space-y-2">
                            {profile.ccMap.map((entry, idx) => (
                              <div
                                key={`${profile.id}-cc-${idx}`}
                                className="grid grid-cols-[1fr_1fr_auto] gap-2 items-center"
                              >
                                <Input
                                  type="number"
                                  min="0"
                                  max="127"
                                  value={entry.from}
                                  onChange={(e) =>
                                    updateProfileMap(profile.id, "ccMap", idx, {
                                      from: Number(e.target.value || 0),
                                    })
                                  }
                                />
                                <Input
                                  type="number"
                                  min="0"
                                  max="127"
                                  value={entry.to}
                                  onChange={(e) =>
                                    updateProfileMap(profile.id, "ccMap", idx, {
                                      to: Number(e.target.value || 0),
                                    })
                                  }
                                />
                                <Button
                                  variant="ghost"
                                  size="icon"
                                  onClick={() => removeProfileMap(profile.id, "ccMap", idx)}
                                >
                                  <Trash2 className="h-4 w-4" />
                                </Button>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                      <div className="space-y-2">
                        <div className="flex items-center justify-between">
                          <Label className="text-xs">{t("routing.programMapTitle")}</Label>
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => addProfileMap(profile.id, "programMap")}
                          >
                            <Plus className="h-3.5 w-3.5 mr-1" />
                            {t("routing.mapAdd")}
                          </Button>
                        </div>
                        {profile.programMap.length === 0 ? (
                          <div className="text-xs text-muted-foreground">{t("routing.mapEmpty")}</div>
                        ) : (
                          <div className="space-y-2">
                            {profile.programMap.map((entry, idx) => (
                              <div
                                key={`${profile.id}-program-${idx}`}
                                className="grid grid-cols-[1fr_1fr_auto] gap-2 items-center"
                              >
                                <Input
                                  type="number"
                                  min="0"
                                  max="127"
                                  value={entry.from}
                                  onChange={(e) =>
                                    updateProfileMap(profile.id, "programMap", idx, {
                                      from: Number(e.target.value || 0),
                                    })
                                  }
                                />
                                <Input
                                  type="number"
                                  min="0"
                                  max="127"
                                  value={entry.to}
                                  onChange={(e) =>
                                    updateProfileMap(profile.id, "programMap", idx, {
                                      to: Number(e.target.value || 0),
                                    })
                                  }
                                />
                                <Button
                                  variant="ghost"
                                  size="icon"
                                  onClick={() => removeProfileMap(profile.id, "programMap", idx)}
                                >
                                  <Trash2 className="h-4 w-4" />
                                </Button>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          <div className="border-t pt-4 space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <Label>{t("routing.assignmentsTitle")}</Label>
                <p className="text-xs text-muted-foreground">{t("routing.assignmentsHint")}</p>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={addAssignment}
                disabled={profiles.length === 0}
              >
                <Plus className="h-4 w-4 mr-2" />
                {t("routing.addAssignment")}
              </Button>
            </div>
            {assignments.length === 0 ? (
              <div className="text-xs text-muted-foreground">{t("routing.assignmentsEmpty")}</div>
            ) : (
              <div className="space-y-2">
                {assignments.map((assignment, idx) => (
                  <div
                    key={`${assignment.source}-${idx}`}
                    className="grid gap-3 md:grid-cols-[1fr_1fr_auto] items-center"
                  >
                    <Select
                      value={assignment.source}
                      onValueChange={(value) => updateAssignment(idx, { source: value })}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder={t("routing.assignmentSource")} />
                      </SelectTrigger>
                      <SelectContent>
                        {sourceOptions.map((source) => (
                          <SelectItem key={source} value={source}>
                            {source}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Select
                      value={assignment.profileId}
                      onValueChange={(value) => updateAssignment(idx, { profileId: value })}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder={t("routing.assignmentProfile")} />
                      </SelectTrigger>
                      <SelectContent>
                        {profiles.map((profile) => (
                          <SelectItem key={profile.id} value={profile.id}>
                            {profile.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Button variant="ghost" size="icon" onClick={() => removeAssignment(idx)}>
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Zap className="h-5 w-5 text-blue-500" />
            {t("routing.testMidiTitle")}
          </CardTitle>
          <CardDescription>{t("routing.testMidiDescription")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-3 md:grid-cols-5">
            <div className="space-y-1">
              <Label className="text-xs">{t("routing.testChannel")}</Label>
              <Input
                type="number"
                min="1"
                max="16"
                value={testChannel}
                onChange={(e) => setTestChannel(Number(e.target.value || 1))}
              />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">{t("routing.testNote")}</Label>
              <Input
                type="number"
                min="0"
                max="127"
                value={testNote}
                onChange={(e) => setTestNote(Number(e.target.value || 0))}
              />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">{t("routing.testVelocity")}</Label>
              <Input
                type="number"
                min="0"
                max="127"
                value={testVelocity}
                onChange={(e) => setTestVelocity(Number(e.target.value || 0))}
              />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">{t("routing.testCc")}</Label>
              <Input
                type="number"
                min="0"
                max="127"
                value={testCc}
                onChange={(e) => setTestCc(Number(e.target.value || 0))}
              />
            </div>
            <div className="space-y-1">
              <Label className="text-xs">{t("routing.testCcValue")}</Label>
              <Input
                type="number"
                min="0"
                max="127"
                value={testCcValue}
                onChange={(e) => setTestCcValue(Number(e.target.value || 0))}
              />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button onClick={handleTestNote}>{t("routing.sendTestNote")}</Button>
            <Button variant="outline" onClick={handleTestCc}>
              {t("routing.sendTestCc")}
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm flex items-center gap-2">
            <Activity className="h-4 w-4" />
            {t("routing.statusTitle")}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="grid gap-3 md:grid-cols-3">
            <div className="flex items-center justify-between p-3 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
              <span className="text-sm text-muted-foreground">{t("routing.input")}</span>
              <Badge variant={config?.midiIn ? "success" : "secondary"}>
                {config?.midiIn ? displayPortName(config.midiIn) : t("routing.notConfigured")}
              </Badge>
            </div>
            <div className="flex items-center justify-between p-3 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
              <span className="text-sm text-muted-foreground">{t("routing.output")}</span>
              <Badge variant={config?.midiOut ? "success" : "secondary"}>
                {config?.midiOut ? displayPortName(config.midiOut) : t("routing.notConfigured")}
              </Badge>
            </div>
            <div className="flex items-center justify-between p-3 rounded-lg bg-slate-50 dark:bg-slate-900/50 border border-slate-100 dark:border-slate-800">
              <span className="text-sm text-muted-foreground">{t("routing.bridge")}</span>
              <Badge variant={status?.running ? "success" : "secondary"}>
                {status?.running ? t("common.active") : t("common.stopped")}
              </Badge>
            </div>
          </div>
          <p className="text-xs text-muted-foreground">{t("routing.tip")}</p>
          <div className="border-t pt-4 space-y-3">
            <h4 className="text-sm font-semibold">{t("routing.activityTitle")}</h4>
            {activity.length === 0 ? (
              <div className="text-xs text-muted-foreground">{t("routing.activityEmpty")}</div>
            ) : (
              <div className="space-y-2">
                {activity.map((item) => {
                  const isActive =
                    typeof item.lastSeenMs === "number" && Date.now() - item.lastSeenMs < 1200;
                  const level = Math.min(item.messagesPerSec ?? 0, 60);
                  return (
                    <div
                      key={item.source}
                      className="flex items-center justify-between rounded-md border border-slate-100 dark:border-slate-800 bg-white/70 dark:bg-slate-950/40 px-3 py-2"
                    >
                      <div className="flex items-center gap-3">
                        <div
                          className={`h-2.5 w-2.5 rounded-full ${
                            isActive ? "bg-emerald-500 animate-pulse" : "bg-slate-400"
                          }`}
                        />
                        <div>
                          <div className="text-sm font-medium">{item.source}</div>
                          <div className="text-xs text-muted-foreground">
                            {t("routing.activityNote")} {formatNote(item.lastNote)} -{" "}
                            {t("routing.activityChannel")} {item.lastChannel ?? "--"}
                          </div>
                        </div>
                      </div>
                      <div className="text-right space-y-1">
                        <div className="text-sm font-semibold">{item.messagesPerSec ?? 0}/s</div>
                        <div className="h-1.5 w-20 bg-slate-200 dark:bg-slate-800 rounded-full overflow-hidden">
                          <div
                            className="h-full bg-emerald-500"
                            style={{ width: `${(level / 60) * 100}%` }}
                          />
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
