import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import LogsPage from "./LogsPage";

const clearLogs = vi.fn();
vi.mock("../providers/BridgeProvider", () => ({ useBridge: () => ({ logs: [], clearLogs }) }));
vi.mock("../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));

it("names the log region and clears only the view without repeating the shell title", () => {
  render(<LogsPage />);
  expect(screen.getByRole("region", { name: "logs.entries" })).toBeTruthy();
  expect(screen.queryByRole("heading", { name: "logs.title" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "logs.clear" }));
  expect(clearLogs).toHaveBeenCalledOnce();
});
