import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import OscPage from "./OscPage";

const mocks = vi.hoisted(() => ({
  updateConfig: vi.fn(),
  saveConfig: vi.fn(),
  status: { running: false } as { running: boolean } | null,
  config: {
    osc: {
      enabled: true,
      targetIp: "127.0.0.1",
      targetPort: 9000,
      inputEnabled: false,
      listenIp: "127.0.0.1",
      listenPort: 9001,
      logMessages: false,
      bundleWait: 0,
    },
  },
  t: (key: string) => key,
}));

vi.mock("../providers/BridgeProvider", () => ({
  useBridge: () => ({
    config: mocks.config,
    status: mocks.status,
    updateConfig: mocks.updateConfig,
    saveConfig: mocks.saveConfig,
  }),
}));

vi.mock("../providers/LanguageProvider", () => ({
  useI18n: () => ({ t: mocks.t }),
}));

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.config.osc.enabled = true;
  mocks.status = { running: false };
});

it("distinguishes an enabled output from a running bridge without claiming connectivity", () => {
  const { rerender } = render(<OscPage />);
  expect(screen.getByText("common.stopped")).toBeTruthy();
  expect(screen.queryByText("osc.outputReady")).toBeNull();
  expect(screen.queryByText("osc.connected")).toBeNull();
  expect(screen.queryByText("osc.flowActive")).toBeNull();
  expect(screen.getByText("osc.deliveryHint")).toBeTruthy();

  mocks.status = { running: true };
  rerender(<OscPage />);
  expect(screen.getByText("osc.outputReady")).toBeTruthy();
  expect(screen.queryByText("common.stopped")).toBeNull();

  mocks.config.osc.enabled = false;
  rerender(<OscPage />);
  expect(screen.getByText("common.disabled")).toBeTruthy();
  expect(screen.queryByText("osc.outputReady")).toBeNull();
});

it("shows an unknown transmission state before runtime status is available", () => {
  mocks.status = null;
  render(<OscPage />);
  expect(screen.getByText("common.unknown")).toBeTruthy();
  expect(screen.queryByText("osc.outputReady")).toBeNull();
});

it("renders OSC configuration fields with current values", () => {
  render(<OscPage />);
  const targetPortInput = screen.getByPlaceholderText("9000") as HTMLInputElement;
  const listenPortInput = screen.getByPlaceholderText("9001") as HTMLInputElement;

  expect(targetPortInput.value).toBe("9000");
  expect(listenPortInput.value).toBe("9001");
});

it("allows clearing and typing a new target port, committing on blur", () => {
  render(<OscPage />);
  const targetPortInput = screen.getByPlaceholderText("9000") as HTMLInputElement;

  // Simulate user backspacing all characters to clear the input
  fireEvent.change(targetPortInput, { target: { value: "" } });
  expect(targetPortInput.value).toBe("");
  expect(mocks.updateConfig).not.toHaveBeenCalled();

  // Type new port and blur
  fireEvent.change(targetPortInput, { target: { value: "8000" } });
  expect(targetPortInput.value).toBe("8000");
  fireEvent.blur(targetPortInput);

  expect(mocks.updateConfig).toHaveBeenCalledWith({
    osc: { targetPort: 8000 },
  });
});

it("commits new port on Enter key press", () => {
  render(<OscPage />);
  const listenPortInput = screen.getByPlaceholderText("9001") as HTMLInputElement;

  fireEvent.change(listenPortInput, { target: { value: "7000" } });
  fireEvent.keyDown(listenPortInput, { key: "Enter" });

  expect(mocks.updateConfig).toHaveBeenCalledWith({
    osc: { listenPort: 7000 },
  });
});

it("retains an invalid port draft and displays correction guidance", () => {
  render(<OscPage />);
  const targetPortInput = screen.getByPlaceholderText("9000") as HTMLInputElement;

  fireEvent.change(targetPortInput, { target: { value: "999999" } });
  fireEvent.blur(targetPortInput);

  expect(targetPortInput.value).toBe("999999");
  expect(screen.getByRole("alert").textContent).toBe("ui.portRange");
  expect(mocks.updateConfig).not.toHaveBeenCalled();
});
