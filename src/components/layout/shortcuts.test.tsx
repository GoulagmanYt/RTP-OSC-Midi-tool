import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { expect, it, vi } from "vitest";
import { Shell } from "./Shell";
const toggleBridge = vi.fn();
vi.mock("../../providers/BridgeProvider", () => ({
  useBridge: () => ({ config: { ui: { developerMode: false } }, status: {}, isLoading: false, toggleBridge }),
}));
vi.mock("../../providers/LanguageProvider", () => ({
  useI18n: () => ({ t: (key: string) => key, language: "fr" }),
}));
it("supports Windows shortcuts without starting the bridge while editing or confirming", () => {
  render(<MemoryRouter><Shell /></MemoryRouter>);
  fireEvent.keyDown(document, { key: "b", ctrlKey: true });
  expect(screen.queryByRole("navigation")).toBeNull();
  fireEvent.keyDown(document, { key: "b", ctrlKey: true });
  expect(screen.getByRole("navigation")).toBeTruthy();
  const input = document.createElement("input"); document.body.append(input);
  fireEvent.keyDown(input, { key: "Enter", ctrlKey: true });
  expect(toggleBridge).not.toHaveBeenCalled();
  input.remove();
  const modal = document.createElement("div"); modal.setAttribute("role", "dialog"); document.body.append(modal);
  fireEvent.keyDown(document, { key: "Enter", ctrlKey: true });
  expect(toggleBridge).not.toHaveBeenCalled();
  modal.remove();
  fireEvent.keyDown(document, { key: "Enter", ctrlKey: true });
  expect(toggleBridge).toHaveBeenCalledOnce();
  fireEvent.keyDown(document, { key: ",", ctrlKey: true });
  expect(screen.getByRole("heading", { name: "topBar.settings" })).toBeTruthy();
});
