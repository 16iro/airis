import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Vite 옵션 — Tauri 개발 시에만 의미 있음
  //
  // 1. Vite가 Rust 에러를 가리지 않도록.
  clearScreen: false,
  // 2. Tauri는 고정 포트 사용. 점유돼 있으면 즉시 실패.
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. Vite는 src-tauri/ 변화를 무시 (Cargo가 감시).
      ignored: ["**/src-tauri/**"],
    },
  },
}));
