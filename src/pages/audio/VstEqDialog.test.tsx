import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createDefaultVstEqSettings } from "./eq";
import { VstEqDialog } from "./VstEqDialog";

vi.mock("../../providers/LanguageProvider", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

describe("VstEqDialog", () => {
  it("names bypass and announces frequencies in Hz rather than slider coordinates", () => {
    render(<VstEqDialog open pluginName="Piano" settings={createDefaultVstEqSettings()}
      sampleRate={48000} t={(key) => key} onOpenChange={vi.fn()} onChange={vi.fn()} onReset={vi.fn()} />);
    expect(screen.getByRole("switch", { name: /audio.eq.enabled/ })).toBeTruthy();
    expect(screen.getAllByRole("slider", { name: "audio.eq.frequency" })[0].getAttribute("aria-valuetext")).toBe("120 units.hz");
    expect(screen.getByRole("img").querySelectorAll("text")).toHaveLength(0);
  });
  it("ends the graph at Nyquist when the active sample rate is low", () => {
    render(<VstEqDialog open pluginName="Piano" settings={createDefaultVstEqSettings()}
      sampleRate={8_000} t={(key) => key} onOpenChange={vi.fn()}
      onChange={vi.fn()} onReset={vi.fn()} />);
    const graph = screen.getByRole("img");
    expect(graph.textContent).not.toContain("20k");
    expect(graph.textContent).toContain("4k");
  });
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
