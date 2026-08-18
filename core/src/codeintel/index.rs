//! 项目索引：增量文件扫描 + tree-sitter 符号提取 + SQLite 持久化
//!
//! 设计目标：
//! - 增量：按 (mtime, size) 比对，只重解析变更文件；
//! - 符号级：复用 `guard::syntax` 的五语言 tree-sitter Parser 范式，
//!   通过通用 field "name" 提取定义符号（fn/struct/class/…）；
//! - 持久化：每 workspace 一个 SQLite 库（`files` / `symbols` / `chunks` 三表），
//!   检索纯查库，不再全盘读盘；
//! - 容错：单文件解析/读取失败跳过并计入 `skipped`，绝不中断整库。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection};
use tree_sitter::{Language as TsLanguage, Node, Parser};

use crate::guard::syntax::Language;

/// 单文件索引体积上限：超过则跳过符号提取与分块（计入 skipped）
pub const MAX_INDEX_BYTES: u64 = 1_048_576;
/// 代码分块行数（按行切块，块间重叠 8 行保留上下文）
const CHUNK_LINES: usize = 40;
const CHUNK_OVERLAP: usize = 8;

/// 索引报告
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexReport {
    pub files_total: usize,
    pub files_updated: usize,
    pub files_unchanged: usize,
    pub files_removed: usize,
    pub skipped: usize,
    pub symbols_total: usize,
}

/// 提取出的符号
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line: usize,
}

/// 定义类节点 kind → 友好符号类型
fn symbol_kind(node_kind: &str) -> Option<&'static str> {
    match node_kind {
        "function_item" | "function_declaration" | "function_definition" => Some("function"),
        "method_declaration" | "method_definition" => Some("method"),
        "struct_item" => Some("struct"),
        "class_definition" | "class_declaration" => Some("class"),
        "enum_item" | "enum_declaration" => Some("enum"),
        "trait_item" => Some("trait"),
        "interface_declaration" => Some("interface"),
        "type_item" | "type_alias_declaration" => Some("type"),
        "const_item" => Some("const"),
        "static_item" => Some("static"),
        "mod_item" => Some("module"),
        "macro_definition" => Some("macro"),
        _ => None,
    }
}

/// 符号提取器：每语言一个 Parser（Mutex 包裹，对齐 guard/syntax.rs 范式）
pub struct SymbolExtractor {
    parsers: HashMap<Language, Mutex<Parser>>,
}

impl Default for SymbolExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolExtractor {
    pub fn new() -> Self {
        let mut parsers = HashMap::new();
        let mut register = |lang: Language, lang_fn: TsLanguage| {
            let mut parser = Parser::new();
            if parser.set_language(&lang_fn).is_ok() {
                parsers.insert(lang, Mutex::new(parser));
            }
        };
        register(
            Language::JavaScript,
            tree_sitter_javascript::LANGUAGE.into(),
        );
        register(
            Language::TypeScript,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        );
        register(Language::Python, tree_sitter_python::LANGUAGE.into());
        register(Language::Rust, tree_sitter_rust::LANGUAGE.into());
        register(Language::Go, tree_sitter_go::LANGUAGE.into());
        Self { parsers }
    }

    /// 提取代码中的定义符号；语言不支持或解析失败返回空（绝不 panic）
    pub fn extract(&self, code: &str, lang: Language, rel_path: &str) -> Vec<Symbol> {
        let Some(parser) = self.parsers.get(&lang) else {
            return Vec::new();
        };
        let mut parser = parser.lock().unwrap_or_else(|e| e.into_inner());
        let Some(tree) = parser.parse(code, None) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        Self::walk(tree.root_node(), code, rel_path, &mut out);
        out
    }

