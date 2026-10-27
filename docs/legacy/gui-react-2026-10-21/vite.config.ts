import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri 开发时固定端口，生产构建输出到 dist
export default defineConfig({
  plugins: [react()],
  // Tauri 以自定义协议加载 dist，需相对路径 base
  base: "./",
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "es2021",
  },
  test: {
    environment: "happy-dom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    css: false,
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
