import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import OscPage from "./OscPage";

const mocks = vi.hoisted(() => ({
  updateConfig: vi.fn(),
  saveConfig: vi.fn(),
  config: {
    osc: {
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
    status: {},
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

it("resets draft to original value when blurred with an invalid port", () => {
  render(<OscPage />);
  const targetPortInput = screen.getByPlaceholderText("9000") as HTMLInputElement;

  fireEvent.change(targetPortInput, { target: { value: "999999" } });
  fireEvent.blur(targetPortInput);

  expect(targetPortInput.value).toBe("9000");
  expect(mocks.updateConfig).not.toHaveBeenCalled();
});
