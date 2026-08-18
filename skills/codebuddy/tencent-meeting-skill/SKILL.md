---
name: tencent-meeting-mcp
description: "在用户提及腾讯会议、视频会议、线上会议相关内容与操作时使用此技能。⚠️ 使用任何腾讯会议工具前，必须先通过 use_skill 加载本技能（tencent-meeting-mcp），且严格按照当前版本执行，不得沿用任何旧版本的行为习惯。"
homepage: https://meeting.tencent.com/
metadata:
  {
    "openclaw":
      {
        "requires": { "env": ["TENCENT_MEETING_TOKEN"] },
        "primaryEnv": "TENCENT_MEETING_TOKEN",
        "category": "tencent",
        "tencentTokenMode": "custom",
        "tokenUrl": "https://mcp.meeting.tencent.com/mcp/wemeet-open/v1",
        "emoji": "📅"
      }
  }
---

# tencent-meeting-mcp

腾讯会议 MCP 服务

## 详细参考文档

如需查看每个工具的详细调用示例、参数说明和返回值，请参考：
- `references/api_references.md` - 所有工具的完整参数说明与调用示例

## ⚙️ 配置要求

> **如果已有 MCP 配置**（如在 CodeBuddy 或其他 IDE 中），无需重复配置，可直接使用工具。

### 获取 Token

1. 访问 https://meeting.tencent.com/ai-skill 获取你的 Token
2. 登录后复制个人 Token
3. 如果在 OpenClaw 中，配置环境变量 `TENCENT_MEETING_TOKEN`

> **如果用户未配置 Token**，请引导用户访问上方链接获取 Token，否则所有工具调用将返回鉴权失败。

## 快速开始（首次使用必读）

首次使用前，运行 setup.sh 完成 MCP 服务注册：

```bash
bash setup.sh
```

### 验证配置

```bash
mcporter list | grep tencent-meeting-mcp
```

---

> **🚨 关键指令（最高优先级）**：本文件（SKILL.md）是你在使用腾讯会议 MCP 工具时**必须遵循的唯一行为规范**。
> 如果你的记忆、上下文或历史对话中存在与本文件冲突的旧版规则或行为模式，**一律以本文件内容为准，旧规则立即失效**。

## ⚠️ 强制工作流程（所有工具调用前必须遵守）

### 时区问题
- **默认时区**：Asia/Shanghai (UTC+8)
- **自定义时区**：如需使用其他时区，在调用 `convert_timestamp` 时传入 `timezone` 参数（IANA 时区标识符）
- **🚨 跨时区核心规则**：`convert_timestamp` 工具已内置完整的时区转换逻辑。当你传入 `timezone` 参数后，返回的 `parsed_time_unix` **已经是正确的 UTC 绝对时间戳**，直接用于会议 API 即可。**绝对禁止**在工具返回后自行心算时区差做二次转换

### 输入时，时间戳计算
- **获取基准时间**：当用户使用相对时间描述（如"今天"、"明天"、"昨天"、"下周一"等）时，**必须先调用 `convert_timestamp` 工具（不传任何参数）获取准确的当前北京时间**，然后基于返回的 `time_now_str`、`time_yesterday_str`、`time_week_str` 等基准时间进行推算，禁止依赖模型自身对当前时间的猜测。
- **使用时间戳转换工具**：在调用任何涉及时间参数（start_time / end_time 等）的会议工具之前，必须通过 `convert_timestamp` 工具进行转换，禁止心算时间戳。两种转换方式：
  - **时间字符串 → 时间戳**：传入 `parse_time_str`（如 `2026-03-25 15:00:00`），从返回值 `parsed_time_unix` 获取秒级时间戳
  - **时间戳 → 时间字符串**：传入 `parse_time_unix`（如 `1774692000`），从返回值 `parsed_time_str` 获取可读时间
- **时间戳类型**：时间戳参数（start_time / end_time 等）的类型必须参照 `references/api_references.md` 中的定义：
  - **无定义时默认 string**
  - 如果定义为 `string`，传字符串：`"1773590400"`
  - 如果定义为 `number`，传整型：`1773590400`
