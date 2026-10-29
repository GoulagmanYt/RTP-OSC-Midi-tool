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
      result.current.flushLogs();
    });

    expect(result.current.logs).toHaveLength(2);
    expect(result.current.logs[0]?.message).toBe("second");
    expect(result.current.logs[1]?.message).toBe("first");

    act(() => {
      result.current.clearLogs();
    });

    expect(result.current.logs).toEqual([]);
  });

  it("assigns arrival sequences before flushing without mutating or reusing incoming entries", () => {
    const { result } = renderHook(() => useLogs());
    const incoming = Object.freeze({ level: "info" as const, message: "repeated", timestamp: "same" });
    act(() => {
      result.current.appendLog(incoming);
      result.current.appendLog(incoming);
      result.current.flushLogs();
    });
    expect(result.current.logs).toEqual([
      { ...incoming, sequence: 2 },
      { ...incoming, sequence: 1 },
    ]);
    expect(incoming).not.toHaveProperty("sequence");
    expect(result.current.logs[0]).not.toBe(result.current.logs[1]);
  });

  it("keeps sequences increasing through ring buffer rollover and clearing", () => {
    const { result } = renderHook(() => useLogs());
    act(() => {
      for (let index = 0; index < 1002; index += 1) {
        result.current.appendLog({ level: "info", message: String(index), timestamp: "same" });
      }
      result.current.flushLogs();
    });
    expect(result.current.logs).toHaveLength(1000);
    expect(result.current.logs[0]).toMatchObject({ message: "1001", sequence: 1002 });
    expect(result.current.logs[999]).toMatchObject({ message: "2", sequence: 3 });

    act(() => {
      result.current.clearLogs();
      result.current.appendLog({ level: "info", message: "after clear", timestamp: "same" });
      result.current.flushLogs();
    });
    expect(result.current.logs).toEqual([{ level: "info", message: "after clear", timestamp: "same", sequence: 1003 }]);
  });
});
