import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// WebView2 is Chromium-based and kept current by the installer, so targeting
// Chrome avoids shipping Safari/Firefox transforms that desktop users never run.
export default defineConfig(() => ({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "chrome111",
    minify: process.env.TAURI_DEBUG ? false : "oxc",
    sourcemap: !!process.env.TAURI_DEBUG,
    rolldownOptions: {
      checks: {
        pluginTimings: false,
      },
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) {
            return undefined;
          }
          if (id.includes("react-router-dom")) {
            return "router";
          }
          if (id.includes("react-dom")) {
            return "react-dom";
          }
          if (id.includes("\\react\\") || id.includes("/react/")) {
            return "react-core";
          }
          if (id.includes("@tauri-apps")) {
            return "tauri";
          }
          if (id.includes("cmdk") || id.includes("@radix-ui/react-dialog")) {
            return "ui-command";
          }
          if (id.includes("@radix-ui/react-select")) {
            return "ui-select";
          }
          if (id.includes("@radix-ui")) {
            return "ui-primitives";
          }
          if (
            id.includes("lucide-react") ||
            id.includes("sonner") ||
            id.includes("clsx") ||
            id.includes("class-variance-authority") ||
            id.includes("tailwind-merge")
          ) {
            return "ui-vendor";
          }
          return "vendor";
        },
      },
    },
  },
}));
