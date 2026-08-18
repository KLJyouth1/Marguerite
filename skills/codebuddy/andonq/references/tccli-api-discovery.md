# API 发现与检索（只读）

通过 curl + grep 检索腾讯云业务、**只读查询类接口**、数据结构。

> **限制**：仅允许发现和调用 `Describe*`、`List*`、`Get*` 这类只读接口；若需求最终落到 `Create*`、`Modify*`、`Delete*`、`Update*`、`Run*`、`Start*`、`Stop*`、`Reboot*` 等写入动作，统一回复：**功能规划中**。

## 发现业务

确定对应的 tccli 服务名（如 cvm、cbs）。

```sh
curl -s https://cloudcache.tencentcs.com/capi/refs/services.md | grep 云服务器
```

参考输出：

```
[cvm](service/cvm/index.md) | 云服务器 | 2017-03-12 | ...
```

## 检索接口

在业务接口列表中优先检索只读 Action（接口名即 tccli 的 `<Action>`）。

```sh
curl -s https://cloudcache.tencentcs.com/capi/refs/service/cvm/actions.md | grep "Describe\|List\|Get\|实例\|地域"
```

## 筛选只读接口

优先选择以下前缀：

- `Describe*`
- `List*`
- `Get*`

排除以下常见写入前缀：

- `Create*`
- `Run*`
- `Modify*`
- `Update*`
- `Delete*`
- `Start*`
- `Stop*`
- `Reboot*`
- `Reset*`
- `Bind*` / `Unbind*`
- `Associate*` / `Disassociate*`
- `Grant*` / `Revoke*`

## 阅读接口文档

确认参数、地域、版本（tccli 一般自动匹配版本）：

```sh
curl -s https://cloudcache.tencentcs.com/capi/refs/service/cvm/action/DescribeInstances.md
```

## 阅读数据结构

文档中涉及的数据结构可进一步查看：

```sh
curl -s https://cloudcache.tencentcs.com/capi/refs/service/cvm/model/Filter.md
```

## URL 模式总结

| 用途 | URL 模式 |
|------|---------|
| 服务列表 | `https://cloudcache.tencentcs.com/capi/refs/services.md` |
| 接口列表 | `https://cloudcache.tencentcs.com/capi/refs/service/<svc>/actions.md` |
| 接口文档 | `https://cloudcache.tencentcs.com/capi/refs/service/<svc>/action/<Action>.md` |
| 数据结构 | `https://cloudcache.tencentcs.com/capi/refs/service/<svc>/model/<Model>.md` |
