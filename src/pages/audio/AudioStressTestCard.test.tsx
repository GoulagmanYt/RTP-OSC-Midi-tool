import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AudioStressTestCard } from "./AudioStressTestCard";

describe("AudioStressTestCard", () => {
  it("hides stress modes that do not exercise a real RTP input", () => {
    render(<AudioStressTestCard bridgeRunning audioRunning />);

    expect(screen.queryByRole("button", { name: /RTP Input/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /End-to-End/i })).toBeNull();
  });
});