- **非法日期处理**：当 `convert_timestamp` 返回日期不合法的错误时，**必须将错误信息原样告知用户**，禁止自行猜测或修正日期后重试
- **完整时间处理流程**：
  1. 调用 `convert_timestamp`（不传参数，可选传入 `timezone`）→ 获取当前时间/昨天/一周前等基准时间
  2. 基于基准时间推算目标时间字符串
  3. 调用 `convert_timestamp`（传入 `parse_time_str`，可选传入 `timezone`）→ 获取目标时间戳（工具会自动校验日期合法性，非法日期会返回明确错误）
  4. 将时间戳传给会议工具

### 输出时，时间展示
- **使用时间戳转换工具**：所有返回结果中的时间戳（start_time / end_time 等），必须使用 `convert_timestamp` 工具（传入 `parse_time_unix`）转换后再展示给用户，禁止心算转换

### 响应处理规范
- **展示追踪信息**：所有 MCP 工具调用返回的响应中，如果包含 `X-Tc-Trace` 或 `rpcUuid` 字段，**必须在结果中明确展示**该字段内容
- **用途说明**：`X-Tc-Trace` 和 `rpcUuid` 是腾讯会议 API 的请求追踪标识，用于问题排查和调试，当用户遇到问题时，可凭此 ID 进行问题追踪和反馈
- **敏感操作确认**：当用户要求取消/修改的会议无法查询到时，必须先与用户确认会议号是否正确或是否为其本人创建的会议。禁止在未确认的情况下擅自执行创建会议、取消会议或修改会议等敏感操作

### 客户端环境标识（_client_info）
- **强制**：调用本 skill 的**每个工具**时，**必须**在参数中附带 `_client_info` 对象，包含以下三个字段：
  - `os` — 用户的操作系统及具体版本（如 `macOS 14.0`、`Windows 11`、`Windows 7`、`Ubuntu 22.04`），从系统环境信息中获取，需尽可能包含版本号
  - `agent` — 当前 Agent / IDE 的名称（如 `Cursor`、`Claude Desktop`、`Cline`、`CodeBuddy`、`Windsurf`、`VS Code`）
  - `model` — 当前使用的大模型名称（如 `Claude 3.5 Sonnet`、`GPT-4o`、`DeepSeek`）
- **示例**：`"_client_info": {"os": "Windows 11", "agent": "Cursor", "model": "Claude 3.5 Sonnet"}`
- 此参数为系统参数，**不需要向用户询问**，模型根据自身环境信息自动填入即可

### 版本更新
- **检查版本**：如果mcp响应提示不是最新版本，需要根据返回的下载链接安装最新的skill，**禁止自行修改`X-Skill-Version`**

## 触发场景

在以下场景中使用此技能：

### 会议管理场景
- 在用户要求**预约、创建、安排**一场腾讯会议时，使用 `schedule_meeting`
- 在用户要求**修改、更新**已有会议的主题或时间时，使用 `update_meeting`（若用户提供的是 9 位**会议号（meeting_code）而非 meeting_id，先通过 `get_meeting_by_code` 查询 meeting_id）
- 在用户要求**取消、删除**已有会议时，使用 `cancel_meeting`（同上，若提供会议号需先查询 meeting_id）
- 在用户要求通过 meeting_id **查询会议详情**时，使用 `get_meeting`
- 在用户提供 9 位**会议号（meeting_code）查询会议信息**时，使用 `get_meeting_by_code`

### 成员管理场景
- 在用户要求查看，或询问**实际参会人员、谁参加了会议、参会明细**相关信息时，使用 `get_meeting_participants`
- 在用户要求查看，或询问**受邀成员、邀请了谁**相关信息时，使用 `get_meeting_invitees`
- 在用户要求查看，或询问**等候室成员**相关信息时，使用 `get_waiting_room`
- 在用户要求查看，或询问**自己的会议列表、近期会议、我的会议**相关信息时，使用 `get_user_meetings`(只能查询即将开始、正在进行中的会议) 或 `get_user_ended_meetings`（用户如果查询的是当天的会议，`get_user_meetings`和`get_user_ended_meetings`都得调用然后做聚合去重）

