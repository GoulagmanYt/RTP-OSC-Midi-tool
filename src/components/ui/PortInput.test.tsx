import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { PortInput } from "./PortInput";

it("keeps an invalid draft visible without applying it and explains the valid range", () => {
  const commit = vi.fn();
  render(<PortInput id="port" value={9000} min={1} max={65535} onCommit={commit} t={(_, values) => `Port ${values?.min}–${values?.max}`} />);
  const input = screen.getByRole("spinbutton") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "0" } });
  fireEvent.blur(input);
  expect(input.value).toBe("0");
  expect(input.getAttribute("aria-invalid")).toBe("true");
  expect(input.getAttribute("aria-describedby")).toBe(screen.getByRole("alert").id);
  expect(screen.getByRole("alert").textContent).toContain("1–65535");
  expect(commit).not.toHaveBeenCalled();
  fireEvent.change(input, { target: { value: "8000" } });
  fireEvent.keyDown(input, { key: "Enter" });
  fireEvent.blur(input);
  expect(commit).toHaveBeenCalledExactlyOnceWith(8000);
  expect(screen.queryByRole("alert")).toBeNull();
});
