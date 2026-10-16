"""File Intelligence Layer.

Implements the contract's file endpoints:

* ``load``      — read a file, split into :class:`FileSection` (by Markdown
  headings / Python def·class / generic top-level declarations / fixed window),
  detect language, compute sha256.
* ``display``   — syntax-highlighted HTML artifact (Pygments if present, else an
  escaped ``<pre>`` fallback).
* ``modify``    — LLM-assisted *or* deterministic rule-based unified-diff edit.
  The path is always ``safe_resolve``-d and a backup is written before mutate.
* ``collapse``  — fold requested (or all long) sections.
* ``ask``       — RAG over the file + KB → answer + citations.
* ``interact``  — multi-turn retrieval-augmented chat backed by L2/L3 memory.

The deterministic edit rule (``apply_rule_edit``) and backup writer are module
functions so the harness bridge can reuse the exact same safe-edit primitive.
"""

from __future__ import annotations

import difflib
import hashlib
import html
import json
import logging
import re
import uuid
from pathlib import Path
from typing import Optional

from marguerite_workbench.llm import llm_or_fallback
from marguerite_workbench.memory import MemoryEngine, VEC_DIM, cosine, embed
from marguerite_workbench.models import (
    AskResult, DisplayResult, FileCard, FileSection, InteractResult,
    ModifyResult,
)
from marguerite_workbench.security import safe_resolve

logger = logging.getLogger("marguerite.file_intel")

# ---------------------------------------------------------------------------
# Constants (anti R5)
# ---------------------------------------------------------------------------
PREVIEW_LINES = 12
COLLAPSE_THRESHOLD_LINES = 40
BACKUP_DIRNAME = "backups"
CARDS_FILENAME = "files.json"
SESSIONS_FILENAME = "sessions.jsonl"
QUOTED_RE = re.compile(r'(["\'])(.*?)\1', re.DOTALL)
HEADING_RE = re.compile(r'^(#{1,6})\s+(.+?)\s*#*\s*$')
PY_DECL_RE = re.compile(r'^(?:async\s+)?(def|class)\s+([A-Za-z_]\w*)')
CODE_DECL_RE = re.compile(
    r'^(?:export\s+)?(?:default\s+)?(?:async\s+)?(?:public\s+|private\s+|protected\s+|static\s+|final\s+)?'
    r'(?:func(?:tion)?|class|struct|interface|enum|impl|mod|package|sub|fn|def|procedure)\b'
)
JS_ARROW_RE = re.compile(r'^const\s+[A-Za-z_]\w*\s*=\s*(?:\([^)]*\)|[A-Za-z_]\w*)\s*=>')
WINDOW_LINES = 60

LANG_BY_EXT = {
    ".py": "python", ".js": "javascript", ".jsx": "javascript", ".ts": "typescript",
    ".tsx": "typescript", ".rs": "rust", ".go": "go", ".java": "java", ".c": "c",
    ".cpp": "cpp", ".h": "c", ".hpp": "cpp", ".md": "markdown", ".json": "json",
    ".yaml": "yaml", ".yml": "yaml", ".html": "html", ".htm": "html", ".css": "css",
    ".sh": "bash", ".sql": "sql", ".toml": "toml", ".txt": "text", ".rb": "ruby",
    ".php": "php", ".cs": "csharp", ".rs": "rust",
}


# ---------------------------------------------------------------------------
# Section splitting
# ---------------------------------------------------------------------------
def detect_language(path: str, text: str) -> Optional[str]:
    ext = Path(path).suffix.lower()
    if ext in LANG_BY_EXT:
        return LANG_BY_EXT[ext]
    # Shebang fallback.
    first = text.splitlines()[:1]
    if first and first[0].startswith("#!"):
        if "python" in first[0]:
            return "python"
        if "node" in first[0]:
            return "javascript"
        if "bash" in first[0]:
            return "bash"
    return "text"


def _has_content(lines: list[str]) -> bool:
    return any(l.strip() for l in lines)


def _prelude_title(start: int) -> str:
    return "module header" if start == 0 else f"lines {start + 1}–{start + 1 + 0}"


def _finalize(lines: list[str], start: int, end: int, kind: str, title: str) -> FileSection:
    seg = lines[start:end]
    text = "\n".join(seg)
    preview = "\n".join(seg[:PREVIEW_LINES])
    return FileSection(
        id=f"{kind}-{start + 1}",
        title=title or None,
        start_line=start + 1,
        end_line=end,
        kind=kind,
        collapsed=False,
        preview=preview,
        full=text,
    )


