import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createDefaultVstEqSettings } from "./eq";
import { VstEqDialog } from "./VstEqDialog";

vi.mock("../../providers/LanguageProvider", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

describe("VstEqDialog", () => {
  it("shows the selected plugin, response graph, live bypass and reset controls", () => {
    const onChange = vi.fn();
    const onReset = vi.fn();
    const settings = createDefaultVstEqSettings();
    const t = (key: string, values?: Record<string, string | number>) =>
      values?.plugin ? `${key}:${values.plugin}` : key;

    render(
      <VstEqDialog
        open
        pluginName="Concert Piano"
        settings={settings}
        sampleRate={48_000}
        t={t}
        onOpenChange={vi.fn()}
        onChange={onChange}
        onReset={onReset}
      />
    );

    expect(screen.getByText("audio.eq.description:Concert Piano")).toBeTruthy();
    expect(screen.getByRole("img", { name: "audio.eq.responseGraph" })).toBeTruthy();
    fireEvent.click(screen.getByRole("switch"));
    expect(onChange).toHaveBeenCalledWith({ ...settings, enabled: true });
    fireEvent.change(screen.getAllByLabelText("audio.eq.gain")[0], { target: { value: "-99" } });
    expect(onChange).toHaveBeenCalledWith({
      ...settings,
      lowShelf: { ...settings.lowShelf, gainDb: -18 },
    });
    fireEvent.click(screen.getByRole("button", { name: /audio.eq.reset/ }));
    expect(onReset).toHaveBeenCalledOnce();
  });
});