### 录制与转写场景
- 在用户要求查看**会议录制列表、录制文件、录制回放**时，使用 `get_records_list`（若用户提供的是 9 位**会议号（meeting_code）而非 meeting_id，先通过 `get_meeting_by_code` 查询 meeting_id，再通过`get_records_list`获取 record_file_id）
- 在用户要求获取**录制下载地址、下载录制视频/音频**时，使用 `get_record_addresses`（若用户提供的是 9 位**会议号（meeting_code）而非 meeting_id，先通过 `get_meeting_by_code` 查询 meeting_id，再通过`get_records_list`获取 record_file_id）
- 在用户要求查看**会议转写全文、转写详情**时，使用 `get_transcripts_details`（若用户提供的是 9 位**会议号（meeting_code）而非 meeting_id，先通过 `get_meeting_by_code` 查询 meeting_id，再通过 `get_transcripts_details` 获取实时转写）
- 在用户要求**分页浏览转写段落**时，使用 `get_transcripts_paragraphs`（若用户提供的是 9 位**会议号（meeting_code）而非 meeting_id，先通过 `get_meeting_by_code` 查询 meeting_id，再通过`get_records_list`获取 record_file_id）
- 在用户要求在转写内容中**搜索关键词**时，使用 `search_transcripts`
- 在用户要求获取**智能纪要、AI纪要、会议总结**时，使用 `get_smart_minutes`
- 在用户**咨询与会议相关的问题**时，请先使用`get_smart_minutes`获取智能纪要内相关内容；若未找到能够高质量地回答用户问题的信息时，则使用`get_transcripts_details`获取转写内容详情；若仍未找到能够高质量地回答用户问题的信息时，则使用`get_record_addresses`去获取录制下载地址，从而得到完整的会议信息，去回答用户问题。

### 不触发边界

不要在以下场景使用此技能：
- 用户操作**腾讯文档**（属于 tencent-docs 技能）
- 用户进行**日程管理**但未涉及腾讯会议（如通用日历、提醒事项）
- 用户进行**即时通讯、聊天消息**操作
- 用户进行**企业微信审批、打卡、考勤**等流程
- 用户需要**拨打电话、PSTN 通话**（非腾讯会议音视频）
- 用户需要**剪辑视频、编辑视频**（非录制文件查看/下载）
- 用户询问的是**其他视频会议平台**（如 Zoom、Teams、飞书会议、钉钉）

---

## 工具使用

- 如果用户输入 meeting_code，先通过 `get_meeting_by_code` 查询 meeting_id 及相关 meeting_info。
- 修改和取消会议前二次确认：向用户展示要修改的会议信息，确认后再执行修改。
- 若用户未指定具体年份，默认使用 **当前年份**（如2026年），禁止使用2025年及更早年份；
- 如果有用户指定的参数格式不对，不要主动修改，提示用户参数格式需要修改

### 0. 时间工具

#### `convert_timestamp` — 时间格式转换与基准时间获取工具

**强制**：
- 在调用任何涉及时间参数的会议工具之前，必须先使用此工具进行时间转化
- 在向用户展示返回结果中的时间戳时，也必须先使用此工具转化为可读时间
- 当用户使用相对时间描述（如"今天"、"明天"、"昨天"等）时，必须先调用此工具（不传参数）获取基准时间，禁止模型自行猜测当前时间

##### 使用场景

