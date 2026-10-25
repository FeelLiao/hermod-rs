import { defineConfig } from "vite";
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    target: "es2021",
    rollupOptions: {
      // 桌面客户端没有服务端组件；依赖中的指令对本构建无效。
      onwarn(warning, warn) {
        if (
          warning.code === "MODULE_LEVEL_DIRECTIVE" &&
          warning.message.includes("use client")
        )
          return;
        warn(warning);
      },
      output: {
        manualChunks(id) {
          if (id.includes("node_modules"))
            return id.includes("@material") || (id.includes("/node_modules/lit") || id.includes("/node_modules/@lit/")) ? "material" : id.includes("@fluentui") || id.includes("@griffel")
              ? "fluent"
              : "vendor";
        },
      },
    },
  },
});