    fn walk(node: Node, code: &str, rel_path: &str, out: &mut Vec<Symbol>) {
        if let Some(kind) = symbol_kind(node.kind()) {
            // 通用策略：定义节点统一经 field "name" 取符号名（五语言 grammar 均支持）
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(code.as_bytes()) {
                    if !name.is_empty() {
                        out.push(Symbol {
                            name: name.to_string(),
                            kind: kind.to_string(),
                            path: rel_path.to_string(),
                            line: name_node.start_position().row + 1,
                        });
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk(child, code, rel_path, out);
        }
    }
}

/// 按行切块（代码天然按行组织，比段落切块更贴合）
fn chunk_lines(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        out.push(lines[start..end].join("\n"));
        if end == lines.len() {
            break;
        }
        start = end.saturating_sub(CHUNK_OVERLAP);
    }
    out
}

fn is_skipped_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".versions" | "build" | "dist" | "out" | ".next"
    )
}

fn is_binary_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "ico"
            | "pdf"
            | "zip"
            | "gz"
            | "tar"
            | "exe"
            | "dll"
            | "so"
            | "o"
            | "rlib"
            | "rmeta"
            | "bin"
            | "wasm"
            | "mp4"
            | "mp3"
    )
}

fn walk_workspace(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !is_skipped_dir(name) {
                walk_workspace(&p, out);
            }
        } else if p.is_file() {
            out.push(p);
        }
    }
}

fn mtime_secs(path: &Path) -> u64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 单 workspace 项目索引（SQLite 持久化，Mutex 串行化写）
pub struct ProjectIndex {
    workspace: PathBuf,
    db: Mutex<Connection>,
    extractor: SymbolExtractor,
}

impl ProjectIndex {
    /// 打开（或创建）索引库。`db_dir` 为 SQLite 落盘目录（生产用 data_dir/codeintel，测试用临时目录）。
    pub fn open(workspace: &Path, db_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(db_dir)?;
        let key = workspace
            .to_string_lossy()
            .replace(['\\', '/', ':'], "_")
            .replace(' ', "_");
        let db_path = db_dir.join(format!("idx_{key}.db"));
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files(
                path TEXT PRIMARY KEY,
                mtime INTEGER NOT NULL,
                size INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS symbols(
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                line INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
             CREATE INDEX IF NOT EXISTS idx_symbols_path ON symbols(path);
             CREATE TABLE IF NOT EXISTS chunks(
                path TEXT NOT NULL,
                idx INTEGER NOT NULL,
                text TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_chunks_path ON chunks(path);",
        )?;
        Ok(Self {
            workspace: workspace.to_path_buf(),
            db: Mutex::new(conn),
            extractor: SymbolExtractor::new(),
        })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// 增量刷新：只重解析 (mtime, size) 变化的文件，清理已删除文件的记录。
    pub fn refresh(&self) -> anyhow::Result<IndexReport> {
        let mut files = Vec::new();
        walk_workspace(&self.workspace, &mut files);

        let mut report = IndexReport {
            files_total: files.len(),
            ..Default::default()
        };

        // 现有记录：path → (mtime, size)
        let mut known: HashMap<String, (u64, u64)> = HashMap::new();
        {
            let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
            let mut stmt = db.prepare("SELECT path, mtime, size FROM files")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, u64>(2)?,
                ))
            })?;
            for row in rows.flatten() {
                known.insert(row.0, (row.1, row.2));
            }
        }

