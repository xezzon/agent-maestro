import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import eslintConfigPrettier from "eslint-config-prettier/flat";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

// 前端静态检查：纯 JS/JSX，不引入 TypeScript。规则集刻意保持最小——核心
// recommended 抓明显错误，react-hooks 抓 hooks 调用与依赖数组的违规；格式交给
// Prettier，故最后并入 eslint-config-prettier 关掉与之冲突的规则。
export default defineConfig([
  {
    // Rust 侧（src-tauri、crates、plugins）由 clippy 与 rustfmt 负责，不进 ESLint；
    // .scratch 放临时产物，不参与检查。
    ignores: [
      "dist/",
      "node_modules/",
      ".scratch/",
      ".agents/",
      "src-tauri/",
      "crates/",
      "plugins/",
      "docs/",
    ],
  },
  {
    files: ["**/*.{js,jsx}"],
    extends: [js.configs.recommended],
    languageOptions: {
      ecmaVersion: "latest",
      sourceType: "module",
      globals: globals.browser,
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      // 显式写出本条：它是验收要求，不能靠 @eslint/js 的 recommended 隐式带入。
      "no-unused-vars": "error",
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "error",
    },
  },
  {
    // 构建与测试配置跑在 Node 上（process、__dirname 等）。
    files: ["*.config.js"],
    languageOptions: { globals: globals.node },
  },
  eslintConfigPrettier,
]);
