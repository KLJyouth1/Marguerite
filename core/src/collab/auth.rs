//! 协作成员身份与基于角色的访问控制（RBAC）
//!
//! 角色：`owner`（全权）/ `maintainer`（可运行 Agent、改文件）/ `observer`（只读事件流）。
//! 约束：团队至少保留一名 `owner`。

use std::collections::HashMap;

use anyhow::{anyhow, Result};

/// 协作权限
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// 运行 Agent（触发一轮对话 / 执行链）
    RunAgent,
    /// 编辑项目文件
    EditFiles,
    /// 编辑协同文档的「块」级区域（比 EditFiles 更细粒度的块级写权限）
    EditBlock,
    /// 管理成员（增删 / 转让所有权）
    ManageMembers,
}

/// 成员角色
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 所有者（全权）
    Owner,
    /// 维护者（可运行 Agent、改文件）
    Maintainer,
    /// 观察者（只读事件流）
    Observer,
}

impl Role {
    /// 角色是否拥有某权限
    pub fn can(self, p: Permission) -> bool {
        match (self, p) {
            (Role::Owner, _) => true,
            (Role::Maintainer, Permission::RunAgent) => true,
            (Role::Maintainer, Permission::EditFiles) => true,
            (Role::Maintainer, Permission::EditBlock) => true,
            (Role::Maintainer, Permission::ManageMembers) => false,
            (Role::Observer, _) => false,
        }
    }
}

/// 便捷方法：角色是否可对协同文档做块级编辑（Track G 引入的 `EditBlock` 权限）
pub fn can_edit_block(role: Role) -> bool {
    role.can(Permission::EditBlock)
}

/// 团队成员
#[derive(Debug, Clone)]
pub struct Member {
    /// 成员唯一 ID
    pub id: String,
    /// 展示名
    pub name: String,
    /// 角色
    pub role: Role,
}

/// 协作鉴权中心：成员表 + 至少一名 owner 不变量
#[derive(Debug, Clone, Default)]
pub struct CollabAuth {
    members: HashMap<String, Member>,
}

impl CollabAuth {
    /// 以首个 owner 创建协作空间
    pub fn new(owner_id: &str, owner_name: &str) -> Self {
        let mut members = HashMap::new();
        members.insert(
            owner_id.to_string(),
            Member {
                id: owner_id.into(),
                name: owner_name.into(),
                role: Role::Owner,
            },
        );
        Self { members }
    }

    /// 成员数
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// 是否为空（仅用于测试）
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// 新增成员（ID 不可重复）
    pub fn add_member(&mut self, id: &str, name: &str, role: Role) -> Result<()> {
        if self.members.contains_key(id) {
            return Err(anyhow!("成员已存在: {id}"));
        }
        self.members.insert(
            id.into(),
            Member {
                id: id.into(),
                name: name.into(),
                role,
            },
        );
        Ok(())
    }

    /// 移除成员（owner 至少保留一名）
    pub fn remove_member(&mut self, id: &str) -> Result<Member> {
        let m = self
            .members
            .get(id)
            .ok_or_else(|| anyhow!("成员不存在: {id}"))?;
        if m.role == Role::Owner && self.owners_count() <= 1 {
            return Err(anyhow!("至少保留一名 owner"));
        }
        Ok(self.members.remove(id).unwrap())
    }

    /// 查询成员角色
    pub fn role_of(&self, id: &str) -> Option<Role> {
        self.members.get(id).map(|m| m.role)
    }

    /// 鉴权：成员是否拥有权限
    pub fn check(&self, id: &str, need: Permission) -> Result<()> {
        match self.role_of(id) {
            Some(role) if role.can(need) => Ok(()),
            Some(_) => Err(anyhow!("成员 {id} 无权限: {need:?}")),
            None => Err(anyhow!("成员不存在: {id}")),
        }
    }

    /// 转让所有权（from 必须是 owner，to 必须存在）
    pub fn transfer_ownership(&mut self, from: &str, to: &str) -> Result<()> {
        let from_role = self
            .role_of(from)
            .ok_or_else(|| anyhow!("转让方不存在: {from}"))?;
        if from_role != Role::Owner {
            return Err(anyhow!("仅 owner 可转让所有权"));
        }
        if self.role_of(to).is_none() {
            return Err(anyhow!("受让方不存在: {to}"));
        }
        if let Some(m) = self.members.get_mut(from) {
            m.role = Role::Maintainer;
        }
        if let Some(m) = self.members.get_mut(to) {
            m.role = Role::Owner;
        }
        Ok(())
    }

