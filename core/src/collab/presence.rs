//! 在场（presence）同步 —— 基于 yrs::Awareness 的光标 / 选区广播（Track G 地基）。
//!
//! 协同层地基只负责「谁在编辑哪里」的实时状态交换：本地设置光标，编码为
//! awareness 更新广播给对端，对端解码后即可在 UI 渲染远端光标。网络层与 UI
//! 由后续 track 在其上构建。本模块保持零业务依赖、可单测。

use anyhow::{anyhow, Result};
use serde_json::Value;
use yrs::sync::{Awareness, AwarenessUpdate};
use yrs::Doc;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;

/// 在场状态：包裹 yrs::Awareness（内部持有一个独立 Doc 以确保 awareness 生命周期有效）。
///
/// 本地光标经 [`CollabPresence::set_cursor`] 写入，编码为更新后由网络层广播；
/// 对端的更新经 [`CollabPresence::apply_remote`] 应用，再经 [`CollabPresence::peers`]
/// 读取全部远端客户端状态。
pub struct CollabPresence {
    awareness: Awareness,
}

impl CollabPresence {
    /// 创建空的在场状态（自带独立 Doc）。
    pub fn new() -> Self {
        let doc = Doc::new();
        let awareness = Awareness::new(doc);
        Self { awareness }
    }

    /// 本端客户端 ID（用于区分本地与远端 peer）。
    pub fn client_id(&self) -> u64 {
        self.awareness.client_id().get()
    }

    /// 设置本端光标位置（字段 + 字符索引），写入本地 awareness 状态。
    ///
    /// 状态结构：`{"cursor": {"field": "<str>", "pos": <int>}}`。
    pub fn set_cursor(&mut self, field: &str, position: usize) {
        // serde_json::Value 可序列化，set_local_state 内部会以 JSON 形式落库并广播。
        let state = serde_json::json!({ "cursor": { "field": field, "pos": position } });
        let _ = self.awareness.set_local_state(state);
    }

    /// 编码本地 awareness 更新，供网络层广播给对端。
    pub fn encode(&self) -> Result<Vec<u8>> {
        let upd = self
            .awareness
            .update()
            .map_err(|e| anyhow!("awareness update failed: {e}"))?;
        Ok(upd.encode_v1())
    }

    /// 应用对端广播来的 awareness 更新，合并进本地 peer 表。
    pub fn apply_remote(&mut self, update: &[u8]) -> Result<()> {
        let upd = AwarenessUpdate::decode_v1(update)
            .map_err(|e| anyhow!("decode awareness failed: {e}"))?;
        self.awareness
            .apply_update(upd)
            .map_err(|e| anyhow!("apply awareness failed: {e}"))?;
        Ok(())
    }

    /// 返回全部远端 peer 的 (client_id, 状态 JSON)。
    ///
    /// 已排除本端 client_id（[`CollabPresence::client_id`]）。
    pub fn peers(&self) -> Vec<(u64, Value)> {
        let local = self.awareness.client_id();
        self.awareness
            .iter()
            .filter(|(id, _)| *id != local)
            .map(|(id, st)| {
                let value = st
                    .data
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .unwrap_or(Value::Null);
                (id.get(), value)
            })
            .collect()
    }
}

impl Default for CollabPresence {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在场收敛性证明：A 设置光标后广播更新，B 应用后必须能看到 A 的 client_id 与光标。
    #[test]
    fn presence_converges() {
        let mut a = CollabPresence::new();
        let mut b = CollabPresence::new();

        a.set_cursor("main.rs", 42);

        let update = a.encode().unwrap();
        b.apply_remote(&update).unwrap();

        let peers = b.peers();
        assert_eq!(peers.len(), 1, "B 应看到恰好 1 个远端 peer");
        let (cid, state) = &peers[0];
        assert_eq!(*cid, a.client_id(), "peer id 应为 A 的 client_id");

        let cursor = state
            .get("cursor")
            .expect("远端状态应包含 cursor 字段");
        assert_eq!(
            cursor.get("field").and_then(|v| v.as_str()),
            Some("main.rs"),
            "远端光标字段应匹配"
        );
        assert_eq!(
            cursor.get("pos").and_then(|v| v.as_i64()),
            Some(42),
            "远端光标位置应匹配"
        );
    }

    #[test]
    fn local_peer_is_excluded_from_peers() {
        let a = CollabPresence::new();
        // 本端未设光标、也无远端更新时，peers 应为空（不含自身）。
        assert!(a.peers().is_empty(), "无远端时 peers 为空");
    }

    #[test]
    fn multiple_remote_peers_visible() {
        let mut a = CollabPresence::new();
        let mut c = CollabPresence::new();
        let mut b = CollabPresence::new();

        a.set_cursor("a.rs", 1);
        c.set_cursor("c.rs", 9);

        let au = a.encode().unwrap();
        let cu = c.encode().unwrap();
        b.apply_remote(&au).unwrap();
        b.apply_remote(&cu).unwrap();

        let ids: Vec<u64> = b.peers().iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), 2, "B 应看到 2 个远端 peer");
        assert!(ids.contains(&a.client_id()));
        assert!(ids.contains(&c.client_id()));
    }
}