        let mut seen: Vec<String> = Vec::with_capacity(files.len());
        for abs in &files {
            let Ok(rel) = abs.strip_prefix(&self.workspace) else {
                continue;
            };
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            seen.push(rel_s.clone());

            let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or("");
            let size = abs.metadata().map(|m| m.len()).unwrap_or(0);
            let mtime = mtime_secs(abs);

            if is_binary_ext(ext) || size > MAX_INDEX_BYTES {
                report.skipped += 1;
                continue;
            }
            if let Some(&(m, s)) = known.get(&rel_s) {
                if m == mtime && s == size {
                    report.files_unchanged += 1;
                    continue;
                }
            }
            let Ok(content) = std::fs::read_to_string(abs) else {
                report.skipped += 1;
                continue;
            };

            let symbols = match Language::from_extension(ext) {
                Some(lang) => self.extractor.extract(&content, lang, &rel_s),
                None => Vec::new(),
            };
            let chunks = chunk_lines(&content);

            let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
            let tx = db.unchecked_transaction()?;
            tx.execute("DELETE FROM symbols WHERE path = ?1", params![rel_s])?;
            tx.execute("DELETE FROM chunks WHERE path = ?1", params![rel_s])?;
            for sym in &symbols {
                tx.execute(
                    "INSERT INTO symbols(path, name, kind, line) VALUES (?1, ?2, ?3, ?4)",
                    params![rel_s, sym.name, sym.kind, sym.line as i64],
                )?;
            }
            for (i, chunk) in chunks.iter().enumerate() {
                tx.execute(
                    "INSERT INTO chunks(path, idx, text) VALUES (?1, ?2, ?3)",
                    params![rel_s, i as i64, chunk],
                )?;
            }
            tx.execute(
                "INSERT OR REPLACE INTO files(path, mtime, size) VALUES (?1, ?2, ?3)",
                params![rel_s, mtime, size as i64],
            )?;
            tx.commit()?;
            report.files_updated += 1;
        }

        // 清理已删除文件
        {
            let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
            for old in known.keys() {
                if !seen.iter().any(|s| s == old) {
                    db.execute("DELETE FROM files WHERE path = ?1", params![old])?;
                    db.execute("DELETE FROM symbols WHERE path = ?1", params![old])?;
                    db.execute("DELETE FROM chunks WHERE path = ?1", params![old])?;
                    report.files_removed += 1;
                }
            }
            let total: i64 = db.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
            report.symbols_total = total as usize;
        }

