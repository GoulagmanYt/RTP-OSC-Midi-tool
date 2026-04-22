import { describe, expect, it } from "vitest";

import { CommandFailure, normalizeCommandError } from "./api-errors";

describe("normalizeCommandError", () => {
  it("wraps typed backend errors into CommandFailure", () => {
    const failure = normalizeCommandError({
      code: "runtime.start-failed",
      domain: "runtime",
      message: "boom",
      details: {
        reason: "device-busy",
      },
    });

    expect(failure).toBeInstanceOf(CommandFailure);
    expect(failure.code).toBe("runtime.start-failed");
    expect(failure.domain).toBe("runtime");
    expect(failure.details.reason).toBe("device-busy");
  });

  it("converts unknown errors to frontend failures", () => {
    const failure = normalizeCommandError("fatal");

    expect(failure.code).toBe("frontend.unknown-error");
    expect(failure.domain).toBe("frontend");
    expect(failure.message).toBe("fatal");
  });
});
