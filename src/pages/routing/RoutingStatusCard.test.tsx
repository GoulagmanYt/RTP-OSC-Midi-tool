import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { RuntimeStatus } from "../../api-types";
import { RoutingStatusCard } from "./RoutingStatusCard";

describe("routing activity", () => {
  it.each([
    { running: true, age: 0, active: true },
    { running: false, age: 0, active: false },
    { running: undefined, age: 0, active: false },
    { running: true, age: 2000, active: false },
  ])("shows active=$active for running=$running and age=$age", ({ running, age, active }) => {
    render(<RoutingStatusCard
      config={null}
      status={running === undefined ? null : { running } as RuntimeStatus}
      activity={[{ source: "Keyboard", messagesPerSec: 10, lastSeenMs: Date.now() - age }]}
      t={(key) => key}
      displayPortName={(name) => name}
      formatNote={() => "--"}
    />);

    expect(screen.getByText(active ? "ui.activityActive" : "ui.activityInactive")).toBeTruthy();
    expect(screen.queryByText(active ? "ui.activityInactive" : "ui.activityActive")).toBeNull();
  });
});
