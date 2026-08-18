import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

/** 错误边界：渲染崩溃时显示错误文本，避免整页白屏且无任何提示 */
class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[mg] render error:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 24, fontFamily: "monospace", fontSize: 13 }}>
          <h2>渲染出错（Marguerite GUI）</h2>
          <pre style={{ whiteSpace: "pre-wrap", color: "#C2410C" }}>
            {String(this.state.error)}
          </pre>
          <pre style={{ whiteSpace: "pre-wrap", color: "#475569" }}>
            {this.state.error.stack}
          </pre>
          <button
            onClick={() => this.setState({ error: null })}
            style={{ marginTop: 12, padding: "8px 16px", borderRadius: 8, border: "1px solid #E2E8F0", background: "#F97316", color: "#fff", fontWeight: 700, cursor: "pointer" }}
          >
            重试渲染
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

// 全局捕获未处理错误，控制台可见
window.addEventListener("error", (e) => {
  console.error("[mg] uncaught:", e.message, e.error);
});
window.addEventListener("unhandledrejection", (e) => {
  console.error("[mg] unhandled rejection:", e.reason);
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
