import { Zap } from "lucide-react";

import { Button } from "../../components/ui/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../../components/ui/Card";
import { Input } from "../../components/ui/Input";
import { Label } from "../../components/ui/Label";

import type { Translate } from "./shared";

type Props = {
  t: Translate;
  testChannel: number;
  testNote: number;
  testVelocity: number;
  testCc: number;
  testCcValue: number;
  setTestChannel: (value: number) => void;
  setTestNote: (value: number) => void;
  setTestVelocity: (value: number) => void;
  setTestCc: (value: number) => void;
  setTestCcValue: (value: number) => void;
  onTestNote: () => void;
  onTestCc: () => void;
};

export function TestMidiCard(props: Props) {
  const {
    t,
    testChannel,
    testNote,
    testVelocity,
    testCc,
    testCcValue,
    setTestChannel,
    setTestNote,
    setTestVelocity,
    setTestCc,
    setTestCcValue,
    onTestNote,
    onTestCc,
  } = props;

  const controls: Array<{
    label: string;
    value: number;
    setter: (value: number) => void;
    min: number;
    max: number;
  }> = [
    { label: "routing.testChannel", value: testChannel, setter: setTestChannel, min: 1, max: 16 },
    { label: "routing.testNote", value: testNote, setter: setTestNote, min: 0, max: 127 },
    { label: "routing.testVelocity", value: testVelocity, setter: setTestVelocity, min: 0, max: 127 },
    { label: "routing.testCc", value: testCc, setter: setTestCc, min: 0, max: 127 },
    { label: "routing.testCcValue", value: testCcValue, setter: setTestCcValue, min: 0, max: 127 },
  ];

  return (
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
          {controls.map(({ label, value, setter, min, max }) => (
            <div key={label} className="space-y-1">
              <Label className="text-xs">{t(label)}</Label>
              <Input
                type="number"
                min={min}
                max={max}
                value={value}
                onChange={(event) => setter(Number(event.target.value || 0))}
              />
            </div>
          ))}
        </div>
        <div className="flex flex-wrap gap-2">
          <Button onClick={onTestNote}>{t("routing.sendTestNote")}</Button>
          <Button variant="outline" onClick={onTestCc}>
            {t("routing.sendTestCc")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
