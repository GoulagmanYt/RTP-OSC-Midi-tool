import { useMemo, useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useBridge } from "../providers/BridgeProvider";
import { panicMidi, sendTestMidi } from "../api";
import type { MidiActivityInfo } from "../api";
import { useI18n } from "../providers/LanguageProvider";
import { AdvancedRoutingCard } from "./routing/AdvancedRoutingCard";
import { BasicRoutingCard } from "./routing/BasicRoutingCard";
import { RoutingStatusCard } from "./routing/RoutingStatusCard";
import { TestMidiCard } from "./routing/TestMidiCard";
import { displayPortName as displayPortNameLabel, formatNote } from "./routing/shared";
import { toast } from "sonner";
import { getErrorMessage } from "../api-errors";

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
  const profiles = config?.midi.routingProfiles ?? [];
  const assignments = config?.midi.routingAssignments ?? [];
  const sourceOptions = useMemo(() => {
    const rtpSource = config?.rtp.sessionName ? `RTP:${config.rtp.sessionName}` : "RTP:OSCMidi";
    const merged = [rtpSource, ...inputsList];
    return Array.from(new Set(merged)).filter(Boolean);
  }, [config?.rtp.sessionName, inputsList]);

  useEffect(() => {
    const unlisten = listen<MidiActivityInfo[]>("midi:activity", (event) => {
      setActivity(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const handleMidiInChange = useCallback(
    (value: string) => {
      updateConfig({ midi: { inputDevice: value || null } });
    },
    [updateConfig]
  );

  const handleMidiOutChange = useCallback(
    (value: string) => {
      updateConfig({ midi: { outputDevice: value || null } });
    },
    [updateConfig]
  );

  const handleChannelFilterChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const val = e.target.value;
      updateConfig({
        midi: {
          channelFilter: val ? Number(val) : null,
        },
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
    updateConfig({ midi: { routingProfiles: next } });
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
    updateConfig({ midi: { routingProfiles: next } });
  };

  const removeProfile = (id: string) => {
    const nextProfiles = profiles.filter((profile) => profile.id !== id);
    const nextAssignments = assignments.filter((assignment) => assignment.profileId !== id);
    updateConfig({ midi: { routingProfiles: nextProfiles, routingAssignments: nextAssignments } });
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
    updateConfig({ midi: { routingProfiles: next } });
  };

  const addProfileMap = (id: string, mapKey: "ccMap" | "programMap") => {
    const next = profiles.map((profile) => {
      if (profile.id !== id) return profile;
      return { ...profile, [mapKey]: [...profile[mapKey], { from: 0, to: 0 }] };
    });
    updateConfig({ midi: { routingProfiles: next } });
  };

  const removeProfileMap = (id: string, mapKey: "ccMap" | "programMap", index: number) => {
    const next = profiles.map((profile) => {
      if (profile.id !== id) return profile;
      return { ...profile, [mapKey]: profile[mapKey].filter((_, idx) => idx !== index) };
    });
    updateConfig({ midi: { routingProfiles: next } });
  };

  const addAssignment = () => {
    if (profiles.length === 0) return;
    const source = sourceOptions[0] ?? "";
    const next = [...assignments, { source, profileId: profiles[0].id }];
    updateConfig({ midi: { routingAssignments: next } });
  };

  const updateAssignment = (index: number, patch: Partial<(typeof assignments)[number]>) => {
    const next = assignments.map((assignment, idx) => (idx === index ? { ...assignment, ...patch } : assignment));
    updateConfig({ midi: { routingAssignments: next } });
  };

  const removeAssignment = (index: number) => {
    updateConfig({ midi: { routingAssignments: assignments.filter((_, idx) => idx !== index) } });
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
      toast.error(getErrorMessage(e) || t("toasts.routing.testMidiFailed"));
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
      toast.error(getErrorMessage(e) || t("toasts.routing.testMidiFailed"));
    }
  };

  const handleReset = () => {
    updateConfig({
      midi: {
        inputDevice: null,
        outputDevice: null,
        channelFilter: null,
        thruEnabled: true,
      },
      audio: {
        gainDb: 0,
      },
    });
  };

  const handlePanic = async () => {
    try {
      await panicMidi();
      toast.success(t("toasts.routing.panicSent"));
    } catch (e) {
      console.error("Failed to send MIDI panic", e);
      toast.error(getErrorMessage(e) || t("toasts.routing.panicFailed"));
    }
  };

  const displayPortName = (name: string) => displayPortNameLabel(name, t);

  return (
    <div className="space-y-6">
      <BasicRoutingCard
        config={config}
        inputsList={inputsList}
        outputsList={outputsList}
        t={t}
        onMidiInChange={handleMidiInChange}
        onMidiOutChange={handleMidiOutChange}
        onChannelFilterChange={handleChannelFilterChange}
        onRefreshLists={refreshLists}
        onReset={handleReset}
        onPanic={handlePanic}
        onUpdateConfig={updateConfig}
        displayPortName={displayPortName}
      />
      <AdvancedRoutingCard
        t={t}
        profiles={profiles}
        assignments={assignments}
        sourceOptions={sourceOptions}
        addProfile={addProfile}
        removeProfile={removeProfile}
        updateProfile={updateProfile}
        updateProfileMap={updateProfileMap}
        addProfileMap={addProfileMap}
        removeProfileMap={removeProfileMap}
        addAssignment={addAssignment}
        updateAssignment={updateAssignment}
        removeAssignment={removeAssignment}
      />
      {config?.ui.developerMode && (
        <TestMidiCard
          t={t}
          testChannel={testChannel}
          testNote={testNote}
          testVelocity={testVelocity}
          testCc={testCc}
          testCcValue={testCcValue}
          setTestChannel={setTestChannel}
          setTestNote={setTestNote}
          setTestVelocity={setTestVelocity}
          setTestCc={setTestCc}
          setTestCcValue={setTestCcValue}
          onTestNote={handleTestNote}
          onTestCc={handleTestCc}
        />
      )}
      <RoutingStatusCard
        t={t}
        config={config}
        status={status}
        activity={activity}
        displayPortName={displayPortName}
        formatNote={formatNote}
      />
    </div>
  );
}
