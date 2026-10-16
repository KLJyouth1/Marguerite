"""本地 mock OpenAI 兼容服务 —— 用于产出真实的端到端演示（截图/录屏）。

它**不是**产品的一部分，只服务于文档演示与端到端验证：让「没有真实 API Key」的机器
也能跑出一段真实经过引擎编排、真实渲染进 Web 控制台的对话。

用法：
    python scripts/demo/mock_llm.py [port]      # 默认 18999

支持：
    GET  /v1/models                        -> 模型列表（供 doctor --probe 校验）
    POST /v1/chat/completions             -> 非流式补全（**支持 OpenAI 兼容 tool_calls**）
    POST /v1/chat/completions (stream)    -> SSE 流式补全（含 tool_calls 增量）

MSC-1 端到端验证（Gap5）：当请求携带 `tools` 时，模型返回一个 **OpenAI 兼容的
`tool_calls`**（调用内置 `write_file` 真实落盘），使交付流水线 / 计划路径的
`.html` / `.pptx` 产物真实写到磁盘 —— 从而产物门禁（artifacts.list ≥2）可被客观核验，
而不是只靠文本声称。

目标文件判定：优先取「计划子步骤标题」里声明的产物（`output.html` /
`presentation.pptx`），避免被整体目标里的「html / PPT」同时命中所干扰；无产物声明的
步骤（检索 / 理解需求）返回纯文本。
"""
import json
import re
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18999

# 依次返回，形成一段有层次的演示对话（无工具时使用）
SCRIPT = [
    "我先把任务拆开：1) 扫描目录结构；2) 按扩展名归类；3) 生成迁移计划。"
    "现在开始第一步——读取当前目录，识别出 6 类文件。",
    "第 2 步完成：已按扩展名归类。文档 41 个、源码 12 个、配置 8 个、数据 3 个、图片 5 个、其余 7 个。",
    "第 3 步：迁移计划已生成，共 6 条规则。所有动作都是可回滚的——执行前会做一次快照。",
    "全部完成。目录已整理为 6 个子目录，原文件均可通过快照回滚。",
    "（演示回复：本次运行使用的是本地 mock 模型服务，未调用任何真实模型 API。）",
]

HTML_CONTENT = (
    "<!doctype html>\n<html lang=\"zh\">\n<head><meta charset=\"utf-8\">"
    "<title>今日 AI 新闻</title></head>\n<body>\n"
    "<h1>今日 AI 新闻（mock 端到端验证）</h1>\n"
    "<p>由本地 mock 模型真实落盘生成的交付物。</p>\n</body>\n</html>\n"
)
PPTX_CONTENT = (
    "PPTX(demo)::今日 AI 新闻演示 —— mock 端到端验证，真实落盘占位内容。"
)

# 已知交付物扩展名（与 core `plan::runner::step_target_paths` 对齐）
_ARTIFACT_RE = re.compile(r"[A-Za-z0-9_./\\-]+\.[A-Za-z0-9]{1,6}")
_ARTIFACT_EXT = {
    "html", "htm", "pptx", "ppt", "xlsx", "xls",
    "csv", "docx", "doc", "pdf", "md",
}
_STEP_TITLE_RE = re.compile(r"【计划(?:并行)?子步骤[^｜]*｜([^】]*)】")


def _messages_text(req):
    """拼接所有消息的可见文本（含 Anthropic 风格的 content blocks）。"""
    parts = []
    for m in req.get("messages", []) or []:
        content = m.get("content")
        if isinstance(content, str):
            parts.append(content)
        elif isinstance(content, list):
            for block in content:
                if isinstance(block, dict) and isinstance(block.get("text"), str):
                    parts.append(block["text"])
    return "\n".join(parts)


def _has_tool_result(req):
    """请求里是否已包含工具执行结果（说明上一轮已调用过工具）。"""
    return any(m.get("role") == "tool" for m in req.get("messages", []) or [])


def _step_title(text):
    m = _STEP_TITLE_RE.search(text)
    return m.group(1) if m else ""


def _declared_artifacts(text):
    """按已知扩展名提取声明的产物路径（保持出现顺序、去重）。"""
    out = []
    for m in _ARTIFACT_RE.finditer(text):
        tok = m.group(0)
        ext = tok.rsplit(".", 1)[-1].lower()
        if ext in _ARTIFACT_EXT and tok not in out:
            out.append(tok)
    return out


