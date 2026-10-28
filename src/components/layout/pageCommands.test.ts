import { afterEach, expect, it, vi } from "vitest";
import { collectPageCommands, activatePageCommand } from "./pageCommands";
afterEach(() => { document.body.innerHTML = ""; });
it("reuses page actions and focuses labelled fields inside collapsed details", () => {
  document.body.innerHTML = '<main><button>Reset</button><button disabled>Unavailable</button><details><summary>Advanced</summary><label for="port">Port</label><input id="port"></details></main>';
  const clicked = vi.fn();
  document.querySelector("button")!.addEventListener("click", clicked);
  const commands = collectPageCommands();
  expect(commands.map(command => command.label)).toEqual(["Reset", "Advanced", "Port"]);
  activatePageCommand(commands[0]);
  expect(clicked).toHaveBeenCalledOnce();
  activatePageCommand(commands[2]);
  expect(document.querySelector("details")!.open).toBe(true);
  expect(document.activeElement?.id).toBe("port");
});
it("includes Radix switches labelled by an external label", () => {
  document.body.innerHTML = '<main><label for="osc-enabled">Enable OSC</label><button id="osc-enabled" role="switch"><span></span></button></main>';
  expect(collectPageCommands().map(command => command.label)).toEqual(["Enable OSC"]);
});
it("uses the active dialog as its command context", () => {
  document.body.innerHTML = '<main><button>Background</button></main><div role="dialog"><button>Reset EQ</button></div>';
  expect(collectPageCommands().map(command => command.label)).toEqual(["Reset EQ"]);
});
it("does not execute a command that became disabled or was removed", () => {
  document.body.innerHTML = "<main><button>Action</button></main>";
  const command = collectPageCommands()[0];
  command.element.setAttribute("disabled", "");
  expect(activatePageCommand(command)).toBe(false);
  command.element.remove();
  expect(activatePageCommand(command)).toBe(false);
});
