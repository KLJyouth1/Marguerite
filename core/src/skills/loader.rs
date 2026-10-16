//! 技能加载器
//!
//! 从本地文件系统或远程 URL 加载技能定义。

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use super::registry::{Skill, SkillSource};
use anyhow::{bail, Context, Result};
use tracing::info;

use crate::security::skill_scan::SkillInstallScanner;

/// 技能扫描器进程级单例：构造会编译 ~20 条正则，成本较高，故 `LazyLock` 复用，
/// 避免 `load_all` 逐个技能加载时重复编译（与 `preflight::Preflight` 的 `LazyLock` 同思路）。
static SKILL_SCANNER: LazyLock<SkillInstallScanner> = LazyLock::new(SkillInstallScanner::new);

/// 技能加载器
///
/// 负责从 YAML 文件加载技能定义，支持本地文件和远程 URL。
pub struct SkillLoader {
    /// 技能文件目录
    skills_dir: PathBuf,
}

/// 用于反序列化 YAML 的中间结构（不含 source 字段）
#[derive(serde::Deserialize)]
struct SkillData {
    name: String,
    description: String,
    version: String,
    author: Option<String>,
    #[serde(default)]
    intent: Vec<String>,
    #[serde(default)]
    tools: Vec<String>,
    prompt_template: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    rating: f64,
    #[serde(default)]
    uses: u32,
    #[serde(default)]
    rank: Option<u32>,
}

impl SkillLoader {
    /// 创建新的技能加载器
    pub fn new(skills_dir: &Path) -> Self {
        Self {
            skills_dir: skills_dir.to_path_buf(),
        }
    }

    /// 加载所有技能
    ///
    /// 扫描技能目录下所有 `.yaml` 文件并加载。
    pub fn load_all(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();

        if !self.skills_dir.exists() {
            info!("技能目录不存在: {}", self.skills_dir.display());
            return Ok(skills);
        }

        let read_dir = std::fs::read_dir(&self.skills_dir)
            .with_context(|| format!("读取技能目录失败: {}", self.skills_dir.display()))?;

        for entry in read_dir {
            let entry = entry?;
            let path = entry.path();

            if path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                match self.load_from_path(&path) {
                    Ok(skill) => {
                        info!("加载技能: {} ({})", skill.name, path.display());
                        skills.push(skill);
                    }
                    Err(e) => {
                        info!("跳过技能文件 {}: {}", path.display(), e);
                    }
                }
            }
        }

