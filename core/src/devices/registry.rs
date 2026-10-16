//! 设备能力与危险等级（契约常量，WP1 填充真实逻辑）。

/// 危险操作分级（方案 §九 安全章）：
/// L0 只读 / L1 安全写自动 / L2 需审批 / L3 物理致动人在回路。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum HazardLevel {
    /// L0：只读查询，自动放行。
    ReadOnly = 0,
    /// L1：安全写（开灯），自动执行。
    SafeAuto = 1,
    /// L2：需审批（开空调、锁门）。
    RequiresApproval = 2,
    /// L3：物理致动 / 开锁 / 车辆 / 机械臂，fail-closed + 人在回路。
    PhysicalActuation = 3,
}

impl HazardLevel {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// 按 Home Assistant domain 自动分级（WP1 安全映射）。
    ///
    /// 物理致动默认 **fail-closed**：能直接改变机械状态的域（lock / garage_door）
    /// 一律判 L3，必须人在回路审批；未知域不臆测为「安全写」，而是落到 L2 要求
    /// 审批——宁可多问一次，也不把不明设备自动执行写操作。
    ///
    /// 映射表：
    /// - L3 物理致动：`lock`、`garage_door`
    /// - L2 需审批：`climate`、`cover`、`valve`
    /// - L1 安全写：`light`、`switch`、`fan`
    /// - L0 只读：`sensor`、`binary_sensor`
    /// - 其它（未知域）：L2（fail-closed 保守）
    pub fn classify(domain: &str) -> HazardLevel {
        match domain {
            "lock" | "garage_door" => HazardLevel::PhysicalActuation,
            "climate" | "cover" | "valve" => HazardLevel::RequiresApproval,
            "light" | "switch" | "fan" => HazardLevel::SafeAuto,
            "sensor" | "binary_sensor" => HazardLevel::ReadOnly,
            // fail-closed：未知域不允许自动写，要求审批。
            _ => HazardLevel::RequiresApproval,
        }
    }
}

/// 设备传输通道：本地优先，云端须显式标注「数据离网」。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Transport {
    Local,
    Cloud,
}

/// 单个设备/实体的能力描述（对齐 MCP tool/resource 原语 + 危险等级元数据）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Capability {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub read: bool,
    pub write: bool,
    pub hazard: HazardLevel,
    pub transport: Transport,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_physical_actuation_to_l3() {
        assert_eq!(HazardLevel::classify("lock"), HazardLevel::PhysicalActuation);
        assert_eq!(
            HazardLevel::classify("garage_door"),
            HazardLevel::PhysicalActuation
        );
        assert_eq!(HazardLevel::classify("lock").as_u8(), 3);
    }

    #[test]
    fn classify_maps_approval_domains_to_l2() {
        for d in ["climate", "cover", "valve"] {
            assert_eq!(HazardLevel::classify(d), HazardLevel::RequiresApproval, "domain={d}");
        }
    }

    #[test]
    fn classify_maps_safe_writes_to_l1() {
        for d in ["light", "switch", "fan"] {
            assert_eq!(HazardLevel::classify(d), HazardLevel::SafeAuto, "domain={d}");
        }
    }

    #[test]
    fn classify_maps_sensors_to_l0() {
        assert_eq!(HazardLevel::classify("sensor"), HazardLevel::ReadOnly);
        assert_eq!(
            HazardLevel::classify("binary_sensor"),
            HazardLevel::ReadOnly
        );
    }

    #[test]
    fn classify_unknown_domain_is_fail_closed_l2() {
        // 未知域不得自动放行写操作 —— fail-closed 落到需审批。
        assert_eq!(
            HazardLevel::classify("media_player"),
            HazardLevel::RequiresApproval
        );
        assert_eq!(HazardLevel::classify(""), HazardLevel::RequiresApproval);
    }
}
