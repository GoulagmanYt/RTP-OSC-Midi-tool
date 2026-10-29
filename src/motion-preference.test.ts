import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { applyMotionPreference, readMotionPreference, setMotionPreference } from "./motion-preference";

beforeEach(() => {
  localStorage.clear();
  document.documentElement.classList.remove("reduce-motion");
});

afterEach(() => {
  vi.restoreAllMocks();
  document.documentElement.classList.remove("reduce-motion");
});

it("restores the motion preference before showing the interface", () => {
  setMotionPreference(true);
  document.documentElement.classList.remove("reduce-motion");
  applyMotionPreference(readMotionPreference());
  expect(document.documentElement.classList.contains("reduce-motion")).toBe(true);
  setMotionPreference(false);
  expect(readMotionPreference()).toBe(false);
  expect(document.documentElement.classList.contains("reduce-motion")).toBe(false);
});

it("still reduces motion in the current session when storage is unavailable", () => {
  vi.spyOn(localStorage, "setItem").mockImplementation(() => { throw new Error("unavailable"); });
  expect(() => setMotionPreference(true)).not.toThrow();
  expect(document.documentElement.classList.contains("reduce-motion")).toBe(true);
  expect(readMotionPreference()).toBe(true);
  setMotionPreference(false);
});
