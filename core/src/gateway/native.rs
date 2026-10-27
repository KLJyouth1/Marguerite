//! 原生娱乐功能方法族（从 methods.rs 分治迁出）
//!
//! 鲸鱼娘桌宠（`pet.*`）、德州扑克（`poker.*`）与内置小游戏（`games.*`）。
//! 这些是 Extras 扩展域的实现，与核心网关路由（chat/config/sessions/plans）
//! 完全解耦；分发臂留在 `methods::dispatch_rust`（`"poker.*" => native::…`），
//! 契约守卫 #4 的源扫描仍然覆盖这些臂。

use serde_json::{json, Value};

use crate::gateway::_helpers::{ok_or_err, param_i64, param_str};

// ── 鲸鱼娘 ──

/// `pet.action`：桌宠交互（摸头/投喂等，affinity 持久化）。
pub(crate) fn pet_action(params: &Value) -> Value {
    let action = param_str(params, "action", "action", "");
    ok_or_err(crate::workbench::pet::interact(action))
}

// ── 德州扑克 ──

/// `poker.state`：读取房间当前状态。
pub(crate) fn poker_state(params: &Value) -> Value {
    let room_id = param_str(params, "roomId", "room_id", "");
    let hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    match hub.room(room_id) {
        Some(r) => serde_json::to_value(r).unwrap_or(json!({ "ok": false, "error": "序列化失败" })),
        None => json!({ "ok": false, "error": "房间不存在" }),
    }
}

/// `poker.create`：创建房间。
pub(crate) fn poker_create(params: &Value) -> Value {
    let name = param_str(params, "name", "name", "牌局").to_string();
    let sb = param_i64(params, "smallBlind", "small_blind", 5);
    let bb = param_i64(params, "bigBlind", "big_blind", 10);
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    let id = hub.create_room(name, sb, bb);
    json!({ "ok": true, "roomId": id })
}