def _build_sections(lines: list[str], bounds: list[tuple[int, str, str]]) -> list[FileSection]:
    """Turn (start_index, kind, title) bounds into FileSection objects.

    Each declaration spans ``[start, next_start)``; a single leading prelude
    (content before the first declaration) is emitted only when present.  No
    overlapping inter-declaration blocks are produced.
    """
    n = len(lines)
    if not bounds:
        return [_finalize(lines, 0, n, "raw", "document")] if n else []
    bounds = sorted(bounds, key=lambda b: b[0])
    sections: list[FileSection] = []
    first_start = bounds[0][0]
    if first_start > 0 and _has_content(lines[:first_start]):
        sections.append(_finalize(lines, 0, first_start, "block", "preamble"))
    for i, (start, kind, title) in enumerate(bounds):
        end = bounds[i + 1][0] if i + 1 < len(bounds) else n
        sections.append(_finalize(lines, start, end, kind, title))
    return sections


def _split_markdown(lines: list[str]) -> list[FileSection]:
    bounds = []
    for i, line in enumerate(lines):
        m = HEADING_RE.match(line)
        if m:
            bounds.append((i, "heading", m.group(2).strip()))
    return _build_sections(lines, bounds)


def _split_python(lines: list[str]) -> list[FileSection]:
    bounds = []
    for i, line in enumerate(lines):
        m = PY_DECL_RE.match(line)
        if m:
            start = i
            while start > 0 and lines[start - 1].lstrip().startswith("@"):
                start -= 1
            kind = "class" if m.group(1) == "class" else "function"
            bounds.append((start, kind, m.group(2)))
    if not bounds:
        return _build_sections(lines, [(0, "block", "module")])
    return _build_sections(lines, bounds)


def _split_code_generic(lines: list[str]) -> list[FileSection]:
    bounds = []
    for i, line in enumerate(lines):
        if CODE_DECL_RE.match(line) or JS_ARROW_RE.match(line):
            name = re.sub(r"[^\w]", "", line.strip()[:24]) or f"L{i + 1}"
            bounds.append((i, "block", name))
    if bounds:
        return _build_sections(lines, bounds)
    # Fixed-window fallback.
    bounds = [(s, "block", f"lines {s + 1}–{min(s + WINDOW_LINES, len(lines))}")
              for s in range(0, len(lines), WINDOW_LINES)]
    return _build_sections(lines, bounds)


def split_sections(text: str, language: Optional[str]) -> list[FileSection]:
    lines = text.splitlines()
    if language == "markdown":
        return _split_markdown(lines)
    if language == "python":
        return _split_python(lines)
    return _split_code_generic(lines)


# ---------------------------------------------------------------------------
# Deterministic safe edit + backup (shared with harness.py)
# ---------------------------------------------------------------------------
def apply_rule_edit(orig: str, instruction: str) -> tuple[str, bool, str]:
    """Apply a *safe, rule-based* edit from a natural-language instruction.

    Returns ``(new_content, applied, note)``.  When no rule matches, the content
    is returned unchanged with ``applied=False`` (honest, never faked).
    """
    low = instruction.lower()
    quoted = QUOTED_RE.findall(instruction)
    # QUOTED_RE returns (quote_char, inner) pairs; keep the inner strings.
    args = [q[1] for q in quoted]

    # replace / change "<old>" with "<new>"
    if ("replace" in low or "change" in low or "改为" in instruction or "替换" in instruction) and len(args) >= 2:
        old, new = args[0], args[1]
        if old in orig:
            return orig.replace(old, new, 1), True, f"replaced first occurrence of {old!r}"
        return orig, False, f"target {old!r} not found in file"

    # prepend / add header
    if ("prepend" in low or "add header" in low or "头部" in instruction or "开头" in instruction) and args:
        return f"{args[0]}\n{orig}", True, "prepended header"

    # append / add at end
    if ("append" in low or "add at end" in low or "末尾" in instruction or "结尾" in instruction) and args:
        return f"{orig}\n{args[0]}", True, "appended content"

    # remove / delete lines containing "<text>"
    if ("remove" in low or "delete" in low or "删除" in instruction or "移除" in instruction) and args:
        target = args[0]
        kept = [ln for ln in orig.splitlines() if target not in ln]
        removed = len(orig.splitlines()) - len(kept)
        if removed:
            return "\n".join(kept), True, f"removed {removed} line(s) containing {target!r}"
        return orig, False, f"no line contained {target!r}"

    # comment out lines containing "<text>"
    if ("comment" in low or "注释" in instruction) and args:
        target = args[0]
        out, changed = [], 0
        for ln in orig.splitlines():
            if target in ln and not ln.lstrip().startswith("#"):
                out.append("# " + ln)
                changed += 1
            else:
                out.append(ln)
        if changed:
            return "\n".join(out), True, f"commented {changed} line(s) containing {target!r}"
        return orig, False, f"no line to comment for {target!r}"

    return orig, False, (
        "no deterministic rule matched the instruction; supply a quoted target "
        "(replace \"a\" with \"b\" / add header \"x\" / append \"x\" / remove \"x\")"
    )


