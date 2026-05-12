import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AudioStressTestCard } from "./AudioStressTestCard";

describe("AudioStressTestCard", () => {
  it("hides the unimplemented RTP stress mode", () => {
    render(<AudioStressTestCard bridgeRunning audioRunning />);

    expect(screen.queryByRole("button", { name: /RTP Input/i })).toBeNull();
  });
});
