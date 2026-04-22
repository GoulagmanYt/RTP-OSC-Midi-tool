import type { RoutingAssignment, RoutingProfile } from "../../api";
import { Plus, SlidersHorizontal, Trash2 } from "lucide-react";

import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import { Input } from "../../components/ui/Input";
import { Label } from "../../components/ui/Label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../components/ui/Select";
import { Switch } from "../../components/ui/Switch";

import type { Translate } from "./shared";

type Props = {
  t: Translate;
  profiles: RoutingProfile[];
  assignments: RoutingAssignment[];
  sourceOptions: string[];
  addProfile: () => void;
  removeProfile: (id: string) => void;
  updateProfile: (id: string, patch: Partial<RoutingProfile>) => void;
  updateProfileMap: (
    id: string,
    mapKey: "ccMap" | "programMap",
    index: number,
    patch: Partial<RoutingProfile["ccMap"][number]>
  ) => void;
  addProfileMap: (id: string, mapKey: "ccMap" | "programMap") => void;
  removeProfileMap: (id: string, mapKey: "ccMap" | "programMap", index: number) => void;
  addAssignment: () => void;
  updateAssignment: (index: number, patch: Partial<RoutingAssignment>) => void;
  removeAssignment: (index: number) => void;
};

export function AdvancedRoutingCard(props: Props) {
  const {
    t,
    profiles,
    assignments,
    sourceOptions,
    addProfile,
    removeProfile,
    updateProfile,
    updateProfileMap,
    addProfileMap,
    removeProfileMap,
    addAssignment,
    updateAssignment,
    removeAssignment,
  } = props;

  return (
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
              <Plus className="mr-2 h-4 w-4" />
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
                  className="space-y-4 rounded-md border border-slate-100 bg-white/70 p-4 dark:border-slate-800 dark:bg-slate-950/40"
                >
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <Input
                      className="min-w-[180px] flex-1"
                      type="text"
                      value={profile.name}
                      onChange={(event) => updateProfile(profile.id, { name: event.target.value })}
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
                        onChange={(event) =>
                          updateProfile(profile.id, {
                            channelFilter: event.target.value ? Number(event.target.value) : null,
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
                        onChange={(event) =>
                          updateProfile(profile.id, {
                            noteMin: event.target.value ? Number(event.target.value) : null,
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
                        onChange={(event) =>
                          updateProfile(profile.id, {
                            noteMax: event.target.value ? Number(event.target.value) : null,
                          })
                        }
                      />
                    </div>
                  </div>
                  <div className="grid gap-4 md:grid-cols-2">
                    {(["ccMap", "programMap"] as const).map((mapKey) => (
                      <div key={mapKey} className="space-y-2">
                        <div className="flex items-center justify-between">
                          <Label className="text-xs">
                            {mapKey === "ccMap" ? t("routing.ccMapTitle") : t("routing.programMapTitle")}
                          </Label>
                          <Button variant="outline" size="sm" onClick={() => addProfileMap(profile.id, mapKey)}>
                            <Plus className="mr-1 h-3.5 w-3.5" />
                            {t("routing.mapAdd")}
                          </Button>
                        </div>
                        {profile[mapKey].length === 0 ? (
                          <div className="text-xs text-muted-foreground">{t("routing.mapEmpty")}</div>
                        ) : (
                          <div className="space-y-2">
                            {profile[mapKey].map((entry, idx) => (
                              <div key={`${profile.id}-${mapKey}-${idx}`} className="grid grid-cols-[1fr_1fr_auto] items-center gap-2">
                                <Input
                                  type="number"
                                  min="0"
                                  max="127"
                                  value={entry.from}
                                  onChange={(event) =>
                                    updateProfileMap(profile.id, mapKey, idx, {
                                      from: Number(event.target.value || 0),
                                    })
                                  }
                                />
                                <Input
                                  type="number"
                                  min="0"
                                  max="127"
                                  value={entry.to}
                                  onChange={(event) =>
                                    updateProfileMap(profile.id, mapKey, idx, {
                                      to: Number(event.target.value || 0),
                                    })
                                  }
                                />
                                <Button variant="ghost" size="icon" onClick={() => removeProfileMap(profile.id, mapKey, idx)}>
                                  <Trash2 className="h-4 w-4" />
                                </Button>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="space-y-3 border-t pt-4">
          <div className="flex items-center justify-between">
            <div>
              <Label>{t("routing.assignmentsTitle")}</Label>
              <p className="text-xs text-muted-foreground">{t("routing.assignmentsHint")}</p>
            </div>
            <Button variant="outline" size="sm" onClick={addAssignment} disabled={profiles.length === 0}>
              <Plus className="mr-2 h-4 w-4" />
              {t("routing.addAssignment")}
            </Button>
          </div>
          {assignments.length === 0 ? (
            <div className="text-xs text-muted-foreground">{t("routing.assignmentsEmpty")}</div>
          ) : (
            <div className="space-y-2">
              {assignments.map((assignment, idx) => (
                <div key={`${assignment.source}-${idx}`} className="grid items-center gap-3 md:grid-cols-[1fr_1fr_auto]">
                  <Select value={assignment.source} onValueChange={(value) => updateAssignment(idx, { source: value })}>
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
                  <Select value={assignment.profileId} onValueChange={(value) => updateAssignment(idx, { profileId: value })}>
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
  );
}
