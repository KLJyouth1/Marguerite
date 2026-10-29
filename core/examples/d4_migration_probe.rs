//! 一次性探针（**跑完即删**，不属于仓库）：拿真实用户库的**副本**跑 D4 迁移，量中文命中率。
//!
//! 用法：`cargo run -p marguerite-core --example d4_migration_probe -- <copy.db> <out.json>`
//! 只接受调用方给的路径，绝不自己拼真实数据目录。

use std::path::PathBuf;

use marguerite_core::memory::short_term::ShortTermMemory;
use serde_json::json;

fn main() {
    let mut args = std::env::args().skip(1);
    let db = PathBuf::from(args.next().expect("usage: d4_migration_probe <copy.db> <out.json>"));
    let out = PathBuf::from(args.next().expect("usage: d4_migration_probe <copy.db> <out.json>"));

    let probes = [
        "配置",
        "会话",
        "记忆",
        "打包",
        "技术方案",
        "架构",
        "评论",
        "标签",
        "博客",
        "Next",
        "Rust",
        "技术方案设计",
    ];

    // init 必须：搬旧 `sessions` 表 → 补 `search_text` 列 → 重建 FTS 虚表 → 回填 → rebuild
    let mut m = ShortTermMemory::new(&db);
    m.init().expect("init + migrate");
    let rows = m.session_count().expect("count");

    let per_query: Vec<serde_json::Value> = probes
        .iter()
        .map(|q| {
            let hits = m.search(q, 5).unwrap_or_else(|e| panic!("search {q:?} 失败: {e}"));
            json!({
                "query": q,
                "hits": hits.len(),
                "ids": hits.iter().map(|h| h.session_id.clone()).collect::<Vec<_>>(),
                "summary_head": hits.first().map(|h| h.summary.chars().take(40).collect::<String>()),
            })
        })
        .collect();

    // 展示字段必须是原文：分词副本只进 `search_text`
    let one: (String, String) = {
        let conn = rusqlite::Connection::open(&db).expect("open");
        conn
            .query_row(
                "SELECT summary, search_text FROM short_term_sessions WHERE length(search_text) > 0 LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("a migrated row")
    };
    let recent = m.get_recent_sessions(3).expect("recent");
    let any_cjk_intact = recent
        .iter()
        .any(|s| s.summary.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));

    // 孤儿行：迁移前真实库里实测到 `fts5: missing row 2496 from content table`，重建后必须归零
    let conn = rusqlite::Connection::open(&db).expect("reopen");
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM short_term_sessions_fts f
             WHERE NOT EXISTS (SELECT 1 FROM short_term_sessions s WHERE s.rowid = f.rowid)",
            [],
            |r| r.get(0),
        )
        .expect("orphans");
    let tokenizer: String = conn
        .query_row(
            "SELECT value FROM short_term_meta WHERE key='tokenizer_version'",
            [],
            |r| r.get(0),
        )
        .expect("meta");
    let fts_cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('short_term_sessions_fts')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    let content_cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('short_term_sessions')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    let hit = per_query
        .iter()
        .filter(|p| p["hits"].as_i64().unwrap_or(0) > 0)
        .count();
    let report = json!({
        "rows_after_migration": rows,
        "content_columns": content_cols,
        "fts_columns": fts_cols,
        "tokenizer_version": tokenizer,
        "fts_orphan_rows": orphans,
        "original_summary_head": one.0.chars().take(60).collect::<String>(),
        "indexed_copy_head": one.1.chars().take(60).collect::<String>(),
        "display_copy_has_cjk": any_cjk_intact,
        "probes": per_query,
        "queries_with_hits": hit,
        "queries_total": probes.len(),
    });
    std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).expect("write report");
    println!("wrote {}", out.display());
}