def write_backup(backups_dir: Path, resolved: Path, content: str) -> str:
    """Write ``content`` to a timestamped backup; return its storage reference."""
    backups_dir.mkdir(parents=True, exist_ok=True)
    safe_stem = re.sub(r"[^\w.-]", "_", resolved.name)
    ref = f"{safe_stem}.{uuid.uuid4().hex[:8]}.bak"
    (backups_dir / ref).write_text(content, encoding="utf-8")
    return f"{BACKUP_DIRNAME}/{ref}"


def unified_diff(old: str, new: str) -> str:
    return "".join(
        difflib.unified_diff(
            old.splitlines(keepends=True),
            new.splitlines(keepends=True),
            fromfile="original", tofile="modified",
        )
    )


# ---------------------------------------------------------------------------
# Syntax highlight
# ---------------------------------------------------------------------------
def highlight(text: str, language: Optional[str]) -> str:
    lexer_name = language or "text"
    try:
        from pygments import highlight as _hl  # lazy (optional dep)
        from pygments.lexers import get_lexer_by_name
        from pygments.formatters import HtmlFormatter

        lexer = get_lexer_by_name(lexer_name, stripall=False)
        body = _hl(text, lexer, HtmlFormatter(nowrap=True))
        return f'<pre class="highlight"><code>{body}</code></pre>'
    except Exception as exc:  # noqa: BLE001 — graceful fallback to escaped <pre>
        logger.info("pygments unavailable (%s); using escaped pre", exc)
        return f"<pre class=\"highlight\"><code>{html.escape(text)}</code></pre>"


