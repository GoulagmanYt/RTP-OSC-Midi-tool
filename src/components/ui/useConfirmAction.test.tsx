import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { useConfirmAction } from "./useConfirmAction";
vi.mock("../../providers/LanguageProvider", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
it("requires the named destructive action and allows cancellation", async () => {
  const action = vi.fn();
  function Harness() {
    const { confirmAction, confirmation } = useConfirmAction(key => key);
    return <><button onClick={() => confirmAction("Delete profile", action)}>Delete</button>{confirmation}</>;
  }
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(screen.getByRole("dialog", { name: "Delete profile" })).toBeTruthy();
  expect(action).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "ui.cancel" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(action).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  fireEvent.click(screen.getByRole("button", { name: "Delete profile" }));
  await waitFor(() => expect(action).toHaveBeenCalledOnce());
});
