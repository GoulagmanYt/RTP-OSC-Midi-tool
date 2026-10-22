import { fireEvent, render, screen, waitFor, act } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { WindowControls } from "./WindowControls";

const mocks = vi.hoisted(() => ({
  minimize: vi.fn(), close: vi.fn(), toggleMaximize: vi.fn(), isMaximized: vi.fn(),
  onResized: vi.fn(), release: vi.fn(), error: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/webviewWindow", () => ({ getCurrentWebviewWindow: () => mocks }));
vi.mock("../../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("sonner", () => ({ toast: { error: mocks.error } }));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.isMaximized.mockResolvedValue(false);
  mocks.onResized.mockResolvedValue(mocks.release);
  mocks.toggleMaximize.mockResolvedValue(undefined);
});

it("reflects native maximize and restore changes and releases its listener", async () => {
  const view = render(<WindowControls />);
  await waitFor(() => expect(mocks.onResized).toHaveBeenCalled());
  mocks.isMaximized.mockResolvedValue(true);
  fireEvent.click(screen.getByRole("button", { name: "window.maximize" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "window.restore" })).toBeTruthy());
  expect(mocks.toggleMaximize).toHaveBeenCalledOnce();
  mocks.isMaximized.mockResolvedValue(false);
  await act(async () => { await mocks.onResized.mock.calls[0][0](); });
  expect(screen.getByRole("button", { name: "window.maximize" })).toBeTruthy();
  view.unmount();
  expect(mocks.release).toHaveBeenCalledOnce();
});

it("sends minimize and close to the native window", async () => {
  render(<WindowControls />);
  fireEvent.click(screen.getByRole("button", { name: "window.minimize" }));
  fireEvent.click(screen.getByRole("button", { name: "window.close" }));
  await waitFor(() => expect(mocks.close).toHaveBeenCalledOnce());
  expect(mocks.minimize).toHaveBeenCalledOnce();
});