/// `poker.join`：加入房间。
pub(crate) fn poker_join(params: &Value) -> Value {
    let room_id = param_str(params, "roomId", "room_id", "");
    let name = param_str(params, "playerName", "player_name", "玩家").to_string();
    let tokens = param_i64(params, "tokens", "tokens", 1000);
    let is_human = params
        .get("isHuman")
        .or_else(|| params.get("is_human"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    ok_or_err(hub.join_room(room_id, name, tokens, is_human))
}

/// `poker.start`：开局。
pub(crate) fn poker_start(params: &Value) -> Value {
    let room_id = param_str(params, "roomId", "room_id", "");
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    ok_or_err(hub.start_room(room_id))
}

/// `poker.action`：玩家动作（fold/check/call/raise/all_in）。
pub(crate) fn poker_do_action(params: &Value) -> Value {
    use crate::poker::ActionKind;
    let room_id = param_str(params, "roomId", "room_id", "");
    let seat = params.get("seat").and_then(Value::as_u64).unwrap_or(0) as u32;
    let action = param_str(params, "action", "action", "");
    let amount = params.get("amount").and_then(Value::as_i64);
    let kind = match action {
        "fold" => ActionKind::Fold,
        "check" => ActionKind::Check,
        "call" => ActionKind::Call,
        "raise" => ActionKind::Raise,
        "all_in" => ActionKind::AllIn,
        other => return json!({ "ok": false, "error": format!("未知动作: {other}") }),
    };
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    ok_or_err(hub.act(room_id, seat, kind, amount))
}

/// `poker.auto`：AI 自动行牌至人类回合/牌局结束（有对加注、无对弃牌的朴素策略）。
pub(crate) fn poker_auto(params: &Value) -> Value {
    use crate::poker::{evaluate_7, ActionKind};
    let room_id = param_str(params, "roomId", "room_id", "");
    let mut hub = match crate::poker::global_hub().lock() {
        Ok(h) => h,
        Err(_) => return json!({ "ok": false, "error": "大厅锁获取失败" }),
    };
    for _ in 0..64 {
        let (hand_over, cur, is_human, to_call, tokens, has_pair, street_high, min_raise) = {
            let room = match hub.room(room_id) {
                Some(r) => r,
                None => return json!({ "ok": false, "error": "房间不存在" }),
            };
            // 座位越界必须返回错误而不是 panic：网关是常驻进程，一次下标越界
            // 会把整个 daemon 带走（房间状态损坏时 `current_seat` 可能指向空位）。
            let Some(p) = room.players.get(room.current_seat as usize) else {
                return json!({
                    "ok": false,
                    "code": "SEAT_OUT_OF_RANGE",
                    "error": "当前座位越界（房间状态异常）",
                });
            };
            let has_pair = match (p.hole[0], p.hole[1]) {
                (Some(a), Some(b)) => evaluate_7([a, b], &room.board).category >= 1,
                _ => false,
            };
            (
                room.hand_over,
                room.current_seat,
                p.is_human,
                room.street_high - p.street_committed,
                p.tokens,
                has_pair,
                room.street_high,
                room.min_raise,
            )
        };
        if hand_over {
            return json!({ "ok": true });
        }
        if is_human {
            return json!({ "ok": true });
        }
        let (action, amount) = if to_call <= 0 {
            if has_pair && tokens > min_raise * 2 {
                (ActionKind::Raise, Some(street_high + min_raise))
            } else {
                (ActionKind::Check, None)
            }
        } else if to_call >= tokens {
            if has_pair {
                (ActionKind::AllIn, None)
            } else {
                (ActionKind::Fold, None)
            }
        } else {
            (ActionKind::Call, None)
        };
        if let Err(e) = hub.act(room_id, cur, action, amount) {
            return json!({ "ok": false, "error": e });
        }
    }
    json!({ "ok": true })
}

// ── 内置小游戏 ──

/// `games.list`：返回内置小游戏目录（与 CLI/TUI/games::catalog 单一事实源一致）。
pub(crate) fn games_list() -> Value {
    let items: Vec<Value> = crate::games::catalog()
        .iter()
        .map(|g| {
            json!({
                "id": g.id,
                "name": g.name,
                "controls": g.controls,
                "status": "implemented",
            })
        })
        .collect();
    json!({ "total": items.len(), "games": items })
}

/// `games.play`：动作驱动运行小游戏引擎，渲染棋盘/结果；玩法错误据实返回。
pub(crate) fn games_play(params: &Value) -> Value {
    let game = param_str(params, "game", "gameId", "").to_string();
    let moves: Vec<&str> = params
        .get("moves")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if game.is_empty() {
        return json!({ "ok": false, "error": "缺少 game 参数" });
    }
    match crate::games::run_game(&game, &moves) {
        Ok(render) => json!({
            "ok": true,
            "game": game,
            "moves": moves,
            "render": render,
        }),
        Err(e) => json!({ "ok": false, "error": e, "game": game, "moves": moves }),
    }
}

#[cfg(test)]
mod poker_auto_guard_tests {
    use super::*;
    use serde_json::json;

    /// 空房 / 损坏的 current_seat 曾让 `room.players[idx]` 直接 panic ——
    /// 网关是常驻进程，一次下标越界会把整个 daemon 带走。这里钉住「回错误、不 panic」。
    #[test]
    fn auto_on_an_empty_room_returns_an_error_instead_of_panicking() {
        let room_id = {
            let mut hub = crate::poker::global_hub().lock().expect("hub lock");
            hub.create_room("guard-test".into(), 5, 10)
        };
        let out = poker_auto(&json!({ "roomId": room_id }));
        assert_eq!(out["ok"], false);
        assert_eq!(out["code"], "SEAT_OUT_OF_RANGE");
        // 失败不得毒化 hub：后续调用仍能拿到锁。
        assert!(crate::poker::global_hub().lock().is_ok());
    }

    #[test]
    fn auto_on_a_missing_room_still_reports_not_found() {
        let out = poker_auto(&json!({ "roomId": "room_does_not_exist" }));
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap_or("").contains("不存在"));
    }
}
