import { beforeEach, expect, it, vi } from "vitest";
import type { AppConfig } from "./api-types";
import { getConfig, importConfig, resetConfigDefaults, saveConfig, startBridge } from "./api";
import { isAcceptedConfigReplacement } from "./config-persistence";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./api-client", () => ({ invokeCommand: invoke }));

function importedConfig(): AppConfig {
  return {
    ui: { developerMode: false },
    logging: { enabled: false, verbose: true, logAllToFile: true },
    osc: { logMessages: true }, rtp: { logMessages: true },
  } as AppConfig;
}

it("persists normalized imported logging before publishing the replacement", async () => {
  invoke.mockResolvedValue(importedConfig());
  const accepted = await importConfig("C:/config.yaml");
  expect(invoke).toHaveBeenNthCalledWith(2, "save_config", { config: accepted });
  expectNormal(accepted);
});

function expectNormal(config: AppConfig) {
  expect(config.logging).toEqual({ enabled: false, verbose: false, logAllToFile: false });
  expect(config.osc.logMessages).toBe(false);
  expect(config.rtp.logMessages).toBe(false);
}

beforeEach(() => vi.clearAllMocks());

it("normalizes loaded configuration without mutating the native response", async () => {
  const native = importedConfig();
  invoke.mockResolvedValue(native);
  expectNormal(await getConfig());
  expect(native.logging.verbose).toBe(true);
});

it("saves and starts with the same diagnostic policy", async () => {
  invoke.mockResolvedValue(undefined);
  const draft = importedConfig();
  await saveConfig(draft);
  await startBridge(draft);
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["save_config", "start_bridge"]);
  for (const [, args] of invoke.mock.calls) expectNormal(args.config);
  expect(draft.logging.verbose).toBe(true);
});

it.each(["import", "reset"])("marks the normalized %s snapshot as the accepted replacement", async (operation) => {
  invoke.mockResolvedValue(importedConfig());
  const accepted = operation === "import" ? await importConfig("C:/config.yaml") : await resetConfigDefaults();
  expectNormal(accepted);
  expect(isAcceptedConfigReplacement(accepted)).toBe(true);
});
