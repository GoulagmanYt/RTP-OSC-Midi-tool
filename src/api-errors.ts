import type { CommandError } from "./api-types";

export class CommandFailure extends Error {
  readonly code: string;
  readonly domain: string;
  readonly details: Record<string, string>;

  constructor(error: CommandError) {
    super(error.message);
    this.name = "CommandFailure";
    this.code = error.code;
    this.domain = error.domain;
    this.details = error.details ?? {};
  }
}

export function isCommandError(value: unknown): value is CommandError {
  if (!value || typeof value !== "object") {
    return false;
  }
  const candidate = value as Partial<CommandError>;
  return (
    typeof candidate.code === "string" &&
    typeof candidate.domain === "string" &&
    typeof candidate.message === "string"
  );
}

export function normalizeCommandError(error: unknown): CommandFailure {
  if (isCommandError(error)) {
    return new CommandFailure(error);
  }
  if (error instanceof Error) {
    return new CommandFailure({
      code: "frontend.invoke-error",
      domain: "frontend",
      message: error.message,
    });
  }
  return new CommandFailure({
    code: "frontend.unknown-error",
    domain: "frontend",
    message: String(error),
  });
}
