import React from "react";
import ReactDOM from "react-dom/client";
import { ConfigProvider } from "antd";
import zhCN from "antd/locale/zh_CN";
import { error as logError, warn, info, debug, trace } from "@tauri-apps/plugin-log";
import App from "./App";

// 前端日志汇入统一日志文件（ADR 0008）：console.* 经 @tauri-apps/plugin-log
// 落盘（记录 target 为 webview::<location>）；未捕获异常另由下面两个监听兜住
// ——浏览器对未捕获错误的内部报告不走 console 对象，转发覆盖不到。命令失败
// 不在这里重复记录——Rust 命令层已经记了。
function formatReason(reason) {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.stack || `${reason.name}: ${reason.message}`;
  // 可能是对象或循环引用：String() 兜不住时以 JSON.stringify 兜底，防止兜底
  // 处理器自己抛出；JSON.stringify 对 undefined 与函数返回 undefined，须回落。
  try {
    return JSON.stringify(reason) ?? String(reason);
  } catch {
    return String(reason);
  }
}

// console.* 转发（级别映射同 Tauri plugin-log 官方示例）：原生输出照旧，同时
// 落盘。log 与 debug 映射到 trace/debug，默认级别 INFO 下不落盘，console.log
// 的对象转储不会进文件；console.info/warn/error 会落盘，不得用来打印凭证。
const consoleForwarders = { log: trace, debug, info, warn, error: logError };
for (const [fnName, logger] of Object.entries(consoleForwarders)) {
  const original = console[fnName];
  console[fnName] = (...args) => {
    original.apply(console, args);
    // plugin-log 只接受 string，传对象会让 invoke 拒绝；落盘失败的 promise 就地
    // 吞掉，否则未处理拒绝会再次进入下面的 unhandledrejection 监听。
    void logger(args.map(formatReason).join(" ")).catch(() => {});
  };
}

// `error()` 返回 promise：落盘失败必须就地吞掉，否则未处理拒绝会再次进入
// `unhandledrejection` 监听、反复调用同一个正在失败的 logger。
window.addEventListener("error", (event) => {
  void logError(formatReason(event.error ?? event.message)).catch(() => {});
});
window.addEventListener("unhandledrejection", (event) => {
  void logError(formatReason(event.reason)).catch(() => {});
});

ReactDOM.createRoot(document.getElementById("root")).render(
  <React.StrictMode>
    <ConfigProvider locale={zhCN}>
      <App />
    </ConfigProvider>
  </React.StrictMode>,
);