def _decide_tool_call(req):
    """返回 (path, content) 或 None：是否应触发一次 write_file。"""
    full = _messages_text(req)
    title = _step_title(full)
    # 优先步骤标题（避免整体目标里的「html / PPT」把所有步骤都带偏）
    scope = title or full
    artifacts = _declared_artifacts(scope)
    if not artifacts:
        return None
    path = artifacts[0]
    ext = path.rsplit(".", 1)[-1].lower()
    if ext in ("pptx", "ppt"):
        return path, PPTX_CONTENT
    if ext in ("html", "htm"):
        return path, HTML_CONTENT
    # 其它交付物：写一段占位内容，保证非空落盘
    return path, f"{path}（mock 端到端验证占位内容）\n"


def _completion_message(req):
    """构造一次补全的 assistant message（可能是 tool_calls，也可能是纯文本）。"""
    i = Handler.counter["n"]
    Handler.counter["n"] = i + 1
    text = SCRIPT[i % len(SCRIPT)]

    # 有工具定义、且尚未收到工具结果 → 返回一次 write_file 工具调用。
    if req.get("tools") and not _has_tool_result(req):
        decided = _decide_tool_call(req)
        if decided is not None:
            path, content = decided
            return (
                {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": [
                        {
                            "id": f"call_mock_{i}",
                            "type": "function",
                            "function": {
                                "name": "write_file",
                                "arguments": json.dumps(
                                    {"path": path, "content": content},
                                    ensure_ascii=False,
                                ),
                            },
                        }
                    ],
                },
                "tool_calls",
            )
    return ({"role": "assistant", "content": text}, "stop")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    counter = {"n": 0}

    def log_message(self, *args):  # 安静，不污染演示终端
        pass

    def _json(self, obj, code=200):
        body = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.rstrip("/").endswith("/v1/models"):
            self._json({"object": "list", "data": [{"id": "mock-model", "object": "model"}]})
        else:
            self._json({"error": "not found"}, 404)

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b"{}"
        try:
            req = json.loads(raw or b"{}")
        except json.JSONDecodeError:
            req = {}

        message, finish = _completion_message(req)
        model = req.get("model", "mock-model")

        if req.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "keep-alive")
            self.end_headers()
            if message.get("tool_calls"):
                # 流式工具调用：一次性给出完整 tool_calls 增量块（OpenAI 语义）
                tc = message["tool_calls"][0]
                delta = {
                    "tool_calls": [
                        {
                            "index": 0,
                            "id": tc["id"],
                            "type": "function",
                            "function": tc["function"],
                        }
                    ]
                }
                chunk = {
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "created": int(time.time()),
                    "model": model,
                    "choices": [{"index": 0, "delta": delta, "finish_reason": None}],
                }
                self.wfile.write(f"data: {json.dumps(chunk, ensure_ascii=False)}\n\n".encode("utf-8"))
                self.wfile.flush()
            else:
                text = message.get("content") or ""
                for j in range(0, len(text), 18):
                    payload = {
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "created": int(time.time()),
                        "model": model,
                        "choices": [
                            {"index": 0, "delta": {"content": text[j : j + 18]}, "finish_reason": None}
                        ],
                    }
                    self.wfile.write(f"data: {json.dumps(payload, ensure_ascii=False)}\n\n".encode("utf-8"))
                    self.wfile.flush()
                    time.sleep(0.03)
            done = {
                "id": "chatcmpl-mock",
                "object": "chat.completion.chunk",
                "created": int(time.time()),
                "model": model,
                "choices": [{"index": 0, "delta": {}, "finish_reason": finish}],
            }
            self.wfile.write(f"data: {json.dumps(done, ensure_ascii=False)}\n\n".encode("utf-8"))
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
            return

        self._json(
            {
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "created": int(time.time()),
                "model": model,
                "choices": [
                    {"index": 0, "message": message, "finish_reason": finish}
                ],
                "usage": {"prompt_tokens": 128, "completion_tokens": 48, "total_tokens": 176},
            }
        )


if __name__ == "__main__":
    print(f"mock LLM listening on http://127.0.0.1:{PORT}/v1", flush=True)
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
