import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vitest/config";

// 前端单元测试：只跑 `src/` 下的 `*.test.js`（纯 JS），运行在 node 环境
// （不引入 DOM / 组件渲染依赖）。与 `vite.config.js` 分开，避免测试继承
// Tauri 开发服务器的配置。
export default defineConfig({
  // `@/` 别名与 `vite.config.js` 同一份（两个配置互不继承，故各自声明）：
  // 被测模块之间的导入按应用里的写法走别名，测试才不会与应用分家。
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.js"],
  },
});