# ---------------------------------------------------------------------------
# File Intelligence engine
# ---------------------------------------------------------------------------
class FileIntel:
    def __init__(
        self,
        workspace: Path,
        data_dir: Path,
        kb,  # KnowledgeBase
        memory: MemoryEngine,
    ) -> None:
        # Resolve both roots so file_id (relative_to) and path comparisons are
        # stable across Windows 8.3 short-name vs long-name forms.
        self.workspace = Path(workspace).resolve()
        self.data_dir = Path(data_dir).resolve()
        self.kb = kb
        self.memory = memory
        self.backups_dir = self.data_dir / BACKUP_DIRNAME
        self.cards_path = self.data_dir / CARDS_FILENAME
        self.sessions_path = self.data_dir / SESSIONS_FILENAME
        self.cards: dict[str, FileCard] = {}
        self._load_cards()

    # -- persistence --------------------------------------------------------
    def _load_cards(self) -> None:
        if self.cards_path.exists():
            try:
                raw = json.loads(self.cards_path.read_text(encoding="utf-8"))
                for c in raw:
                    self.cards[c["file_id"]] = FileCard(**c)
            except (json.JSONDecodeError, TypeError) as exc:  # R3: log + start fresh
                logger.warning("could not load cards: %s", exc)

    def _save_cards(self) -> None:
        self.cards_path.write_text(
            json.dumps([c.model_dump() for c in self.cards.values()], ensure_ascii=False, indent=2),
            encoding="utf-8",
        )

    def _get_card(self, file_id: str) -> FileCard:
        if file_id not in self.cards:
            raise KeyError(f"unknown file_id: {file_id!r} — load it first via /api/files/load")
        return self.cards[file_id]

    # -- load ---------------------------------------------------------------
    def load(self, path: str) -> FileCard:
        resolved = safe_resolve(self.workspace, path)
        raw = resolved.read_bytes()
        size = len(raw)
        sha = hashlib.sha256(raw).hexdigest()
        try:
            text = raw.decode("utf-8")
            encoding, has_binary = "utf-8", False
        except UnicodeDecodeError:
            text = raw.decode("utf-8", "replace")
            encoding, has_binary = "binary/replace", True
        language = detect_language(str(resolved), text)
        sections = split_sections(text, language)
        file_id = resolved.relative_to(self.workspace.resolve()).as_posix()
        summary = next((ln.strip() for ln in text.splitlines() if ln.strip()), "")
        card = FileCard(
            file_id=file_id,
            path=str(resolved),
            language=language,
            size_bytes=size,
            sha256=sha,
            sections=sections,
            meta={
                "lines": len(text.splitlines()),
                "encoding": encoding,
                "has_binary": has_binary,
                "summary": summary[:200],
            },
        )
        self.cards[file_id] = card
        self._save_cards()
        # Promote a semantic memory entry (L5) for the file overview.
        self.memory.store("L5", "file_overview", f"{file_id}: {summary}", refs=[file_id])
        return card

    # -- display ------------------------------------------------------------
    def display(self, file_id: str) -> DisplayResult:
        card = self._get_card(file_id)
        resolved = Path(card.path)
        text = resolved.read_text(encoding="utf-8", errors="replace")
        return DisplayResult(html=highlight(text, card.language), language=card.language or "text")

    # -- modify -------------------------------------------------------------
    def modify(self, file_id: str, instruction: str) -> ModifyResult:
        card = self._get_card(file_id)
        resolved = Path(card.path)
        orig = resolved.read_text(encoding="utf-8", errors="replace")
        new_content, applied, _note = self._edit(orig, instruction, card.language)
        diff = unified_diff(orig, new_content)
        backup_ref: Optional[str] = None
        if applied:
            backup_ref = write_backup(self.backups_dir, resolved, orig)
            resolved.write_text(new_content, encoding="utf-8")
            self._reload_card(card, new_content)
        return ModifyResult(
            new_content=new_content, diff=diff, backup_ref=backup_ref, applied=applied,
        )

    def _edit(self, orig: str, instruction: str, language: Optional[str] = None) -> tuple[str, bool, str]:
        # Deterministic baseline — always works with no API key.
        new_content, applied, note = apply_rule_edit(orig, instruction)
        # Optional upgrade: ask an LLM for the full patched file (sanity-checked).
        try:
            from llm import LLMClient

            client = LLMClient()
            if client.enabled:
                prompt = (
                    "You are a precise code editor. Return ONLY the complete modified file "
                    "content (no commentary). Do not truncate.\n\n"
                    f"# FILE ({language or 'text'})\n{orig}\n\n# INSTRUCTION\n{instruction}\n"
                )
                llm_out = client.complete(prompt, system="Return the full file only.")
                if llm_out and len(llm_out) > 0.3 * len(orig) and llm_out.strip() != orig.strip():
                    return llm_out, True, "applied via LLM"
        except Exception as exc:  # noqa: BLE001 — keep the deterministic result
            logger.info("LLM edit unavailable (%s); using rule engine", exc)
        return new_content, applied, note

    def _reload_card(self, card: FileCard, new_text: str) -> None:
        card.sections = split_sections(new_text, card.language)
        card.size_bytes = len(new_text.encode("utf-8"))
        card.sha256 = hashlib.sha256(new_text.encode("utf-8")).hexdigest()
        card.meta["lines"] = len(new_text.splitlines())
        self._save_cards()

    # -- collapse ------------------------------------------------------------
    def collapse(self, file_id: str, section_ids: Optional[list[str]]) -> FileCard:
        card = self._get_card(file_id)
        if section_ids is None:
            target = {
                s.id for s in card.sections
                if (s.end_line - s.start_line + 1) > COLLAPSE_THRESHOLD_LINES
            }
        else:
            target = set(section_ids)
        for s in card.sections:
            if s.id in target:
                s.collapsed = True
                s.full = None
        self._save_cards()
        return card

    # -- retrieval helpers --------------------------------------------------
    def _retrieve_sections(self, card: FileCard, query: str, top_k: int = 3) -> list[tuple[FileSection, float]]:
        qv = embed(query)
        scored = []
        for s in card.sections:
            text = s.full or s.preview
            ev = embed(text)
            if len(ev) != VEC_DIM:
                continue
            scored.append((cosine(qv, ev), s))
        scored.sort(key=lambda x: -x[0])
        return scored[:top_k]

    @staticmethod
    def _best_excerpt(text: str, query: str) -> str:
        """Pick the sentence with the highest query-term overlap."""
        q_terms = {t for t in re.findall(r"[A-Za-z_]{3,}|[\u4e00-\u9fff]", query.lower())}
        best, best_score = text, -1
        for sent in re.split(r"(?<=[\n。.!?])", text):
            sent = sent.strip()
            if not sent:
                continue
            low = sent.lower()
            score = sum(1 for t in q_terms if t in low)
            if score > best_score:
                best, best_score = sent, score
        return best[:400]

    # -- ask (RAG) ----------------------------------------------------------
    def ask(self, file_id: str, question: str) -> AskResult:
        card = self._get_card(file_id)
        section_hits = self._retrieve_sections(card, question, top_k=3)
        kb_hits = self.kb.search(question, k=3)
        citations: list[dict] = []
        for score, s in section_hits:
            citations.append({
                "section_id": s.id, "file": card.path,
                "snippet": self._best_excerpt(s.full or s.preview, question),
                "score": round(score, 4),
            })
        for h in kb_hits:
            citations.append({"source": h["source"], "snippet": h["text"][:300], "score": h["score"]})

        def _fallback() -> str:
            if section_hits:
                top = section_hits[0][1]
                excerpt = self._best_excerpt(top.full or top.preview, question)
                lead = f"基于文件 {card.path} 的相关片段"
                if top.title:
                    lead += f"（{top.title}）"
                return f"{lead}：\n“{excerpt}”\n\n本地知识库另命中 {len(kb_hits)} 条相关记录。"
            if kb_hits:
                return f"文件内未找到强相关片段；本地知识库命中 {len(kb_hits)} 条：\n" + \
                       "\n".join(f"- {h['text'][:160]}" for h in kb_hits)
            return "未在文件或本地知识库中找到与问题强相关的内容，请尝试更具体的措辞。"

        answer = llm_or_fallback(
            f"Answer the question using ONLY the context.\n\n"
            f"QUESTION: {question}\n\nFILE: {card.path}\n"
            + "\n".join(f"[sec {c['section_id']}] {c['snippet']}" for c in citations[:4]),
            _fallback,
            system="You are a file Q&A assistant. Cite the source sections.",
        )
        return AskResult(answer=answer, citations=citations)

    # -- interact (multi-turn, L2/L3 backed) -------------------------------
    def interact(self, file_id: str, message: str, session_id: Optional[str]) -> InteractResult:
        card = self._get_card(file_id)
        session_id = session_id or uuid.uuid4().hex
        history = self._load_session(session_id)
        section_hits = self._retrieve_sections(card, message, top_k=2)
        kb_hits = self.kb.search(message, k=2)

        ctx_lines = []
        for _score, s in section_hits:
            ctx_lines.append(f"[{s.id}] {(s.full or s.preview)[:500]}")
        for h in kb_hits:
            ctx_lines.append(f"[kb:{h['source']}] {h['text'][:300]}")

        def _fallback() -> str:
            base = ""
            if section_hits:
                top = section_hits[0][1]
                base = f"（基于 {card.path} 的“{self._best_excerpt(top.full or top.preview, message)}”）"
            turn_no = len(history) + 1
            return (
                f"第 {turn_no} 轮：{base} 我理解你的问题是「{message}」。"
                + (f"本地知识库另有 {len(kb_hits)} 条相关记录可参考。" if kb_hits else "")
                + " 你可以继续追问或要求我基于上下文修改文件。"
            )

        reply = llm_or_fallback(
            "Chat with the user using the file context and prior turns.\n\n"
            f"FILE: {card.path}\nCONTEXT:\n" + "\n".join(ctx_lines) + "\n\n"
            + "\n".join(f"{r}: {c}" for r, c in history[-6:]) + f"\nUSER: {message}",
            _fallback,
            system="You are a retrieval-augmented file assistant. Be concise.",
        )

        # Persist the exchange into L2 (short_term) + L3 (working) memory so the
        # next turn can retrieve it, and into the session log.
        self.memory.store("L2", "chat", f"[session {session_id}] user: {message}\nassistant: {reply}",
                          refs=[session_id, file_id])
        self.memory.store("L3", "scratch", f"open question in {file_id}: {message}",
                          refs=[session_id, file_id])
        self._append_session(session_id, message, reply)
        return InteractResult(reply=reply, session_id=session_id)

    # -- session log --------------------------------------------------------
    def _load_session(self, session_id: str) -> list[tuple[str, str]]:
        out: list[tuple[str, str]] = []
        if not self.sessions_path.exists():
            return out
        for line in self.sessions_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if rec.get("session_id") == session_id:
                out.append((rec.get("role", "user"), rec.get("content", "")))
        return out

    def _append_session(self, session_id: str, user_msg: str, reply: str) -> None:
        with self.sessions_path.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps({"session_id": session_id, "role": "user", "content": user_msg},
                                ensure_ascii=False) + "\n")
            fh.write(json.dumps({"session_id": session_id, "role": "assistant", "content": reply},
                                ensure_ascii=False) + "\n")
