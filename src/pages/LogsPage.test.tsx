import { fireEvent, render, screen, within, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { BufferedLogEntry } from "../api";
import LogsPage from "./LogsPage";

const state = vi.hoisted(() => ({ logs: [] as BufferedLogEntry[], clearLogs: vi.fn() }));
vi.mock("../providers/BridgeProvider", () => ({ useBridge: () => state }));
vi.mock("../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string, vars?: Record<string, string | number>) => vars?.number === undefined ? key : `${key} ${vars.number}` }) }));
beforeEach(() => {
  state.logs = Array.from({ length: 1000 }, (_, index) => ({ sequence: 1000 - index, timestamp: String(index), level: index % 2 ? "warn" : "info", message: `message ${index}` }));
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
  state.logs = [{ sequence: 1001, timestamp: "new", message: "incoming", level: "error" }];
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
it("freezes the reading position when follow is disabled", () => {
  const { rerender } = render(<LogsPage />);
  fireEvent.click(screen.getByRole("checkbox", { name: "ui.logs.autoscroll" }));
  state.logs = [{ sequence: 1001, timestamp: "next", level: "error", message: "new arrival" }, ...state.logs];
  rerender(<LogsPage />);
  expect(screen.queryByText("new arrival")).toBeNull();
  expect(screen.getByText("message 0")).toBeTruthy();
  expect(screen.getByRole("button", { name: "ui.logs.resume" })).toBeTruthy();
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
  state.logs = [{ sequence: 1001, timestamp: "next", level: "error", message: "replacement" }];
  rerender(<LogsPage />);
  fireEvent.click(screen.getByRole("button", { name: "ui.logs.copySelection" }));
  expect(writeText).toHaveBeenCalledWith("0 [info] message 0");
});

it("uses arrival sequence labels even when filtering reveals a previously virtualized row", () => {
  const { unmount } = render(<LogsPage />);
  expect(screen.getByRole("checkbox", { name: "ui.logs.selectLine 1000" })).toBeTruthy();
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "message 999" } });
  expect(screen.getByRole("checkbox", { name: "ui.logs.selectLine 1" })).toBeTruthy();

  unmount();
  render(<LogsPage />);
  expect(screen.getByRole("checkbox", { name: "ui.logs.selectLine 1000" })).toBeTruthy();
});

it.each(["button", "keyboard"])("copies selection chronologically via %s regardless of click order or timestamps", (method) => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  state.logs = [
    { sequence: 40, timestamp: "00:00", level: "info", message: "newest" },
    { sequence: 30, timestamp: "23:59", level: "warn", message: "middle" },
    { sequence: 20, timestamp: "12:00", level: "error", message: "oldest" },
  ];
  render(<LogsPage />);
  const region = screen.getByRole("log");
  const rows = within(region).getAllByRole("checkbox");
  fireEvent.click(rows[1]);
  fireEvent.click(rows[0]);
  fireEvent.click(rows[2]);
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "middle" } });
  if (method === "button") {
    fireEvent.click(screen.getByRole("button", { name: "ui.logs.copySelection" }));
  } else {
    fireEvent.keyDown(region, { key: "c", ctrlKey: true });
  }
  expect(writeText).toHaveBeenCalledWith("12:00 [error] oldest\n23:59 [warn] middle\n00:00 [info] newest");
});
