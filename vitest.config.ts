import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    environment: "jsdom",
    // Avoid starving lazy-route integration tests when the Windows build runs the full suite.
    maxWorkers: 2,
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
  },
});
