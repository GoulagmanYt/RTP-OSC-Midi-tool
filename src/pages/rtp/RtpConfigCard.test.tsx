import { useState } from "react";
import { fireEvent, render, screen, within, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import type { AppConfig, RtpRemoteEntry } from "../../api-types";
import { RtpConfigCard } from "./RtpConfigCard";

vi.mock("../../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));

function mount() {
  const commit = vi.fn();
  const noop = vi.fn(async () => undefined);
  function Harness() {
    const [port, setPort] = useState(5004);
    const [remotePort, setRemotePort] = useState(5006);
    return <RtpConfigCard
      busy={false} discovering={false} remoteEnabled
      config={{ rtp: { port }, logging: { verbose: false } } as AppConfig}
      remotes={[{ id: "keyboard", name: "Keyboard", host: "localhost", port: remotePort, autoConnect: false } as RtpRemoteEntry]}
      sessions={[]} t={(key) => key}
      isRemoteConnected={() => false} isSelectedSession={() => false}
      onAddRemote={noop} onConnect={noop} onRefreshSessions={noop} onRemoveRemote={noop}
      onSave={noop} onToggleRtp={noop}
      onUpdateConfig={async (patch) => { commit(patch); if (patch.rtp?.port !== undefined) setPort(patch.rtp.port); }}
      onUpdateRemote={async (id, patch) => { commit(id, patch); if (patch.port !== undefined) setRemotePort(patch.port); }}
    />;
  }
  render(<Harness />);
  return commit;
}

it.each(["rtp.portLabel", "rtp.favoritePort"])("allows replacing %s digit by digit before committing", (label) => {
  const commit = mount();
  const input = screen.getByLabelText(label) as HTMLInputElement;
  fireEvent.focus(input);
  for (const value of ["500", "", "6", "60", "600", "6008"]) {
    fireEvent.change(input, { target: { value } });
    expect(input.value).toBe(value);
  }
  expect(commit).not.toHaveBeenCalled();
  fireEvent.blur(input);
  expect(commit).toHaveBeenCalledExactlyOnceWith(...(label === "rtp.portLabel"
    ? [{ rtp: { port: 6008 } }]
    : ["keyboard", { port: 6008 }]));
  expect((screen.getByLabelText(label) as HTMLInputElement).value).toBe("6008");
});

it.each(["", "500", "65535", "5004.5"])("retains invalid draft %s without applying it", (value) => {
  const commit = mount();
  const input = screen.getByLabelText("rtp.portLabel") as HTMLInputElement;
  fireEvent.change(input, { target: { value } });
  fireEvent.blur(input);
  expect(input.value).toBe(value);
  expect(screen.getByRole("alert").textContent).toBe("ui.portRange");
  expect(commit).not.toHaveBeenCalled();
});

it("commits with Enter and resets only after confirmation", async () => {
  const commit = mount();
  const input = screen.getByLabelText("rtp.portLabel") as HTMLInputElement;
  input.focus();
  fireEvent.change(input, { target: { value: "6010" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(commit).toHaveBeenCalledWith({ rtp: { port: 6010 } });
  fireEvent.click(screen.getByRole("button", { name: "ui.resetRtp" }));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "ui.resetRtp" }));
  await waitFor(() => expect((screen.getByLabelText("rtp.portLabel") as HTMLInputElement).value).toBe("5004"));
});
