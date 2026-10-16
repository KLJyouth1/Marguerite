# GitHub 全盘代码上传与本地清理 · 任务交接文档

> 本文件是跨会话交接文档。因为 GitHub 连接器要求**新开会话**才能使用，请把本文件内容完整复制/告知到**新会话**，由新会话执行上传与删除。

## 一、任务目标

1. 把 **C盘、D盘、E盘** 所有**含代码及代码文件的文件夹**，各自作为一个独立项目，上传到 GitHub。
2. 每个项目创建**私有（private）非公开**仓库；zip 压缩包内含项目代码的，按 zip 仓库或按压缩包实际内容命名仓库。
3. 上传并**确认成功**后，删除对应本地文件夹。
4. **绝对保留不动**：`D:\Downloads\Marguerite` 及其所有相关文件（本项目、工作区、文档、配置等）。

## 二、关键前置（新会话必须确认）

- GitHub 连接器已授权，但需**新开会话**后才能调用（授权提示：Start a new conversation before retrying this service）。
- 机器上安装了 `git`（`C:\Program Files\Git\cmd\git.exe`），**未安装 `gh` CLI**。
- 新会话中若连接器可用，优先用连接器创建私有仓库 + `git push`；
  若连接器仍不可用，则需用户提供 Personal Access Token（需 repo 权限），用 `git` + `https://<token>@github.com/...` 推送。

## 三、仓库命名规则

- 普通项目：用文件夹名（尽量转成英文/拼音，避免中文与空格，GitHub 仓库名只允许 `[a-zA-Z0-9._-]`）。
- zip 压缩包内含代码：若压缩包名显然是对应项目，用项目名；否则先解压查看实际内容再按内容命名。
- 所有仓库统一为 `--private`。

## 四、已确认要上传的项目（人工确认过）

| 本地路径 | 类型 | 建议仓库名 |
|---|---|---|
| `D:\桌面\2025-7-9\C` | C 语言项目（main.c、test.c、Makefile） | c_project_2025-07-09 |
| `D:\Pad_shop` | PHP + Maven 项目 | pad_shop |
| `D:\Downloads\html` | web 项目（server.js、高中全科教学.html 等） | html_web |
| `D:\桌面\2025-6-24\众乐邦` | web 项目（index.html、css、js） | zhongle_bang |
| `D:\桌面\2025-6-24` 下各级 zip（含代码的） | 压缩包 | 按实际内容命名 |
| `D:\桌面\2025-7-9` 下各 zip（含代码的） | 压缩包 | 按实际内容命名 |

## 五、全盘扫描：C/D/E 盘含代码的目录（需新会话继续盘点并执行）

以下是根据顶层目录初步观察，**可能含代码**的候选，需新会话逐个确认是否含代码文件，再决定是否上传：

- C 盘：`C:\c`、`C:\marbu`、`C:\tmp`、`C:\tools`、`C:\tp_marg`、`C:\soft`、`C:\soft2`、`C:\uploads`、`C:\temp`
- D 盘：`D:\c`、`D:\d`、`D:\workspace`、`D:\wwwroot`、`D:\UnivAgent`、`D:\UnivAgent_Backup`、`D:\tmp`、`D:\tp_build3`、`D:\mgt_build5`、`D:\mgt_tmp5`、`D:\empty_tmp`、`D:\Ksoftware`、`D:\backup`
- E 盘：`E:\Claude_code`、`E:\CodeBuddy`、`E:\github-profile`、`E:\hsword`、`E:\mgt-target-pr`、`E:\qoder`、`E:\qoderwork`、`E:\Study`、`E:\temp`、`E:\tmp`、`E:\TryFixTools`、`E:\VSCode`、`E:\WechatBot`、`E:\wk`、`E:\ZMJ`、`E:\下载`、`E:\个人文件`、`E:\教学资料`、`E:\文档表单`、`E:\新建文件夹*`、`E:\校外工作`、`E:\毕业设计相关材料`、`E:\DBC`、`E:\DDOS`、`E:\SanGame`、`E:\UserTabul`、`E:\back_be20251119_1130`、`E:\3DGameF`、`E:\DataRecoveryPro`、`E:\DataRecoveryPro_oss`、`E:\FlashFXP`、`E:\GT_New_Horizons_2.8.4_Java_8`

> 注意：很多目录是**软件/游戏/系统安装目录**（如 `C:\Windows`、`C:\Program Files`、`D:\SteamLibrary`、`E:\Steam`、`E:\Epic`、`E:\Unreal`、杀毒软件目录等），**不属于代码项目，不要上传、不要删**。新会话需过滤：仅把含源码特征（`.py/.js/.ts/.java/.c/.cpp/.html/css/pom.xml/package.json/Cargo.toml` 等）且为用户项目文件的目录纳入。

## 六、上传与删除的安全流程（务必遵守）

1. **逐个处理**：每处理一个项目，先 `git init` → `git add` → `git commit` → 创建 GitHub 私有仓库 → `git push`。
2. **上传后校验**：`git ls-remote` 或 `git status` 确认推送成功、远端文件齐全。
3. **确认后再删**：只有该仓库在 GitHub 上确认存在且代码完整后，才删除对应本地目录。
4. 删除使用安全方式，先移到临时回收区或直接删除，但**前提是远端口径一致**。
5. 全程**绝不触碰** `D:\Downloads\Marguerite`。

## 七、执行建议（新会话）

这是一个超大任务（全盘海量目录）。建议新会话：
1. 先确认 GitHub 连接器可用（或要 Token）。
2. 按 D 盘 → E 盘 → C 盘顺序，用脚本批量扫描「含源码特征文件」的目录。
3. 产出完整清单后，再用脚本化方式批量创建私有仓库并推送。
4. 每批上传后校验，再删除本地。

## 八、当前会话已完成的确认

- GitHub 连接器授权成功（需新会话使用）。
- 已确认 5 个具体项目 + 压缩包需上传。
- 用户已选择「新开会话继续」+「确认上传后再删」。
- 用户在问题中补充说明："C盘、D盘、E盘的所有含有代码及代码文件的文件夹都为一个项目"。