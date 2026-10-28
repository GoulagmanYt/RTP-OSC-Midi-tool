import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { MeterRow } from "./shared";

it.each([[0.4, 0.4], [-1, 0], [2, 1], [NaN, 0]])("exposes a named bounded meter for %s", (value, expected) => {
  render(<MeterRow label="Audio left" value={value} max={1} valueLabel="40%" />);
  const meter = screen.getByRole("meter", { name: "Audio left" });
  expect(meter.getAttribute("aria-valuenow")).toBe(String(expected));
  expect(meter.getAttribute("aria-valuemin")).toBe("0");
  expect(meter.getAttribute("aria-valuemax")).toBe("1");
  expect(meter.getAttribute("aria-valuetext")).toBe("40%");
});
