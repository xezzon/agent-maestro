import { defineConfig } from "vitest/config";

// 前端单元测试：只跑 `src/` 下的纯 JS 测试，运行在 node 环境
// （不引入 DOM / 组件渲染依赖）。与 `vite.config.js` 分开，避免测试继承
// Tauri 开发服务器的配置。
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.{test,spec}.js"],
  },
});