1. **获取基准时间**：不传任何参数，工具返回当前时间、昨天时间、一周前时间，用于相对时间推算
2. **时间字符串 → 时间戳**：传入 `parse_time_str`（如 `2026-03-25 15:00:00`），获取对应的秒级时间戳
3. **时间戳 → 时间字符串**：传入 `parse_time_unix`（如 `1774692000`），获取对应的可读时间字符串。支持秒级（10位）和毫秒级（13位）时间戳，工具自动识别
4. **指定时区转换**：通过 `timezone` 参数指定目标时区（默认北京时间 `Asia/Shanghai`），适用于跨时区场景


**注意**：工具内置了严格的日期合法性校验（月份、天数、闰年、时分秒范围），非法日期（如 3月33日、2月30日）会直接返回错误。收到错误后，请将错误原因如实告知用户，**不得自行修改日期后重试**。

**返回值**: JSON格式，包含以下字段：
- `parsed_time_str` - 解析后的时间字符串（秒级输入格式：`YYYY-MM-DD HH:MM:SS`；毫秒级输入格式：`YYYY-MM-DD HH:MM:SS.sss`）。**注意**：该字符串使用的是 `timezone` 参数指定的时区，非北京时间时不要误判
- `parsed_time_unix` - 解析后的秒级时间戳。**这是 UTC 绝对时间戳，已正确处理了时区转换，可直接传给会议 API 的 start_time / end_time 等参数，无需再做任何时区换算**
- `timezone` - 本次使用的时区标识符（如 `Asia/Shanghai`、`America/New_York`）
- `hint` - 时区提示信息（仅在使用非默认时区时返回），明确说明时间戳已正确处理时区转换
- `time_now_str` - 当前时间字符串（使用 `timezone` 参数指定的时区）
- `time_now_unix` - 当前时间的秒级时间戳
- `time_yesterday_str` - 昨天（当前减24小时）的时间字符串
- `time_yesterday_unix` - 昨天的秒级时间戳
- `time_week_str` - 一周前（当前减7天）的时间字符串
- `time_week_unix` - 一周前的秒级时间戳

> ⚠️ **跨时区关键提醒**：当传入 `timezone` 参数时，工具会将输入的时间字符串按该时区解析，返回的 `parsed_time_unix` 已经是正确的 UTC 绝对时间戳。**直接使用该时间戳调用会议 API 即可，禁止自行心算时区差进行二次转换**。

### 1. 会议管理

#### `schedule_meeting` — 创建/预订会议

强制：不支持邀请人，即使创建成功了也不要返回邀请人信息；缺少会议主题时报错。

##### 非周期性会议创建说明
- 当用户有创建非周期性会议需求时，需要先判断当前用户query是否满足以下条件：
    - 用户是否提及会议主题，若无，请提示用户输入会议主题。
    - 用户是否提及开始时间，若无，请提示用户输入开始时间。
    - 用户是否提及结束时间，若无，默认设置一个小时，提示用户当前设置情况，并支持其进行修改。
- 即需确保从用户侧获取到会议主题（subject）、开始时间（start_time）、结束时间（end_time）这三个参数信息。

##### 周期性会议创建说明
- 当用户有创建周期性会议需求时，需要先判断当前用户query是否满足以下条件：
    - 用户是否提及会议主题，若无，请提示用户输入会议主题。
    - 用户是否提及开始时间，若无，请提示用户输入开始时间。
    - 用户是否提及结束时间，若无，默认设置一个小时，提示用户当前设置情况，并支持其进行修改。
    - 用户是否提及周期类型（recurring_type），若无，请提示用户输入周期类型。
    - 用户是否提及重复次数（until_count），若无，默认设置为50次，提示用户当前设置情况，并支持其进行修改。
- 即需确保从用户侧获取到会议主题（subject）、开始时间（start_time）、结束时间（end_time）、周期类型（recurring_type）、重复次数（until_count）这五个参数信息。

**参数**: 
- `subject`(必填) - 会议主题
- `start_time`(必填) - 会议开始时间，秒级时间戳

> 调用示例请参考：`references/api_references.md` - schedule_meeting

#### `update_meeting` — 修改会议

强制：修改前让用户进行二次确认：向用户展示要修改的会议信息，确认后再执行修改。

