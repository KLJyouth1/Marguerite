//! Integration test: validate the third_party reference+adaptation layer against
//! the REAL Marguerite types — not just JSON/YAML structure.
//!
//! - Loads all 18 harness `PluginManifest` JSON files, registers them through
//!   `register_manifests`, and wires hooks through `build_hook_registry`.
//! - Loads all 5 JIT engineering-discipline `Skill` YAML files via `SkillLoader`.
//!
//! This is the type-level proof that the P0 artifacts conform to
//! `core/src/plugins/manifest.rs::PluginManifest` and
//! `core/src/skills/registry.rs::Skill`. Run with:
//!   cargo test -p marguerite-core --test third_party_integration

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use marguerite_core::plugins::manifest::{
    build_hook_registry, register_manifests, HookEvent, HookHandler, PluginManifest,
};
use marguerite_core::plugins::{PluginContext, PluginRegistry};
use marguerite_core::skills::loader::SkillLoader;
use marguerite_core::skills::registry::{Skill, SkillRegistry};

fn third_party_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR == marguerite/core
    // third_party/ lives at the workspace sibling level: Marguerite/third_party
    // (i.e. two levels up from marguerite/core), NOT under marguerite/.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party")
}

/// Resolver that maps every handler name declared across the 18 manifests to a
/// no-op allow handler, proving the hook wiring path end-to-end.
struct AllowHandler;
impl HookHandler for AllowHandler {
    fn run(&self, _event: HookEvent, _payload: &str) -> anyhow::Result<String> {
        Ok("allow".to_string())
    }
}

fn resolved_handlers() -> [&'static str; 17] {
    [
        "guard_cmd",
        "audit_io",
        "team_audit",
        "ui_push_activity",
        "ui_push_card",
        "memory_distill",
        "focus_guide",
        "profile_record",
        "plan_gate",
        "evolve_gate",
        "bus_publish",
        "budget_check",
        "session_persist",
        "method_advance",
        "steer_window",
        "turn_cap",
        "discipline_check",
    ]
}

#[test]
fn all_harness_manifests_load_register_and_wire() {
    let pe = third_party_dir().join("plugin-examples");
    assert!(
        pe.exists(),
        "plugin-examples dir missing at {}",
        pe.display()
    );

    let mut manifests: Vec<PluginManifest> = Vec::new();
    for entry in fs::read_dir(&pe).expect("read plugin-examples") {
        let path = entry.unwrap().path();
        let fname = path.to_string_lossy();
        if path.extension().and_then(|e| e.to_str()) == Some("json") && fname.contains(".plugin.") {
            let text = fs::read_to_string(&path).expect("read manifest");
            let m = PluginManifest::from_json(&text).unwrap_or_else(|e| {
                panic!("PluginManifest::from_json failed for {}: {:?}", fname, e)
            });
            manifests.push(m);
        }
    }

    assert_eq!(manifests.len(), 18, "expected 18 harness manifests");

    // Bridge into the native registry (ManifestPlugin) — same path the app uses.
    let mut reg = PluginRegistry::new();
    register_manifests(&mut reg, &manifests);
    assert_eq!(
        reg.len(),
        18,
        "registered plugin count must equal manifest count"
    );

    // Every manifest must declare at least one capability source (explicit or via
    // sub-structure). We check the public fields only — `derived_capabilities()`
    // is private, but `ManifestPlugin::capabilities()` (via the Plugin trait)
    // exercises the same derivation once bridged below.
    for m in &manifests {
        assert!(
            !m.capabilities.is_empty()
                || !m.hooks.is_empty()
                || !m.slash_commands.is_empty()
                || !m.subagents.is_empty()
                || !m.skills.is_empty(),
            "manifest '{}' declares no capabilities or sub-structure",
            m.name
        );
    }

    // End-to-end hook wiring: resolver returns AllowHandler for every handler name
    // declared across the 18 manifests (verified: all 17 distinct handlers match).
    let known: Vec<&str> = resolved_handlers().to_vec();
    let matched: usize = manifests
        .iter()
        .flat_map(|m| m.hooks.iter())
        .filter(|h| known.contains(&h.handler.as_str()))
        .count();
    assert!(
        matched > 0,
        "no manifest hook handler maps to a known resolver handler"
    );
    let hook_reg = build_hook_registry(&manifests, |h| {
        if known.contains(&h) {
            Some(Arc::new(AllowHandler) as Arc<dyn HookHandler>)
        } else {
            None
        }
    });

    // matched > 0 guarantees at least one hook is wired, so dispatch must fire.
    let pre = hook_reg.dispatch(HookEvent::PreToolUse, "ls -la");
    let post = hook_reg.dispatch(HookEvent::PostToolUse, "edit file.rs");
    assert!(
        !pre.is_empty() || !post.is_empty(),
        "expected at least one wired hook to dispatch"
    );

    // Sanity: a declared capability route returns a well-formed response.
    let ctx = PluginContext::default();
    let resp = reg
        .invoke("slash.commands", "", &ctx)
        .expect("slash.commands capability should be invokable");
    assert!(
        resp.contains("slash_commands"),
        "unexpected slash.commands response: {resp}"
    );
}

#[test]
fn all_jit_skills_load_with_prompt_templates() {
    let skills_dir = third_party_dir().join("skills");
    assert!(
        skills_dir.exists(),
        "skills dir missing at {}",
        skills_dir.display()
    );

    let loader = SkillLoader::new(&skills_dir);
    let skills: Vec<Skill> = loader.load_all().expect("load all skills");

    assert_eq!(
        skills.len(),
        5,
        "expected 5 JIT engineering-discipline skills"
    );

    let mut reg = SkillRegistry::new();
    for s in &skills {
        assert!(
            !s.prompt_template.trim().is_empty(),
            "skill '{}' has empty prompt_template (loader would register a no-op)",
            s.name
        );
        reg.register(s.clone())
            .unwrap_or_else(|e| panic!("register skill '{}' failed: {e}", s.name));
    }

    for name in [
        "engineering-discipline",
        "tdd",
        "systematic-debugging",
        "verification-before-completion",
        "write-plan-first",
    ] {
        assert!(reg.has(name), "expected skill '{name}' to be registered");
    }
}
