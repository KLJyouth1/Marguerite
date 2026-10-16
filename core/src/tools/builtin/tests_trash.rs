//! T1 删除转回收站：工具层测试（delete_file 默认转回收站 / 永久删除显式动词 /
//! restore_from_trash / empty_trash）。

use super::*;

use super::tests::{setup, put};

/// 从成功消息 `moved to recycle bin: x (trash id 1, restorable ...)` 里抠出纯数字 id。
/// 消息里 id 后紧跟逗号（`trash id 1, restorable`），直接 split(' ') 会得到 `"1,"`。
fn trash_id(data: &str) -> String {
    let rest = data.split("trash id ").nth(1).unwrap_or("");
    rest.chars().take_while(|c| c.is_ascii_digit()).collect()
}

/// delete_file 默认行为：移入回收站（原路径消失、payload 在、id 可恢复）。
#[tokio::test]
async fn delete_file_default_moves_to_trash_and_is_restorable() {
    let (reg, dir) = setup();
    let mut a = HashMap::new();
    put(&mut a, "path", "hello.txt");
    let r = reg.execute_local_tool("delete_file", &a).await.unwrap();
    let data = match r {
        ToolResult::Success { data, .. } => data,
        other => panic!("delete_file 应成功: {other:?}"),
    };
    assert!(!dir.join("hello.txt").exists(), "原文件应已移入回收站");
    assert!(data.contains("recycle bin"), "结果应说明去了回收站: {data}");
    // 从消息里抠出 trash id
    let id = trash_id(&data);
    assert!(!id.is_empty(), "结果应带 trash id: {data}");
    let payload = dir.join(".marguerite/trash").join(&id).join("payload");
    assert!(payload.exists(), "回收站 payload 应存在: {}", payload.display());
    assert_eq!(std::fs::read_to_string(&payload).unwrap(), "line1\nline2\nline3\n");

    // 按 id 恢复
    let mut r2 = HashMap::new();
    put(&mut r2, "id", &id);
    let out = reg.execute_local_tool("restore_from_trash", &r2).await.unwrap();
    match out {
        ToolResult::Success { data, .. } => assert!(data.contains("restored")),
        other => panic!("restore_from_trash 应成功: {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("hello.txt")).unwrap(),
        "line1\nline2\nline3\n",
        "恢复后内容必须原样"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// delete_file_permanent：显式动词，真的永久删除，回收站零条目。
#[tokio::test]
async fn permanent_delete_leaves_no_trash_entry() {
    let (reg, dir) = setup();
    let mut a = HashMap::new();
    put(&mut a, "path", "hello.txt");
    let r = reg.execute_local_tool("delete_file_permanent", &a).await.unwrap();
    match r {
        ToolResult::Success { data, .. } => assert!(data.contains("permanently deleted")),
        other => panic!("permanent delete 应成功: {other:?}"),
    }
    assert!(!dir.join("hello.txt").exists());
    assert!(
        !dir.join(".marguerite/trash").exists() || std::fs::read_dir(dir.join(".marguerite/trash"))
            .map(|d| d.count())
            .unwrap_or(0)
            == 0,
        "永久删除不得在回收站留下条目"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 按原路径恢复（同路径多条记录取最新）。
#[tokio::test]
async fn restore_by_original_path_takes_latest_entry() {
    let (reg, dir) = setup();
    // 三轮：写 v1→删→写 v2→删→写 v3→删，然后按原路径恢复应得 v3
    // 注意：工具内部 safe_resolve 用 canonical 根解析（Windows 带 \\?\ 前缀），
    // 恢复按「原路径精确匹配」，故这里必须用 canonical 后的路径作为 id。
    let canon_dir = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
    for v in ["v1", "v2", "v3"] {
        std::fs::write(dir.join("hello.txt"), v).unwrap();
        let mut a = HashMap::new();
        put(&mut a, "path", "hello.txt");
        reg.execute_local_tool("delete_file", &a).await.unwrap();
    }
    let mut r = HashMap::new();
    put(&mut r, "id", &canon_dir.join("hello.txt").display().to_string());
    let out = reg.execute_local_tool("restore_from_trash", &r).await.unwrap();
    assert!(!matches!(out, ToolResult::Error { .. }), "按原路径恢复应成功");
    assert_eq!(
        std::fs::read_to_string(dir.join("hello.txt")).unwrap(),
        "v3",
        "同路径多条记录必须恢复最新一条"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 恢复冲突：绝不覆盖在位文件，安全命名恢复。
#[tokio::test]
async fn restore_conflict_never_overwrites_in_place_file() {
    let (reg, dir) = setup();
    let mut a = HashMap::new();
    put(&mut a, "path", "hello.txt");
    let del = reg.execute_local_tool("delete_file", &a).await.unwrap();
    let id = match del {
        ToolResult::Success { ref data, .. } => trash_id(data),
        ref other => panic!("{other:?}"),
    };
    assert!(!id.is_empty());
    // 在位放一个新文件占位
    std::fs::write(dir.join("hello.txt"), "in-the-way").unwrap();

    let mut r = HashMap::new();
    put(&mut r, "id", &id);
    let out = reg.execute_local_tool("restore_from_trash", &r).await.unwrap();
    let data = match out {
        ToolResult::Success { data, .. } => data,
        ref other => panic!("冲突恢复应成功（安全命名）: {other:?}"),
    };
    assert!(data.contains("安全改名"), "应提示冲突改名: {data}");
    assert_eq!(
        std::fs::read_to_string(dir.join("hello.txt")).unwrap(),
        "in-the-way",
        "在位文件绝不允许被回收站内容覆盖"
    );
    let restored = dir.join("hello (restored-1).txt");
    assert_eq!(
        std::fs::read_to_string(&restored).unwrap(),
        "line1\nline2\nline3\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// empty_trash 必须 confirm=true 二次确认。
#[tokio::test]
async fn empty_trash_requires_explicit_confirm() {
    let (reg, dir) = setup();
    let mut a = HashMap::new();
    put(&mut a, "path", "hello.txt");
    reg.execute_local_tool("delete_file", &a).await.unwrap();

    let mut no = HashMap::new();
    put(&mut no, "confirm", "false");
    let r = reg.execute_local_tool("empty_trash", &no).await.unwrap();
    match r {
        ToolResult::Error { code, .. } => assert_eq!(code, "INVALID_PARAMS"),
        ref other => panic!("缺 confirm 必须拒绝: {other:?}"),
    }
    let mut missing = HashMap::new();
    let r2 = reg.execute_local_tool("empty_trash", &missing).await.unwrap();
    assert!(r2.is_error(), "完全缺失 confirm 也必须拒绝");

    put(&mut missing, "confirm", "true");
    let r3 = reg.execute_local_tool("empty_trash", &missing).await.unwrap();
    match r3 {
        ToolResult::Success { data, .. } => assert!(data.contains("1 entries")),
        ref other => panic!("confirm=true 应成功清空: {other:?}"),
    }
    assert!(!dir.join("hello.txt").exists(), "清空回收站不等于恢复文件");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 永久删除与默认删除都仍受 L3 凭证目录护栏约束。
#[tokio::test]
async fn permanent_delete_still_guarded_by_l3() {
    let (reg, dir) = setup();
    std::fs::create_dir_all(dir.join(".ssh")).unwrap();
    std::fs::write(dir.join(".ssh/id_rsa"), "KEY").unwrap();
    let mut a = HashMap::new();
    put(&mut a, "path", ".ssh/id_rsa");
    let r = reg
        .execute_local_tool("delete_file_permanent", &a)
        .await
        .unwrap();
    assert!(r.is_error(), "删除凭证目录内文件必须被 L3 拒绝");
    assert!(dir.join(".ssh/id_rsa").exists(), "文件不得被删除");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目录删除 → 回收站 → 恢复整棵目录。
#[tokio::test]
async fn delete_directory_roundtrip_via_tools() {
    let (reg, dir) = setup();
    std::fs::create_dir_all(dir.join("d/sub")).unwrap();
    std::fs::write(dir.join("d/one.txt"), "1").unwrap();
    std::fs::write(dir.join("d/sub/two.txt"), "22").unwrap();

    let mut a = HashMap::new();
    put(&mut a, "path", "d");
    let del = reg.execute_local_tool("delete_file", &a).await.unwrap();
    assert!(!del.is_error());
    assert!(!dir.join("d").exists());

    let id = match del {
        ToolResult::Success { ref data, .. } => trash_id(data),
        ref other => panic!("{other:?}"),
    };
    assert!(!id.is_empty());
    let mut r = HashMap::new();
    put(&mut r, "id", &id);
    assert!(!reg
        .execute_local_tool("restore_from_trash", &r)
        .await
        .unwrap()
        .is_error());
    assert_eq!(std::fs::read_to_string(dir.join("d/sub/two.txt")).unwrap(), "22");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 恢复不存在的条目：响亮失败，不乱恢复。
#[tokio::test]
async fn restore_unknown_entry_errors() {
    let (reg, dir) = setup();
    let mut r = HashMap::new();
    put(&mut r, "id", "9999");
    let out = reg.execute_local_tool("restore_from_trash", &r).await.unwrap();
    match out {
        ToolResult::Error { code, message } => {
            assert_eq!(code, "RESTORE_FAILED");
            assert!(message.contains("not found"), "错误应说明条目不存在: {message}");
        }
        ref other => panic!("未知 id 必须拒绝: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 路径逃逸（`..` 跳出工作区）：delete_file 必须在 safe_resolve 阶段就拒绝，
/// 绝不能把工作区外的文件移进本工作区回收站。外部文件保持原位。
///
/// 守卫的是「L3 路径门在删除链路上真的拦得住逃逸」这一属性——
/// trash 层自己的 restore 穿越测试覆盖的是恢复侧，这里覆盖删除入口侧。
#[tokio::test]
async fn delete_file_refuses_path_escape_and_touches_nothing_outside() {
    let (reg, dir) = setup();
    // 在工作区隔壁造一个「工作区外」文件
    let sibling = dir
        .parent()
        .unwrap()
        .join(format!(
            "mgt_escape_out_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("secret.txt"), "keep me outside").unwrap();
    let rel_path = format!(
        "../{}/secret.txt",
        sibling.file_name().unwrap().to_string_lossy()
    );

    let mut a = HashMap::new();
    put(&mut a, "path", &rel_path);
    // safe_resolve 在工具入口即拒绝逃逸，错误以外层 Result::Err 抛出（fail-closed）。
    match reg.execute_local_tool("delete_file", &a).await {
        Ok(other) => panic!("逃逸删除必须失败，却返回了结果: {other:?}"),
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("escapes"),
                "错误必须点明路径逃逸被拒: {msg}"
            );
        }
    }
    assert!(
        sibling.join("secret.txt").exists(),
        "工作区外文件绝不能被删除或移走"
    );
    // 回收站里也不应留下任何条目
    let trash_root = dir.join(".marguerite/trash");
    let leftover = if trash_root.exists() {
        std::fs::read_dir(&trash_root)
            .map(|d| d.filter_map(|e| e.ok()).count())
            .unwrap_or(0)
    } else {
        0
    };
    assert_eq!(leftover, 0, "逃逸删除不得在回收站留下条目");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&sibling);
}

/// 删除一个不存在的目标：错误透传，绝不因为「删不掉」而走任何静默硬删分支。
#[tokio::test]
async fn delete_file_missing_target_errors_without_hard_delete() {
    let (reg, dir) = setup();
    let mut a = HashMap::new();
    put(&mut a, "path", "does_not_exist.txt");
    // 删除不存在目标：错误响亮抛出（外层 Result::Err），绝不静默假装成功或走硬删分支。
    match reg.execute_local_tool("delete_file", &a).await {
        Ok(other) => panic!("删除不存在目标必须报错，而非假装成功: {other:?}"),
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("not found") || msg.contains("target"),
                "错误应说明目标不存在: {msg}"
            );
        }
    }
    // 工作区其它文件未受影响
    assert!(dir.join("hello.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