    fn owners_count(&self) -> usize {
        self.members
            .values()
            .filter(|m| m.role == Role::Owner)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_owner() {
        let auth = CollabAuth::new("u1", "Alice");
        assert_eq!(auth.len(), 1);
        assert_eq!(auth.role_of("u1"), Some(Role::Owner));
    }

    #[test]
    fn test_add_and_role_lookup() {
        let mut auth = CollabAuth::new("u1", "Alice");
        auth.add_member("u2", "Bob", Role::Maintainer).unwrap();
        assert_eq!(auth.role_of("u2"), Some(Role::Maintainer));
        // 重复添加失败
        assert!(auth.add_member("u2", "Bob2", Role::Observer).is_err());
    }

    #[test]
    fn test_role_permissions() {
        assert!(Role::Owner.can(Permission::ManageMembers));
        assert!(Role::Maintainer.can(Permission::RunAgent));
        assert!(Role::Maintainer.can(Permission::EditFiles));
        assert!(!Role::Maintainer.can(Permission::ManageMembers));
        assert!(!Role::Observer.can(Permission::RunAgent));
        assert!(!Role::Observer.can(Permission::EditFiles));
    }

    #[test]
    fn test_check_permission_flow() {
        let mut auth = CollabAuth::new("u1", "Alice");
        auth.add_member("u2", "Bob", Role::Observer).unwrap();
        assert!(auth.check("u1", Permission::ManageMembers).is_ok());
        assert!(auth.check("u2", Permission::RunAgent).is_err());
        assert!(auth.check("u2", Permission::EditFiles).is_err());
        assert_eq!(
            auth.check("u2", Permission::RunAgent)
                .unwrap_err()
                .to_string(),
            "成员 u2 无权限: RunAgent"
        );
    }

    #[test]
    fn test_edit_block_permission() {
        // 块级写权限：Owner 与 Maintainer 可编辑，Observer 仅可读。
        assert!(Role::Owner.can(Permission::EditBlock));
        assert!(Role::Maintainer.can(Permission::EditBlock));
        assert!(!Role::Observer.can(Permission::EditBlock));

        // 鉴权流：Observer 尝试 EditBlock 必须被拒绝。
        let mut auth = CollabAuth::new("u1", "Alice");
        auth.add_member("u2", "Bob", Role::Observer).unwrap();
        assert!(auth.check("u1", Permission::EditBlock).is_ok());
        let err = auth.check("u2", Permission::EditBlock).unwrap_err();
        assert_eq!(err.to_string(), "成员 u2 无权限: EditBlock");

        // Maintainer 通过。
        auth.add_member("u3", "Carol", Role::Maintainer).unwrap();
        assert!(auth.check("u3", Permission::EditBlock).is_ok());
    }

    #[test]
    fn test_remove_protects_last_owner() {
        let mut auth = CollabAuth::new("u1", "Alice");
        assert!(auth.remove_member("u1").is_err());
        auth.add_member("u2", "Bob", Role::Owner).unwrap();
        // 现在有两个 owner，可移除其一
        assert!(auth.remove_member("u1").is_ok());
        assert_eq!(auth.role_of("u1"), None);
    }

    #[test]
    fn test_transfer_ownership() {
        let mut auth = CollabAuth::new("u1", "Alice");
        auth.add_member("u2", "Bob", Role::Maintainer).unwrap();
        auth.transfer_ownership("u1", "u2").unwrap();
        assert_eq!(auth.role_of("u2"), Some(Role::Owner));
        assert_eq!(auth.role_of("u1"), Some(Role::Maintainer));
        // 非 owner 不能转让
        assert!(auth.transfer_ownership("u1", "u2").is_err());
    }

    #[test]
    fn observer_cannot_edit_block() {
        assert!(!Role::Observer.can(Permission::EditBlock));
        assert!(!can_edit_block(Role::Observer));
    }

    #[test]
    fn maintainer_can_edit_block() {
        assert!(Role::Maintainer.can(Permission::EditBlock));
        assert!(can_edit_block(Role::Maintainer));
        assert!(can_edit_block(Role::Owner));
    }
}
