# Marguerite 壁纸资产包（DSH 融合 v8 · 主题/个性化）

4 款 SVG 壁纸 + `manifest.json`（版本化资产清单：id/name/description/file/palette）。
由 `webui/public/wallpapers/` 静态托管（`/wallpapers/...`），皮肤中心（skin-center）
按 manifest 渲染「壁纸」面板，palette 字段与蓝橙语义令牌对账。

| id | 名称 | 主题 |
| --- | --- | --- |
| ink-wash | 水墨 | 默认水墨 skin（青灰渐变 + 墨色山影） |
| deep-space | 深空蓝橙 | 蓝橙语义令牌主题（品牌蓝 #3b82f6 + 橙 #f59e0b 星点） |
| sakura | 夜樱 | 个性化樱粉光晕 |
| whale-ocean | 鲸海 | 鲸鱼娘桌宠主题（深海渐变 + 气泡） |

约定：新增壁纸 = 新增一个 SVG + manifest 追加一条（version+1）；
palette 只放主题语义令牌（禁止裸 hex 漂移——对齐 webui check:architecture 的 theme-contract 守卫）。