如果用户输入 meeting_code，先通过 `get_meeting_by_code` 查询 meeting_id 及相关 meeting_info。

**参数**: `meeting_id`(必填), `subject`, `start_time`, `end_time`, `password`, `time_zone`, `meeting_type`, `only_user_join_type`, `auto_in_waiting_room`, `recurring_rule`

> 调用示例请参考：`references/api_references.md` - update_meeting

#### `cancel_meeting` — 取消会议

强制：取消前让用户进行二次确认：向用户展示要取消的会议信息，用户确认后再执行取消。

如果用户输入 meeting_code，先通过 `get_meeting_by_code` 查询 meeting_id 及相关 meeting_info。

**参数**: 
- `meeting_id`(必填) - 会议ID
- `sub_meeting_id`(可选) - 周期性会议子会议ID，取消周期性会议的某个子会议时传入
- `meeting_type`(可选) - 如果需要取消整场周期性会议，传1，其他情况下不传

> 调用示例请参考：`references/api_references.md` - cancel_meeting

#### `get_meeting` — 查询会议详情
- 返回主持人和参会者时，如果没有特殊要求，用户id和用户昵称中只返回用户昵称

**参数**: `meeting_id`(必填)

> 调用示例请参考：`references/api_references.md` - get_meeting

#### `get_meeting_by_code` — 通过会议Code查询

**参数**: `meeting_code`(必填)

> 调用示例请参考：`references/api_references.md` - get_meeting_by_code

---

### 2. 成员管理

#### `get_meeting_participants` — 获取参会成员明细

**参数**: `meeting_id`(必填), `sub_meeting_id`(可选, 周期性会议子会议ID), `pos`(可选, 分页起始位置, 默认0), `size`(可选, 每页条数, 最大100), `start_time`(可选, 秒级时间戳), `end_time`(可选, 秒级时间戳)

> 调用示例请参考：`references/api_references.md` - get_meeting_participants

#### `get_meeting_invitees` — 获取受邀成员列表
- 返回邀请人时，如果没有特殊要求，用户id和用户昵称中只返回用户昵称

**参数**: `meeting_id`(必填), `page_size`(默认20), `page_number`(默认1)

> 调用示例请参考：`references/api_references.md` - get_meeting_invitees

#### `get_waiting_room` — 查询等候室成员记录

**参数**: `meeting_id`(必填), `page_size`(默认20), `page`(默认1)

> 调用示例请参考：`references/api_references.md` - get_waiting_room

#### `get_user_meetings` — 查询用户会议列表

只能查询即将开始、正在进行中的会议

如果用户需要查询今天的会议, 需要组合 `get_user_meetings` 和 `get_user_ended_meetings`(查询今天已结束的会议) 2 个 接口的返回结果

根据返回值中的remaining、next_pos、next_cursory进行翻页查询。

**参数**: 
- `pos`(可选) - 查询起始位置，默认为0。分页获取用户会议列表的查询起始时间值，unix 秒级时间戳
- `cursory`(可选) - 分页游标，默认为100
- `is_show_all_sub_meetings`(可选) - 是否展示全部子会议，0-不展示，1-展示，默认为0

> 调用示例请参考：`references/api_references.md` - get_user_meetings

#### `get_user_ended_meetings` — 查询用户已结束会议列表

**参数**: `start_time`(必填, 字符串，秒级时间戳), `end_time`(必填, 字符串，秒级时间戳), `page_size`(默认20), `page_number`(默认1)

> 调用示例请参考：`references/api_references.md` - get_user_ended_meetings

---

### 3. 录制与转写

录制相关接口需要的record_file_id可以通过`get_records_list`获取

#### `get_records_list` — 查询录制列表

**参数**: 
- `start_time`(可选) - 查询开始时间，秒级时间戳，会议id/会议code为空时该参数必填
- `end_time`(可选) - 查询结束时间，秒级时间戳，会议id/会议code为空时该参数必填
- `page_number`(可选) - 页码，从1开始
- `meeting_id`(可选) - 会议ID，不为空时优先根据会议ID查询(若用户提供的是 9 位**会议号（meeting_code）而非 meeting_id，先通过 get_meeting_by_code 查询 meeting_id)
- `meeting_code`(可选) - 会议code，当meeting_id为空且meeting_code不为空时根据会议code查询

