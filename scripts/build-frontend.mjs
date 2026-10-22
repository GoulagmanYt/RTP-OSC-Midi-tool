import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const windows = process.platform === "win32";
const child = windows && process.env.OSCMIDI_FRONTEND_NODE_CHILD === "1";
const viaPowerShell = windows && !child;
const args = child ? JSON.parse(process.env.OSCMIDI_VITE_ARGS || "[]") : process.argv.slice(2);
const env = { ...process.env, OSCMIDI_VITE_ARGS: JSON.stringify(args) };
// Windows PowerShell must rebuild its own module path when launched through
// Node from pwsh; it cannot load pwsh 7's Microsoft.PowerShell.Utility module.
if (viaPowerShell) {
  for (const key of Object.keys(env)) {
    if (key.toLowerCase() === "psmodulepath") delete env[key];
  }
}
const result = spawnSync(
  viaPowerShell ? "powershell.exe" : process.execPath,
  viaPowerShell
    ? ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", fileURLToPath(new URL("./build-frontend.ps1", import.meta.url))]
    : [fileURLToPath(new URL("../node_modules/vite/bin/vite.js", import.meta.url)), "build", ...args],
  { stdio: "inherit", windowsHide: true, env },
);
if (result.error) console.error(result.error.message);
process.exit(result.status ?? 1);
