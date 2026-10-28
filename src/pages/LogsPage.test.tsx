import { fireEvent, render, screen, within, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { LogEntry } from "../api";
import LogsPage from "./LogsPage";

const state = vi.hoisted(() => ({ logs: [] as LogEntry[], clearLogs: vi.fn() }));
vi.mock("../providers/BridgeProvider", () => ({ useBridge: () => state }));
vi.mock("../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
beforeEach(() => {
  state.logs = Array.from({ length: 1000 }, (_, index) => ({ timestamp: String(index), level: index % 2 ? "warn" : "info", message: `message ${index}` }));
  state.clearLogs.mockClear();
});
it("virtualizes the buffer and leaves live announcements off", () => {
  render(<LogsPage />);
  const region = screen.getByRole("log", { name: "logs.entries" });
  expect(region.getAttribute("aria-live")).toBe("off");
  expect(region.querySelectorAll("[data-log-row]").length).toBeLessThan(30);
  expect(screen.queryByRole("heading", { name: "logs.title" })).toBeNull();
});
it("pauses incoming display updates and filters by level and search", () => {
  const { rerender } = render(<LogsPage />);
  fireEvent.click(screen.getByRole("button", { name: "ui.logs.pause" }));
  state.logs = [{ timestamp: "new", message: "incoming", level: "error" }];
  rerender(<LogsPage />);
  expect(screen.queryByText("incoming")).toBeNull();
  fireEvent.change(screen.getByRole("combobox", { name: "ui.logs.level" }), { target: { value: "warn" } });
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "message 999" } });
  expect(screen.getByText("message 999")).toBeTruthy();
  expect(screen.queryByText("message 998")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "ui.logs.resume" }));
  fireEvent.change(screen.getByRole("combobox"), { target: { value: "all" } });
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "" } });
  expect(screen.getByText("incoming")).toBeTruthy();
});
it("confirms clearing and preserves the buffer on cancel", async () => {
  render(<LogsPage />);
  fireEvent.click(screen.getByRole("button", { name: "ui.clearLogView" }));
  expect(state.clearLogs).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "ui.cancel" }));
  expect(state.clearLogs).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "ui.clearLogView" }));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "ui.clearLogView" }));
  await waitFor(() => expect(state.clearLogs).toHaveBeenCalledOnce());
});
it("copies a stable selected entry while incoming data changes", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  const { rerender } = render(<LogsPage />);
  fireEvent.click(within(screen.getByRole("log")).getAllByRole("checkbox")[0]);
  state.logs = [{ timestamp: "next", level: "error", message: "replacement" }];
  rerender(<LogsPage />);
  fireEvent.click(screen.getByRole("button", { name: "ui.logs.copySelection" }));
  expect(writeText).toHaveBeenCalledWith("0 [info] message 0");
});