> 调用示例请参考：`references/api_references.md` - get_records_list

#### `get_record_addresses` — 获取录制下载地址

**参数**: `meeting_record_id`(必填), `page_number`(默认1)

> 调用示例请参考：`references/api_references.md` - get_record_addresses

#### `get_transcripts_details` — 查询转写详情

**参数**: `record_file_id`(必填), `meeting_id`(可选, 若用户提供的是 9 位**会议号（meeting_code）而非 meeting_id，先通过 get_meeting_by_code 查询 meeting_id)

> 调用示例请参考：`references/api_references.md` - get_transcripts_details

#### `get_transcripts_paragraphs` — 查询转写段落

**参数**: 
- `record_file_id`(必填) - 录制文件ID
- `meeting_id`(可选) - 会议ID

> 调用示例请参考：`references/api_references.md` - get_transcripts_paragraphs

#### `search_transcripts` — 搜索转写内容

**参数**: 
- `record_file_id`(必填) - 录制文件ID
- `text`(必填) - 搜索的文本，如果是中文需要 urlencode
- `meeting_id`(可选) - 会议ID

> 调用示例请参考：`references/api_references.md` - search_transcripts

#### `get_smart_minutes` — 获取智能纪要

**参数**: 
- `record_file_id`(必填) - 录制文件ID
- `lang`(可选) - 翻译语言选择: default(原文，不翻译) / zh(简体中文) / en(英文) / ja(日语)，默认 default
- `pwd`(可选) - 录制文件访问密码

> 调用示例请参考：`references/api_references.md` - get_smart_minutes

---

## 版本更新

### `check_skill_version` — 检查版本更新

**重要**：版本更新完成后，请**重新开始一个新的对话/会话**，以确保新版本的规则完全生效。在同一会话中继续使用可能仍受旧规则影响。

**触发场景**：
- 用户询问是否有新版本时
- 用户遇到已知问题可能在新版本中修复时

**特点**：
- 只提示更新，不强制更新
- 自动检测用户当前版本与最新版本
- 用户可选择更新或跳过，不会强制更新

**参数**: 无

> 调用示例：
```bash
mcporter call tencent-meeting-mcp check_skill_version --args '{}'
```

---

## 调用方式

```bash
# 示例：获取基准时间（用户使用相对时间描述时必须先调用，不传参数）
mcporter call tencent-meeting-mcp convert_timestamp --args '{}'

# 示例：将时间字符串转化为时间戳（调用会议工具前先转化时间）
mcporter call tencent-meeting-mcp convert_timestamp --args '{"parse_time_str": "2026-03-25 14:00:00"}'

# 示例：将秒级时间戳转化为可读时间（展示结果时先转化时间戳）
mcporter call tencent-meeting-mcp convert_timestamp --args '{"parse_time_unix": 1774594800}'

# 示例：将毫秒级时间戳转化为可读时间（自动识别13位毫秒级，保留毫秒精度）
mcporter call tencent-meeting-mcp convert_timestamp --args '{"parse_time_unix": 1774594800123}'

# 示例：指定时区转换（如纽约时间）
mcporter call tencent-meeting-mcp convert_timestamp --args '{"parse_time_unix": 1774594800, "timezone": "America/New_York"}'

# 示例：创建会议
mcporter call tencent-meeting-mcp schedule_meeting --args '{"subject": "周会", "start_time": "1773280800", "end_time": "1773284400"}'

# 示例：查询会议详情
mcporter call tencent-meeting-mcp get_meeting --args '{"meeting_id": "xxx"}'

# 示例：获取智能纪要
mcporter call tencent-meeting-mcp get_smart_minutes --args '{"record_file_id": "xxx"}'
```