import React from "react";
import ReactDOM from "react-dom/client";
import { ConfigProvider } from "antd";
import zhCN from "antd/locale/zh_CN";
import { error as logError } from "@tauri-apps/plugin-log";
import App from "./App";

// 未捕获异常汇入统一日志文件（ADR 0008）：经 @tauri-apps/plugin-log 落盘，
// 记录 target 为 webview::<location>。不整包转发 console.*（噪音控制，且
// console.log(provider) 会把 api_key 写进日志文件）；命令失败不在这里重复
// 记录——Rust 命令层已经记了。
function formatReason(reason) {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.stack || `${reason.name}: ${reason.message}`;
  // event.reason 可能是对象或循环引用：String() 兜不住时以 JSON.stringify 兜底，
  // 防止兜底处理器自己抛出。
  try {
    return JSON.stringify(reason);
  } catch {
    return String(reason);
  }
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
