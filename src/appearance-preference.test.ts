import { afterEach, expect, it } from "vitest";
import { initialAppearance, rememberAppearance } from "./appearance-preference";
afterEach(() => localStorage.clear());
it("follows the system for the legacy unmarked light default", () => {
  expect(initialAppearance("light")).toBe("system");
  expect(initialAppearance(undefined)).toBe("system");
  expect(initialAppearance("dark")).toBe("dark");
  expect(initialAppearance("contrast")).toBe("contrast");
});
it("keeps system appearance after a reset returns the legacy native light default", () => {
  rememberAppearance("light");
  rememberAppearance("system");
  expect(initialAppearance("light")).toBe("system");
});
it("preserves explicit and imported light choices once accepted", () => {
  rememberAppearance("light");
  expect(initialAppearance("light")).toBe("light");
});
