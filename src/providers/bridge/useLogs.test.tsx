import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useLogs } from "./useLogs";

describe("useLogs", () => {
  it("prepends newest log entries and clears the buffer", () => {
    const { result } = renderHook(() => useLogs());

    act(() => {
      result.current.appendLog({
        level: "info",
        message: "first",
        timestamp: "10:00:00.000",
      });
      result.current.appendLog({
        level: "warn",
        message: "second",
        timestamp: "10:00:01.000",
      });
    });

    expect(result.current.logs).toHaveLength(2);
    expect(result.current.logs[0]?.message).toBe("second");
    expect(result.current.logs[1]?.message).toBe("first");

    act(() => {
      result.current.clearLogs();
    });

    expect(result.current.logs).toEqual([]);
  });
});
