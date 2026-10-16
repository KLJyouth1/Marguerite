//! 企业级实时协同 CRDT 引擎（Track G 地基）
//!
//! 基于 yrs 0.27（Yjs 兼容 CRDT）的共享文本文档，提供无冲突并发编辑能力：
//! 本地编辑、全量状态编码、远端更新应用、双向合并。本模块仅为协同层地基，
//! 网络传输 / Tauri 桥 / UI 由后续 track 在其上构建。

use anyhow::{anyhow, Result};
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, TextRef, Transact, Update};

/// 协同文档：包裹 yrs::Doc + 一个共享文本根类型（命名 "root"）。
///
/// CRDT 特性保证：多端并发 [`edit`](CollabDoc::edit) 后，经
/// [`apply_update`](CollabDoc::apply_update) / [`merge_from`](CollabDoc::merge_from)
/// 交换状态更新，所有副本必然收敛到完全一致的内容（可交换、幂等、可结合）。
pub struct CollabDoc {
    doc: Doc,
    text: TextRef,
}

impl CollabDoc {
    /// 创建空协同文档，内部持有一个名为 "root" 的共享文本类型。
    pub fn new() -> Self {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("root");
        Self { doc, text }
    }

    /// 文档客户端 ID（用于调试 / 关联在场状态）。
    pub fn client_id(&self) -> u64 {
        self.doc.client_id().get()
    }

    /// 在指定字符索引处插入文本（无冲突，由 CRDT 保证顺序收敛）。
    pub fn edit(&mut self, index: usize, text: &str) {
        let mut txn = self.doc.transact_mut();
        self.text.insert(&mut txn, index as u32, text);
    }

    /// 读取当前文档完整内容。
    pub fn content(&self) -> String {
        let txn = self.doc.transact();
        self.text.get_string(&txn)
    }

    /// 编码全量状态更新（V1 格式），用于网络传输或持久化。
    ///
    /// 使用空 [`StateVector`] 编码整份文档，而非当前事务内的增量，
    /// 保证无论调用时机都能得到可合并的完整状态。
    pub fn encode(&self) -> Result<Vec<u8>> {
        let txn = self.doc.transact();
        let sv = StateVector::default();
        Ok(txn.encode_state_as_update_v1(&sv))
    }

    /// 应用远端的状态更新（无冲突合并进本地副本）。
    pub fn apply_update(&mut self, update: &[u8]) -> Result<()> {
        let update = Update::decode_v1(update).map_err(|e| anyhow!("decode update failed: {e}"))?;
        let mut txn = self.doc.transact_mut();
        txn.apply_update(update)
            .map_err(|e| anyhow!("apply update failed: {e}"))?;
        Ok(())
    }

    /// 将另一文档的状态合并进自身（other -> self 单向合并）。
    ///
    /// CRDT 可交换、幂等、可结合：在 A、B 两端分别调用
    /// `a.merge_from(&b)` 与 `b.merge_from(&a)` 后，两者内容必然一致。
    pub fn merge_from(&mut self, other: &CollabDoc) -> Result<()> {
        let update = other.encode()?;
        self.apply_update(&update)
    }
}

impl Default for CollabDoc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 核心收敛性证明：两份文档独立分叉编辑后交换更新，必须收敛到完全一致。
    #[test]
    fn two_docs_converge_after_divergent_edits() {
        let mut a = CollabDoc::new();
        let mut b = CollabDoc::new();

        // 分叉编辑：A 在 0 处插入 "hello "，B 在 0 处插入 "world"。
        a.edit(0, "hello ");
        b.edit(0, "world");

        // 交换全量状态更新：A 应用 B、B 应用 A。
        let a_update = a.encode().unwrap();
        let b_update = b.encode().unwrap();
        a.apply_update(&b_update).unwrap();
        b.apply_update(&a_update).unwrap();

        let ca = a.content();
        let cb = b.content();
        assert_eq!(ca, cb, "两文档内容必须字节一致（CRDT 收敛）");

        // 合并后应包含各方插入的内容（字符均被保留）。
        assert!(ca.contains("hello "), "应保留 A 的插入");
        assert!(ca.contains("world"), "应保留 B 的插入");

        // 幂等性：二次 merge_from 双向调用后仍然一致。
        a.merge_from(&b).unwrap();
        b.merge_from(&a).unwrap();
        assert_eq!(a.content(), b.content(), "二次合并后仍然一致");
    }

    #[test]
    fn edit_and_content_roundtrip() {
        let mut doc = CollabDoc::new();
        doc.edit(0, "Marguerite");
        assert_eq!(doc.content(), "Marguerite");
        doc.edit(0, "Hello ");
        assert_eq!(doc.content(), "Hello Marguerite");
    }

    #[test]
    fn apply_remote_update_merges_into_empty() {
        let mut a = CollabDoc::new();
        a.edit(0, "x");
        let update = a.encode().unwrap();

        let mut b = CollabDoc::new();
        b.apply_update(&update).unwrap();
        assert_eq!(b.content(), "x");
    }

    #[test]
    fn concurrent_inserts_interleave_deterministically() {
        // 多轮交替编辑后，所有副本应确定性收敛。
        let mut a = CollabDoc::new();
        let mut b = CollabDoc::new();

        a.edit(0, "A1");
        b.edit(0, "B1");
        a.edit(0, "A2");
        b.edit(0, "B2");

        // 双向各交换一次。
        let au = a.encode().unwrap();
        let bu = b.encode().unwrap();
        a.apply_update(&bu).unwrap();
        b.apply_update(&au).unwrap();

        assert_eq!(a.content(), b.content(), "多轮并发编辑后收敛");
        assert!(a.content().contains("A1"));
        assert!(a.content().contains("B1"));
    }
}