        Ok(skills)
    }

    /// 加载单个技能
    ///
    /// 根据技能名称查找并加载对应的 YAML 文件。
    pub fn load(&self, name: &str) -> Result<Skill> {
        let path = self.get_skill_path(name);
        if !path.exists() {
            bail!("技能文件不存在: {}", path.display());
        }
        self.load_from_path(&path)
    }

    /// 从绝对/相对路径加载单个技能文件（供 register-batch 等 CLI 场景做单文件解析）。
    ///
    /// 与 `load(name)` 等价：会基于文件位置打上 Builtin / Local 标记，并自动推断 rank
    /// （未显式指定 rank 时按 source 落入 SYSTEM/USER/COMMUNITY 等级）。
    pub fn load_single(&self, path: &Path) -> Result<Skill> {
        if !path.exists() {
            bail!("技能文件不存在: {}", path.display());
        }
        self.load_from_path(path)
    }

    /// 从 URL 加载技能（远程安装）
    ///
    /// 注意：当前实现仅设置 Remote 标记，不执行实际 HTTP 请求。
    pub fn load_from_url(&self, url: &str) -> Result<Skill> {
        // 从 URL 中推断技能名称
        let name = url
            .rsplit('/')
            .next()
            .and_then(|s| s.strip_suffix(".yaml").or_else(|| s.strip_suffix(".yml")))
            .unwrap_or("unknown");

        let skill = Skill {
            name: name.to_string(),
            description: format!("远程技能: {}", url),
            version: "0.0.0".to_string(),
            author: None,
            intent: vec![],
            tools: vec![],
            prompt_template: String::new(),
            dependencies: vec![],
            source: SkillSource::Remote {
                url: url.to_string(),
            },
            rating: 0.0,
            uses: 0,
            rank: super::registry::ranks::COMMUNITY,
        };

        Ok(skill)
    }

    /// 从远程 URL 下载技能 YAML 并注册到本地技能目录
    ///
    /// 执行真实 HTTP 请求获取 YAML 内容，先经 L2 恶意扫描（见 `SkillInstallScanner`），
    /// 命中即拒绝落盘，否则解析并保存到技能目录。
    pub async fn download_and_register(&self, url: &str) -> Result<Skill> {
        let client = reqwest::Client::new();
        let resp = client
            .get(url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .with_context(|| format!("下载技能失败: {url}"))?;
        if !resp.status().is_success() {
            bail!("下载技能失败: HTTP {}", resp.status());
        }
        let content = resp.text().await?;

        // 解析一次以取技能名称（URL 无可靠 name 字段时兜底）
        let data: SkillData =
            serde_yaml::from_str(&content).with_context(|| format!("解析技能 YAML 失败: {url}"))?;
        let name = if data.name.is_empty() {
            url.rsplit('/')
                .next()
                .and_then(|s| s.strip_suffix(".yaml").or_else(|| s.strip_suffix(".yml")))
                .unwrap_or("skill")
                .to_string()
        } else {
            data.name
        };

        self.install_yaml(&name, &content)
            .with_context(|| format!("技能安装被安全层拦截: {url}"))
    }

    /// 安装本地 YAML 内容到技能目录：先经 L2 恶意扫描，命中即拒绝，否则落盘。
    ///
    /// 这是"技能安装前恶意扫描"的统一入口——所有进入工作区的技能文件（无论来自
    /// 远程下载还是本地导入）在调用 `std::fs::write` 落盘前都必须经过此函数。命中
    /// 危险模式（任意 `Critical` 或累计 risk_score ≥ 120）时拒绝安装，绝不写盘。
    pub fn install_yaml(&self, name: &str, content: &str) -> Result<Skill> {
        // —— L2：落盘前扫描技能内容（含 manifest / prompt_template / dependencies）——
        let verdict = SKILL_SCANNER.scan(content);
        if verdict.blocked {
            crate::security::skill_scan::log_skill_block("install", name, None, &verdict);
            bail!(
                "技能安装被安全层拒绝（{}）：{}",
                verdict.code(),
                verdict.reason
            );
        }

        let data: SkillData =
            serde_yaml::from_str(content).with_context(|| "解析技能 YAML 失败".to_string())?;

        // 扫描通过后才落盘
        std::fs::create_dir_all(&self.skills_dir)?;
        let dest = self.skills_dir.join(format!("{name}.yaml"));
        std::fs::write(&dest, content)?;

        let skill = Skill {
            name: name.to_string(),
            description: data.description,
            version: data.version,
            author: data.author,
            intent: data.intent,
            tools: data.tools,
            prompt_template: data.prompt_template,
            dependencies: data.dependencies,
            source: SkillSource::Local { path: dest },
            rating: data.rating,
            uses: data.uses,
            rank: data.rank.unwrap_or(super::registry::ranks::USER),
        };
        info!("技能已安装并通过安全扫描: {name}");
        Ok(skill)
    }

    /// 验证技能签名（基本版）
    ///
    /// 检查 source 字段的合法性。
    pub fn verify_signature(&self, skill: &Skill) -> bool {
        match &skill.source {
            SkillSource::Builtin => true,
            SkillSource::Local { path } => path.exists(),
            SkillSource::Remote { url } => {
                url.starts_with("http://") || url.starts_with("https://")
            }
        }
    }

    /// 获取技能文件路径
    ///
    /// 根据技能名称拼接对应的 YAML 文件路径。
    pub fn get_skill_path(&self, name: &str) -> PathBuf {
        self.skills_dir.join(format!("{}.yaml", name))
    }

    /// 从指定路径加载技能
    fn load_from_path(&self, path: &Path) -> Result<Skill> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("读取技能文件失败: {}", path.display()))?;

        // 运行时防篡改：已安装 / 本地导入的技能在加载执行前再扫一遍 L2。
        // 覆盖两类场景：① 绕过 `install_yaml` 直接丢入 skills_dir 的未审查 YAML；
        // ② 安装后被篡改（落盘内容被改写）的技能文件。命中即拒绝加载，绝不执行。
        let verdict = SKILL_SCANNER.scan(&content);
        if verdict.blocked {
            let audit_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            crate::security::skill_scan::log_skill_block("load", audit_name, Some(path), &verdict);
            bail!(
                "技能加载被安全层拒绝（{}）：{}",
                verdict.code(),
                verdict.reason
            );
        }

        let data: SkillData = serde_yaml::from_str(&content)
            .with_context(|| format!("解析技能文件失败: {}", path.display()))?;

        let source = if path.starts_with(&self.skills_dir) {
            // 相对于技能目录的文件标记为 Builtin（如果来自内置目录）
            SkillSource::Builtin
        } else {
            SkillSource::Local {
                path: path.to_path_buf(),
            }
        };

        let rank = data.rank.unwrap_or(match &source {
            SkillSource::Builtin => super::registry::ranks::SYSTEM,
            SkillSource::Local { .. } => super::registry::ranks::USER,
            SkillSource::Remote { .. } => super::registry::ranks::COMMUNITY,
        });

        let skill = Skill {
            name: data.name,
            description: data.description,
            version: data.version,
            author: data.author,
            intent: data.intent,
            tools: data.tools,
            prompt_template: data.prompt_template,
            dependencies: data.dependencies,
            source,
            rating: data.rating,
            uses: data.uses,
            rank,
        };

        Ok(skill)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_skill_yaml(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{}.yaml", name));
        let content = format!(
            r#"name: {}
description: "测试技能: {}"
version: "1.0.0"
intent:
  - test
  - 测试
tools: []
prompt_template: "模板: {{text}}"
"#,
            name, name
        );
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_load_single_skill() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_single");
        fs::create_dir_all(&dir).unwrap();
        create_test_skill_yaml(&dir, "test_skill");

        let loader = SkillLoader::new(&dir);
        let skill = loader.load("test_skill").unwrap();
        assert_eq!(skill.name, "test_skill");
        assert_eq!(skill.version, "1.0.0");
        assert!(skill.intent.contains(&"test".to_string()));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_nonexistent");
        let loader = SkillLoader::new(&dir);
        assert!(loader.load("nonexistent").is_err());
    }

    #[test]
    fn test_load_all() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_all");
        fs::create_dir_all(&dir).unwrap();
        create_test_skill_yaml(&dir, "skill_a");
        create_test_skill_yaml(&dir, "skill_b");

        let loader = SkillLoader::new(&dir);
        let skills = loader.load_all().unwrap();
        assert_eq!(skills.len(), 2);

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"skill_a"));
        assert!(names.contains(&"skill_b"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_all_empty_dir() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_empty");
        fs::create_dir_all(&dir).unwrap();

        let loader = SkillLoader::new(&dir);
        let skills = loader.load_all().unwrap();
        assert!(skills.is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_all_nonexistent_dir() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_nodir");
        let loader = SkillLoader::new(&dir);
        let skills = loader.load_all().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_get_skill_path() {
        let dir = Path::new("/tmp/skills");
        let loader = SkillLoader::new(dir);
        let path = loader.get_skill_path("my_skill");
        assert_eq!(path, Path::new("/tmp/skills/my_skill.yaml"));
    }

    #[test]
    fn test_verify_signature_builtin() {
        let dir = std::env::temp_dir().join("marguerite_test_verify");
        let loader = SkillLoader::new(&dir);

        let skill = Skill {
            name: "test".to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec![],
            tools: vec![],
            prompt_template: "".to_string(),
            dependencies: vec![],
            source: SkillSource::Builtin,
            rating: 0.0,
            uses: 0,
            rank: 300,
        };
        assert!(loader.verify_signature(&skill));
    }

    #[test]
    fn test_verify_signature_local() {
        let dir = std::env::temp_dir().join("marguerite_test_verify_local");
        fs::create_dir_all(&dir).unwrap();
        let yaml_path = create_test_skill_yaml(&dir, "local_skill");

        let loader = SkillLoader::new(&dir);

        let skill = Skill {
            name: "local_skill".to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec![],
            tools: vec![],
            prompt_template: "".to_string(),
            dependencies: vec![],
            source: SkillSource::Local {
                path: yaml_path.clone(),
            },
            rating: 0.0,
            uses: 0,
            rank: 300,
        };
        assert!(loader.verify_signature(&skill));

        // 不存在的路径应验证失败
        let skill_bad = Skill {
            source: SkillSource::Local {
                path: Path::new("/nonexistent/skill.yaml").to_path_buf(),
            },
            ..skill
        };
        assert!(!loader.verify_signature(&skill_bad));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_verify_signature_remote() {
        let dir = std::env::temp_dir().join("marguerite_test_verify_remote");
        let loader = SkillLoader::new(&dir);

        let skill = Skill {
            name: "test".to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec![],
            tools: vec![],
            prompt_template: "".to_string(),
            dependencies: vec![],
            source: SkillSource::Remote {
                url: "https://example.com/skill.yaml".to_string(),
            },
            rating: 0.0,
            uses: 0,
            rank: 300,
        };
        assert!(loader.verify_signature(&skill));

        // 非 http(s) URL 应验证失败
        let skill_bad = Skill {
            source: SkillSource::Remote {
                url: "ftp://example.com/skill.yaml".to_string(),
            },
            ..skill
        };
        assert!(!loader.verify_signature(&skill_bad));
    }

    #[test]
    fn test_load_from_url() {
        let dir = std::env::temp_dir().join("marguerite_test_url");
        let loader = SkillLoader::new(&dir);

        let skill = loader
            .load_from_url("https://example.com/skills/my_skill.yaml")
            .unwrap();
        assert_eq!(skill.name, "my_skill");
        assert!(matches!(skill.source, SkillSource::Remote { .. }));
    }

    /// 简易 mock HTTP 服务器：单次响应
    async fn mock_http_server(status: u16, body: &str) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    let reason = if status >= 500 { "Server Error" } else { "OK" };
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/x-yaml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn test_download_and_register_success() {
        let dir = std::env::temp_dir().join("marguerite_test_download");
        let _ = std::fs::remove_dir_all(&dir);
        let yaml = r#"name: remote_skill
description: "远程技能测试"
version: "0.1.0"
prompt_template: "模板: {{text}}"
"#;
        let url = mock_http_server(200, yaml).await;
        let loader = SkillLoader::new(&dir);

        let skill = loader.download_and_register(&url).await.unwrap();
        assert_eq!(skill.name, "remote_skill");
        assert_eq!(skill.version, "0.1.0");
        assert!(matches!(skill.source, SkillSource::Local { .. }));
        // 已保存到技能目录
        let saved = loader.load("remote_skill").unwrap();
        assert_eq!(saved.name, "remote_skill");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_download_and_register_http_error() {
        let dir = std::env::temp_dir().join("marguerite_test_download_err");
        let url = mock_http_server(404, "not found").await;
        let loader = SkillLoader::new(&dir);
        assert!(loader.download_and_register(&url).await.is_err());
    }

    #[tokio::test]
    async fn test_download_and_register_invalid_yaml() {
        let dir = std::env::temp_dir().join("marguerite_test_download_bad");
        let url = mock_http_server(200, "not: [valid: yaml").await;
        let loader = SkillLoader::new(&dir);
        assert!(loader.download_and_register(&url).await.is_err());
    }

    /// 技能安装前恶意扫描：内嵌 eval 的 YAML 应被拒绝，且绝不能落盘。
    #[test]
    fn install_yaml_blocks_malicious_skill_and_writes_nothing() {
        let dir = std::env::temp_dir().join("marguerite_test_install_malicious");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let loader = SkillLoader::new(&dir);

        let evil = "name: evil\nversion: \"0.1.0\"\nprompt_template: |\n  result = eval(user_input)\n  return result\n";
        let res = loader.install_yaml("evil", evil);
        assert!(res.is_err(), "含 eval 的技能安装应被拒绝");
        let msg = format!("{res:?}");
        assert!(
            msg.contains(crate::security::skill_scan::SKILL_MALWARE_BLOCKED),
            "拒绝消息应带错误码: {msg}"
        );

        // 关键不变量：拒绝后工作区不得出现该文件（绝不写盘）
        assert!(!dir.join("evil.yaml").exists(), "被拦截的技能绝不应落盘");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 良性技能 YAML 应正常安装并落盘。
    #[test]
    fn install_yaml_allows_benign_skill() {
        let dir = std::env::temp_dir().join("marguerite_test_install_benign");
        let _ = std::fs::remove_dir_all(&dir);
        let loader = SkillLoader::new(&dir);

        let good = "name: adder\ndescription: 求和技能\nversion: \"1.0.0\"\nprompt_template: |\n  请对给定数字求和并返回结果。\n";
        let skill = loader.install_yaml("adder", good).unwrap();
        assert_eq!(skill.name, "adder");
        assert!(dir.join("adder.yaml").exists(), "良性技能应已落盘");

        // 已安装的技能可被 load 读出
        let reloaded = loader.load("adder").unwrap();
        assert_eq!(reloaded.name, "adder");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 本地导入的恶意技能在加载执行前被 L2 扫描拒绝（运行时防篡改）。
    ///
    /// 模拟"绕过 install_yaml、直接把恶意 YAML 丢进 skills_dir"的场景，
    /// `load()` → `load_from_path` 应在解析前扫描并拒绝。
    #[test]
    fn load_rejects_malicious_skill_on_path() {
        let dir = std::env::temp_dir().join("marguerite_test_load_malicious");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 直接写盘，绕过 install_yaml 的落盘前扫描（模拟本地导入 / 落盘后被篡改）
        let evil = "name: evil\nversion: \"0.1.0\"\nprompt_template: |\n  result = eval(user_input)\n  return result\n";
        std::fs::write(dir.join("evil.yaml"), evil).unwrap();

        let loader = SkillLoader::new(&dir);
        assert!(
            loader.load("evil").is_err(),
            "本地导入的恶意技能加载应被安全层拒绝"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
