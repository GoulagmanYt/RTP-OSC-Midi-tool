import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite config tuned for Tauri + React
export default defineConfig(() => ({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: ["es2021", "chrome107", "safari13"],
    minify: process.env.TAURI_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
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
