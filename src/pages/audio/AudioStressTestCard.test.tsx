import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { StressTestResult } from "../../api-types";
import { AudioStressTestCard } from "./AudioStressTestCard";

const mocks = vi.hoisted(() => ({ runTest: vi.fn() }));
vi.mock("../../api", () => ({ runAutomatedStressTest: mocks.runTest }));

describe("AudioStressTestCard", () => {
  it("hides stress modes that do not exercise a real RTP input", () => {
    render(<AudioStressTestCard bridgeRunning audioRunning t={(key) => key} />);

    expect(screen.queryByRole("button", { name: /RTP Input/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /End-to-End/i })).toBeNull();
  });

  it("keeps completed measurements labelled with the mode that produced them", async () => {
    mocks.runTest.mockResolvedValueOnce({
      sentNotes: 25_000, droppedNotes: 0, xruns: 0, elapsedMs: 5000,
    } satisfies StressTestResult);
    render(<AudioStressTestCard bridgeRunning audioRunning t={(key) => key} />);

    fireEvent.click(screen.getByRole("button", { name: "stress.run" }));
    const results = await screen.findByText("stress.results");
    expect(mocks.runTest).toHaveBeenCalledWith(5000, 5, "bridge");
    fireEvent.click(screen.getByRole("button", { name: "stress.mode.audio" }));

    expect(results.parentElement?.textContent).toContain("stress.mode.bridge");
    expect(results.parentElement?.textContent).not.toContain("stress.mode.audio");

    mocks.runTest.mockResolvedValueOnce({
      sentNotes: 25_000, droppedNotes: 1, xruns: 0, elapsedMs: 5000,
    } satisfies StressTestResult);
    fireEvent.click(screen.getByRole("button", { name: "stress.run" }));
    const nextResults = await screen.findByText("stress.results");
    expect(nextResults.parentElement?.textContent).toContain("stress.mode.audio");
  });
});
