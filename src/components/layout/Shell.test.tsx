import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import { Shell } from "./Shell";

const toggleBridge = vi.fn();
vi.mock("../../providers/BridgeProvider", () => ({
  useBridge: () => ({ config: { ui: { developerMode: false } }, status: { running: false }, isLoading: false, toggleBridge }),
}));
vi.mock("../../providers/LanguageProvider", () => ({
  useI18n: () => ({ t: (key: string) => key, language: "fr" }),
}));

const originalScrollIntoView = HTMLElement.prototype.scrollIntoView;
afterEach(() => {
  vi.unstubAllGlobals();
  HTMLElement.prototype.scrollIntoView = originalScrollIntoView;
});

it("lets keyboard users hide and restore the navigation without leaving the page", () => {
  render(<MemoryRouter><Shell /></MemoryRouter>);
  const toggle = screen.getByRole("button", { name: "topBar.toggleSidebar" });
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  fireEvent.click(toggle);
  expect(screen.queryByRole("navigation")).toBeNull();
  expect(screen.getByRole("main")).toBeTruthy();
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(toggle);
  expect(screen.getByRole("navigation")).toBeTruthy();
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
});

it("opens an accessibly named command dialog from the keyboard and lets Escape close it", async () => {
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    unobserve() {}
    disconnect() {}
  });
  HTMLElement.prototype.scrollIntoView = vi.fn();
  render(<MemoryRouter><Shell /></MemoryRouter>);
  fireEvent.keyDown(document, { key: "k", ctrlKey: true });
  const dialog = await screen.findByRole("dialog", { name: "command.title" });
  fireEvent.keyDown(dialog, { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});