        Ok(report)
    }

    /// 统计：文件数 / 符号数 / 分块数
    pub fn stats(&self) -> anyhow::Result<(usize, usize, usize)> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let f: i64 = db.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let s: i64 = db.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
        let c: i64 = db.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        Ok((f as usize, s as usize, c as usize))
    }

    /// 符号查询：按名称子串匹配（不区分大小写），limit 截断
    pub fn symbols_named(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Symbol>> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = db.prepare(
            "SELECT name, kind, path, line FROM symbols WHERE lower(name) LIKE ?1 ORDER BY name LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |r| {
            Ok(Symbol {
                name: r.get(0)?,
                kind: r.get(1)?,
                path: r.get(2)?,
                line: r.get::<_, i64>(3)? as usize,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    /// 全部分块（供检索层 BM25 打分）
    pub fn all_chunks(&self) -> anyhow::Result<Vec<(String, String)>> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = db.prepare("SELECT path, text FROM chunks ORDER BY path, idx")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.flatten().collect())
    }

    /// 指定文件的分块（按 idx 升序，供上下文组装）
    pub fn chunks_of(&self, path: &str) -> anyhow::Result<Vec<String>> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = db.prepare("SELECT text FROM chunks WHERE path = ?1 ORDER BY idx")?;
        let rows = stmt.query_map(params![path], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "codeintel_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(ws: &Path, rel: &str, content: &str) {
        let p = ws.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
    }

    #[test]
    fn extracts_symbols_across_languages() {
        let ws = tmp_dir("sym");
        let dbd = tmp_dir("sym_db");
        write(
            &ws,
            "a.rs",
            "pub struct Foo;\npub fn bar() {}\nconst N: u32 = 1;\n",
        );
        write(
            &ws,
            "b.py",
            "class Baz:\n    pass\n\ndef qux():\n    pass\n",
        );
        write(&ws, "c.ts", "function hello() {}\ninterface Iface {}\n");
        write(&ws, "d.go", "package main\nfunc main() {}\n");
        write(&ws, "e.js", "class Widget {}\n");

        let idx = ProjectIndex::open(&ws, &dbd).unwrap();
        let report = idx.refresh().unwrap();
        assert_eq!(report.files_updated, 5, "{report:?}");

        let names: Vec<String> = idx
            .symbols_named("", 100)
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        for want in [
            "Foo", "bar", "Baz", "qux", "hello", "Iface", "main", "Widget",
        ] {
            assert!(
                names.contains(&want.to_string()),
                "missing {want} in {names:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dbd);
    }

    #[test]
    fn incremental_refresh_skips_unchanged() {
        let ws = tmp_dir("incr");
        let dbd = tmp_dir("incr_db");
        write(&ws, "one.rs", "pub fn alpha() {}\n");
        write(&ws, "two.rs", "pub fn beta() {}\n");

        let idx = ProjectIndex::open(&ws, &dbd).unwrap();
        let r1 = idx.refresh().unwrap();
        assert_eq!(r1.files_updated, 2);
        assert_eq!(r1.files_unchanged, 0);

        // 未改动 → 全部 unchanged
        let r2 = idx.refresh().unwrap();
        assert_eq!(r2.files_updated, 0, "{r2:?}");
        assert_eq!(r2.files_unchanged, 2, "{r2:?}");

        // 改动一个文件 → 只重解析一个
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&ws, "one.rs", "pub fn alpha() {}\npub fn gamma() {}\n");
        let r3 = idx.refresh().unwrap();
        assert_eq!(r3.files_updated, 1, "{r3:?}");
        assert_eq!(r3.files_unchanged, 1, "{r3:?}");
        let names: Vec<String> = idx
            .symbols_named("gamma", 10)
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"gamma".to_string()));
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dbd);
    }

    #[test]
    fn removed_files_are_cleaned() {
        let ws = tmp_dir("rm");
        let dbd = tmp_dir("rm_db");
        write(&ws, "keep.rs", "pub fn keep_me() {}\n");
        write(&ws, "gone.rs", "pub fn gone_fn() {}\n");

        let idx = ProjectIndex::open(&ws, &dbd).unwrap();
        idx.refresh().unwrap();
        std::fs::remove_file(ws.join("gone.rs")).unwrap();
        let r = idx.refresh().unwrap();
        assert_eq!(r.files_removed, 1, "{r:?}");
        assert!(idx.symbols_named("gone_fn", 10).unwrap().is_empty());
        assert!(!idx.symbols_named("keep_me", 10).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dbd);
    }

    #[test]
    fn binary_and_oversized_files_skipped() {
        let ws = tmp_dir("skip");
        let dbd = tmp_dir("skip_db");
        write(&ws, "img.png", "fake binary");
        write(&ws, "big.rs", &"x".repeat((MAX_INDEX_BYTES as usize) + 1));
        write(&ws, "ok.rs", "pub fn fine() {}\n");

        let idx = ProjectIndex::open(&ws, &dbd).unwrap();
        let r = idx.refresh().unwrap();
        assert_eq!(r.skipped, 2, "{r:?}");
        assert_eq!(r.files_updated, 1, "{r:?}");
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dbd);
    }

    #[test]
    fn index_persists_across_reopen() {
        let ws = tmp_dir("persist");
        let dbd = tmp_dir("persist_db");
        write(&ws, "a.rs", "pub fn persisted_fn() {}\n");

        let idx = ProjectIndex::open(&ws, &dbd).unwrap();
        idx.refresh().unwrap();
        drop(idx);

        // 重新打开：数据仍在，且 refresh 判定全部 unchanged
        let idx2 = ProjectIndex::open(&ws, &dbd).unwrap();
        assert!(!idx2.symbols_named("persisted_fn", 10).unwrap().is_empty());
        let r = idx2.refresh().unwrap();
        assert_eq!(r.files_unchanged, 1, "{r:?}");
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dbd);
    }

    #[test]
    fn chunk_lines_covers_full_text() {
        let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let chunks = chunk_lines(&text);
        assert!(chunks.len() >= 3, "100 行应切成多块，实际 {}", chunks.len());
        let joined = chunks.join("\n");
        assert!(joined.contains("line 0"));
        assert!(joined.contains("line 99"));
    }
}
