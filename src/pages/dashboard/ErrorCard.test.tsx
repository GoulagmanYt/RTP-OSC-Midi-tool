import { render, screen, within } from "@testing-library/react";
import { expect, it } from "vitest";
import { ErrorCard } from "./ErrorCard";

it("keeps an actionable alert while removing retained internal errors when developer mode is disabled", () => {
  const error = "worker IPC C:\\internal\\plugin.dll callback failed 0x123";
  const t = (key: string) => key;
  const { rerender } = render(<ErrorCard error={error} developerMode t={t} />);
  expect(within(screen.getByRole("alert")).getByText(error)).toBeTruthy();
  rerender(<ErrorCard error={error} developerMode={false} t={t} />);
  expect(screen.queryByText(error)).toBeNull();
  expect(within(screen.getByRole("alert")).getByText("dashboard.errorRecoveryHint")).toBeTruthy();
  rerender(<ErrorCard error={error} developerMode t={t} />);
  expect(within(screen.getByRole("alert")).getByText(error)).toBeTruthy();
});
