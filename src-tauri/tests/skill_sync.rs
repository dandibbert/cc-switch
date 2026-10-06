use std::fs;

use cc_switch_lib::{
    migrate_skills_to_ssot, AppType, ImportSkillSelection, InstalledSkill, SkillApps, SkillService,
};

#[path = "support.rs"]
mod support;
use support::{create_test_state, ensure_test_home, reset_test_fs, test_mutex};

fn write_skill(dir: &std::path::Path, name: &str) {
    fs::create_dir_all(dir).expect("create skill dir");
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Test skill\n---\n"),
    )
    .expect("write SKILL.md");
}

#[cfg(unix)]
fn symlink_dir(src: &std::path::Path, dest: &std::path::Path) {
    std::os::unix::fs::symlink(src, dest).expect("create symlink");
}

#[cfg(windows)]
fn symlink_dir(src: &std::path::Path, dest: &std::path::Path) {
    std::os::windows::fs::symlink_dir(src, dest).expect("create symlink");
}

#[test]
fn import_from_apps_respects_explicit_app_selection() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();

    write_skill(
        &home.join(".claude").join("skills").join("shared-skill"),
        "Shared",
    );
    write_skill(
        &home
            .join(".config")
            .join("opencode")
            .join("skills")
            .join("shared-skill"),
        "Shared",
    );

    let state = create_test_state().expect("create test state");

    let imported = SkillService::import_from_apps(
        &state.db,
        vec![ImportSkillSelection {
            directory: "shared-skill".to_string(),
            apps: SkillApps {
                opencode: true,
                ..Default::default()
            },
            source_path: None,
        }],
    )
    .expect("import skills");

    assert_eq!(imported.len(), 1, "expected exactly one imported skill");
    let skill = imported.first().expect("imported skill");
    assert!(
        skill.apps.opencode,
        "explicitly selected OpenCode app should remain enabled"
    );
    assert!(
        !skill.apps.claude && !skill.apps.codex && !skill.apps.gemini,
        "import should no longer infer apps from every matching source path"
    );
}

#[test]
fn import_from_apps_does_not_rewrite_selected_app_directory() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();

    let ssot_skill_dir = home.join(".cc-switch").join("skills").join("codex-skill");
    write_skill(&ssot_skill_dir, "Stale SSOT Skill");
    fs::write(ssot_skill_dir.join("prompt.md"), "stale ssot").expect("write stale ssot prompt");

    let codex_skill_dir = home.join(".codex").join("skills").join("codex-skill");
    write_skill(&codex_skill_dir, "Live Codex Skill");
    fs::write(codex_skill_dir.join("prompt.md"), "live codex").expect("write live codex prompt");

    let state = create_test_state().expect("create test state");

    let imported = SkillService::import_from_apps(
        &state.db,
        vec![ImportSkillSelection {
            directory: "codex-skill".to_string(),
            apps: SkillApps {
                codex: true,
                ..Default::default()
            },
            source_path: None,
        }],
    )
    .expect("import skills");

    assert_eq!(imported.len(), 1, "expected exactly one imported skill");
    assert!(
        imported[0].apps.codex,
        "import should preserve the selected Codex app state"
    );
    assert_eq!(
        fs::read_to_string(codex_skill_dir.join("prompt.md")).expect("read live codex prompt"),
        "live codex",
        "import should not replace the app skill directory with SSOT contents"
    );
    assert!(
        !fs::symlink_metadata(&codex_skill_dir)
            .expect("read codex skill metadata")
            .file_type()
            .is_symlink(),
        "import should not replace the app skill directory with a managed symlink"
    );
}

#[test]
fn sync_to_app_removes_disabled_and_orphaned_ssot_symlinks() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();

    let ssot_dir = home.join(".cc-switch").join("skills");
    let disabled_skill = ssot_dir.join("disabled-skill");
    let orphan_skill = ssot_dir.join("orphan-skill");
    write_skill(&disabled_skill, "Disabled");
    write_skill(&orphan_skill, "Orphan");

    let opencode_skills_dir = home.join(".config").join("opencode").join("skills");
    fs::create_dir_all(&opencode_skills_dir).expect("create opencode skills dir");
    symlink_dir(&disabled_skill, &opencode_skills_dir.join("disabled-skill"));
    symlink_dir(&orphan_skill, &opencode_skills_dir.join("orphan-skill"));

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_skill(&InstalledSkill {
            id: "local:disabled-skill".to_string(),
            name: "Disabled".to_string(),
            description: None,
            directory: "disabled-skill".to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps::default(),
            installed_at: 0,
            content_hash: None,
            updated_at: 0,
        })
        .expect("save disabled skill");

    SkillService::sync_to_app(&state.db, &AppType::OpenCode).expect("reconcile skills");

    assert!(
        !opencode_skills_dir.join("disabled-skill").exists(),
        "DB-known disabled skill should be removed from OpenCode live dir"
    );
    assert!(
        !opencode_skills_dir.join("orphan-skill").exists(),
        "orphaned symlink into SSOT should be cleaned up"
    );
}

#[test]
fn uninstall_skill_creates_backup_before_removing_ssot() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();

    let ssot_skill_dir = home.join(".cc-switch").join("skills").join("backup-skill");
    write_skill(&ssot_skill_dir, "Backup Skill");
    fs::write(ssot_skill_dir.join("prompt.md"), "backup me").expect("write prompt.md");

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_skill(&InstalledSkill {
            id: "local:backup-skill".to_string(),
            name: "Backup Skill".to_string(),
            description: Some("Back me up before uninstall".to_string()),
            directory: "backup-skill".to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps {
                claude: true,
                ..Default::default()
            },
            installed_at: 123,
            content_hash: None,
            updated_at: 0,
        })
        .expect("save skill");

    let result = SkillService::uninstall(&state.db, "local:backup-skill").expect("uninstall skill");
    let backup_path = result.backup_path.expect("backup path should be returned");
    let backup_dir = std::path::PathBuf::from(&backup_path);

    assert!(backup_dir.exists(), "backup directory should exist");
    assert!(
        backup_dir.join("skill").join("SKILL.md").exists(),
        "backup should include SKILL.md"
    );
    assert_eq!(
        fs::read_to_string(backup_dir.join("skill").join("prompt.md"))
            .expect("read backed up prompt"),
        "backup me"
    );

    let metadata: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(backup_dir.join("meta.json")).expect("read backup metadata"),
    )
    .expect("parse backup metadata");
    assert_eq!(metadata["skill"]["directory"], "backup-skill");
    assert_eq!(metadata["skill"]["name"], "Backup Skill");

    assert!(
        !ssot_skill_dir.exists(),
        "SSOT skill directory should be removed after uninstall"
    );
    assert!(
        state
            .db
            .get_installed_skill("local:backup-skill")
            .expect("query skill")
            .is_none(),
        "database row should be deleted after uninstall"
    );
}

#[test]
fn restore_skill_backup_restores_files_to_ssot_and_current_app() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();

    let ssot_skill_dir = home.join(".cc-switch").join("skills").join("restore-skill");
    write_skill(&ssot_skill_dir, "Restore Skill");
    fs::write(ssot_skill_dir.join("prompt.md"), "restore me").expect("write prompt.md");

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_skill(&InstalledSkill {
            id: "local:restore-skill".to_string(),
            name: "Restore Skill".to_string(),
            description: Some("Bring the files back".to_string()),
            directory: "restore-skill".to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps {
                claude: true,
                ..Default::default()
            },
            installed_at: 456,
            content_hash: None,
            updated_at: 0,
        })
        .expect("save skill");

    let uninstall =
        SkillService::uninstall(&state.db, "local:restore-skill").expect("uninstall skill");
    let backup_id = std::path::Path::new(
        &uninstall
            .backup_path
            .expect("backup path should be returned on uninstall"),
    )
    .file_name()
    .expect("backup dir name")
    .to_string_lossy()
    .to_string();

    let restored = SkillService::restore_from_backup(&state.db, &backup_id, &AppType::Claude)
        .expect("restore from backup");

    assert_eq!(restored.directory, "restore-skill");
    assert!(restored.apps.claude, "restored skill should enable Claude");
    assert!(
        !restored.apps.codex && !restored.apps.gemini && !restored.apps.opencode,
        "restore should only enable the selected app"
    );
    assert!(
        home.join(".cc-switch")
            .join("skills")
            .join("restore-skill")
            .join("prompt.md")
            .exists(),
        "restored skill should exist in SSOT"
    );
    assert!(
        home.join(".claude")
            .join("skills")
            .join("restore-skill")
            .join("prompt.md")
            .exists(),
        "restored skill should sync to the selected app"
    );
    assert!(
        state
            .db
            .get_installed_skill("local:restore-skill")
            .expect("query restored skill")
            .is_some(),
        "restored skill should be written back to the database"
    );
}

#[test]
fn delete_skill_backup_removes_backup_directory() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();

    let ssot_skill_dir = home
        .join(".cc-switch")
        .join("skills")
        .join("delete-backup-skill");
    write_skill(&ssot_skill_dir, "Delete Backup Skill");

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_skill(&InstalledSkill {
            id: "local:delete-backup-skill".to_string(),
            name: "Delete Backup Skill".to_string(),
            description: Some("Remove my backup".to_string()),
            directory: "delete-backup-skill".to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps {
                claude: true,
                ..Default::default()
            },
            installed_at: 789,
            content_hash: None,
            updated_at: 0,
        })
        .expect("save skill");

    let uninstall =
        SkillService::uninstall(&state.db, "local:delete-backup-skill").expect("uninstall skill");
    let backup_path = uninstall
        .backup_path
        .expect("backup path should be returned on uninstall");
    let backup_id = std::path::Path::new(&backup_path)
        .file_name()
        .expect("backup dir name")
        .to_string_lossy()
        .to_string();

    assert!(
        std::path::Path::new(&backup_path).exists(),
        "backup directory should exist before deletion"
    );

    SkillService::delete_backup(&backup_id).expect("delete backup");

    assert!(
        !std::path::Path::new(&backup_path).exists(),
        "backup directory should be removed"
    );
    assert!(
        SkillService::list_backups()
            .expect("list backups")
            .into_iter()
            .all(|entry| entry.backup_id != backup_id),
        "deleted backup should no longer appear in backup list"
    );
}

#[test]
fn migration_snapshot_overrides_multi_source_directory_inference() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();

    write_skill(
        &home.join(".claude").join("skills").join("demo-skill"),
        "Demo",
    );
    write_skill(
        &home
            .join(".config")
            .join("opencode")
            .join("skills")
            .join("demo-skill"),
        "Demo",
    );

    let state = create_test_state().expect("create test state");
    state
        .db
        .set_setting(
            "skills_ssot_migration_snapshot",
            r#"[{"directory":"demo-skill","app_type":"claude"}]"#,
        )
        .expect("seed migration snapshot");

    let count = migrate_skills_to_ssot(&state.db).expect("migrate skills to ssot");
    assert_eq!(count, 1, "expected one migrated skill");

    let skills = state.db.get_all_installed_skills().expect("get skills");
    let migrated = skills
        .values()
        .find(|skill| skill.directory == "demo-skill")
        .expect("migrated demo-skill");

    assert!(
        migrated.apps.claude,
        "legacy snapshot should preserve Claude enablement"
    );
    assert!(
        !migrated.apps.opencode,
        "migration should no longer infer OpenCode enablement from a duplicate directory alone"
    );
}

fn native_test_skill(id: &str, directory: &str, apps: SkillApps) -> InstalledSkill {
    InstalledSkill {
        id: id.to_string(),
        name: directory.to_string(),
        description: None,
        directory: directory.to_string(),
        repo_owner: None,
        repo_name: None,
        repo_branch: None,
        readme_url: None,
        apps,
        installed_at: 1_000,
        content_hash: None,
        updated_at: 0,
    }
}

fn clean_native_roots(home: &std::path::Path) {
    for sub in [".agents", ".pi"] {
        let _ = fs::remove_dir_all(home.join(sub));
    }
}

fn codex_row(name: &str, model: &str, base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "auth": { "OPENAI_API_KEY": format!("sk-{name}") },
        "config": format!(
            "model_provider = \"{name}\"\nmodel = \"{model}\"\n\n[model_providers.{name}]\nname = \"{name}\"\nbase_url = \"{base_url}\"\nwire_api = \"responses\"\n"
        ),
    })
}

/// 关闭写进 `[[skills.config]]` 之后，切换供应商、同步 MCP 都不会把它冲掉；反过来，
/// 打开时也只删这一条，供应商字段和 MCP 段原样留着。
#[test]
fn codex_skill_rule_coexists_with_provider_switch_and_mcp_sync() {
    use cc_switch_lib::{
        McpApps, McpServer, McpService, MultiAppConfig, Provider, ProviderService,
    };

    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    fs::create_dir_all(home.join(".codex")).expect("create codex dir");
    fs::write(
        cc_switch_lib::get_codex_config_path(),
        "# 用户的注释\napproval_policy = \"on-request\"\n",
    )
    .expect("seed config.toml");

    let mut config = MultiAppConfig::default();
    {
        let manager = config
            .get_manager_mut(&AppType::Codex)
            .expect("codex manager");
        for (id, model, url) in [
            ("alpha", "gpt-a", "https://a.example/v1"),
            ("beta", "gpt-b", "https://b.example/v1"),
        ] {
            manager.providers.insert(
                id.to_string(),
                Provider::with_id(
                    id.to_string(),
                    id.to_string(),
                    codex_row(id, model, url),
                    None,
                ),
            );
        }
    }
    let state = support::create_test_state_with_config(&config).expect("create test state");
    ProviderService::switch(&state, AppType::Codex, "alpha").expect("switch to alpha");

    write_skill(
        &SkillService::get_ssot_dir().unwrap().join("demo-skill"),
        "demo",
    );
    let skill = native_test_skill(
        "local:demo-skill",
        "demo-skill",
        SkillApps {
            codex: true,
            ..Default::default()
        },
    );
    state.db.save_skill(&skill).expect("save skill");

    let read = || fs::read_to_string(cc_switch_lib::get_codex_config_path()).unwrap();

    SkillService::toggle_app(&state.db, &skill.id, &AppType::Codex, false).expect("disable");
    let text = read();
    assert!(
        text.contains("[[skills.config]]\nname = \"demo\"\nenabled = false"),
        "{text}"
    );

    ProviderService::switch(&state, AppType::Codex, "beta").expect("switch to beta");
    McpService::upsert_server(
        &state,
        McpServer {
            id: "echo".into(),
            name: "Echo".into(),
            server: serde_json::json!({ "type": "stdio", "command": "echo" }),
            apps: McpApps {
                codex: true,
                ..Default::default()
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("sync MCP");
    let text = read();
    assert!(text.contains("model = \"gpt-b\""), "{text}");
    assert!(text.contains("[mcp_servers.echo]"), "{text}");
    assert!(text.contains("name = \"demo\"\nenabled = false"), "{text}");
    assert!(text.starts_with("# 用户的注释\n"), "{text}");

    SkillService::toggle_app(&state.db, &skill.id, &AppType::Codex, true).expect("enable");
    let text = read();
    assert!(!text.contains("skills"), "{text}");
    assert!(text.contains("model = \"gpt-b\""), "{text}");
    assert!(text.contains("[mcp_servers.echo]"), "{text}");
    assert!(
        state
            .db
            .get_installed_skill(&skill.id)
            .unwrap()
            .unwrap()
            .apps
            .codex
    );
}

/// 关掉 Claude 里的 Skill 写 `skillOverrides`，个人目录里用户自己放的同名目录不删。
#[test]
fn claude_disable_writes_override_and_keeps_a_user_owned_directory() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    let settings_path = cc_switch_lib::get_claude_settings_path();
    fs::create_dir_all(settings_path.parent().unwrap()).expect("create claude dir");
    fs::write(&settings_path, "{\n  \"model\": \"opus\"\n}\n").expect("seed settings");

    let state = create_test_state().expect("create test state");
    write_skill(
        &SkillService::get_ssot_dir().unwrap().join("demo-skill"),
        "demo",
    );
    let user_copy = home.join(".claude").join("skills").join("demo-skill");
    write_skill(&user_copy, "demo-user-edit");
    let skill = native_test_skill(
        "local:demo-skill",
        "demo-skill",
        SkillApps {
            claude: true,
            ..Default::default()
        },
    );
    state.db.save_skill(&skill).expect("save skill");

    SkillService::toggle_app(&state.db, &skill.id, &AppType::Claude, false).expect("disable");
    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    assert_eq!(settings["skillOverrides"]["demo"], "off");
    assert_eq!(settings["model"], "opus");
    assert!(
        fs::read_to_string(user_copy.join("SKILL.md"))
            .unwrap()
            .contains("demo-user-edit"),
        "a directory CC Switch did not place must not be deleted"
    );

    // 切换供应商时的重新同步也不能把它当成「关掉的投影」删掉。
    SkillService::sync_to_app(&state.db, &AppType::Claude).expect("resync");
    assert!(user_copy.join("SKILL.md").exists());

    // 打开也不拿 SSOT 的版本盖掉用户的目录：拒绝，什么都不改。
    SkillService::toggle_app(&state.db, &skill.id, &AppType::Claude, true)
        .expect_err("enable must not overwrite a user-owned directory");
    assert!(fs::read_to_string(user_copy.join("SKILL.md"))
        .unwrap()
        .contains("demo-user-edit"));
    assert!(fs::read_to_string(&settings_path)
        .unwrap()
        .contains("\"off\""));

    // 用户把自己的目录挪走之后就能打开，关闭项随之撤掉。
    fs::remove_dir_all(&user_copy).unwrap();
    SkillService::toggle_app(&state.db, &skill.id, &AppType::Claude, true).expect("enable");
    assert_eq!(
        fs::read_to_string(&settings_path).unwrap(),
        "{\n  \"model\": \"opus\"\n}\n"
    );
}

/// Pi 也从 `~/.agents/skills` 加载：只在那里的 Skill 也能关，状态按原生事实显示。
#[test]
fn pi_can_disable_a_skill_it_loads_from_the_agents_root() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    let agent_dir = home.join(".pi").join("agent");
    fs::create_dir_all(&agent_dir).expect("create pi agent dir");
    let settings_path = agent_dir.join("settings.json");
    fs::write(&settings_path, "{\n  \"defaultModel\": \"m\"\n}").expect("seed pi settings");

    let state = create_test_state().expect("create test state");
    write_skill(
        &SkillService::get_ssot_dir().unwrap().join("demo-skill"),
        "demo",
    );
    write_skill(
        &home.join(".agents").join("skills").join("demo-skill"),
        "demo",
    );
    let skill = native_test_skill("local:demo-skill", "demo-skill", SkillApps::default());
    state.db.save_skill(&skill).expect("save skill");

    let pi_state = || {
        SkillService::get_all_installed(&state.db).unwrap()[0]
            .apps
            .pi
    };
    assert!(
        pi_state(),
        "Pi loads ~/.agents/skills, so the skill is active"
    );

    SkillService::toggle_app(&state.db, &skill.id, &AppType::Pi, false).expect("disable");
    assert!(!pi_state());
    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    let skills = settings["skills"].as_array().expect("skills array");
    let agents_entry = format!(
        "-{}",
        home.join(".agents")
            .join("skills")
            .join("demo-skill")
            .to_string_lossy()
            .replace('\\', "/")
    );
    assert!(
        skills
            .iter()
            .any(|v| v.as_str() == Some(agents_entry.as_str())),
        "{skills:?}"
    );
    assert!(home
        .join(".agents")
        .join("skills")
        .join("demo-skill")
        .exists());

    SkillService::toggle_app(&state.db, &skill.id, &AppType::Pi, true).expect("enable");
    assert!(pi_state());
    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    assert!(settings.get("skills").is_none(), "{settings}");
    clean_native_roots(home);
}

/// 卸载时撤掉 CC Switch 写过的原生关闭项，不留下关着同名 Skill 的孤儿条目。
#[test]
fn uninstall_clears_native_disable_rules() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    fs::create_dir_all(home.join(".codex")).expect("create codex dir");
    let settings_path = cc_switch_lib::get_claude_settings_path();
    fs::create_dir_all(settings_path.parent().unwrap()).expect("create claude dir");

    let state = create_test_state().expect("create test state");
    write_skill(
        &SkillService::get_ssot_dir().unwrap().join("demo-skill"),
        "demo",
    );
    let skill = native_test_skill(
        "local:demo-skill",
        "demo-skill",
        SkillApps {
            claude: true,
            codex: true,
            ..Default::default()
        },
    );
    state.db.save_skill(&skill).expect("save skill");
    SkillService::toggle_app(&state.db, &skill.id, &AppType::Claude, false).unwrap();
    SkillService::toggle_app(&state.db, &skill.id, &AppType::Codex, false).unwrap();
    assert!(fs::read_to_string(cc_switch_lib::get_codex_config_path())
        .unwrap()
        .contains("name = \"demo\""));

    SkillService::uninstall(&state.db, &skill.id).expect("uninstall");
    assert!(!fs::read_to_string(cc_switch_lib::get_codex_config_path())
        .unwrap()
        .contains("demo"));
    assert!(!fs::read_to_string(&settings_path).unwrap().contains("demo"));
}

fn select(skill: &cc_switch_lib::UnmanagedSkill, apps: SkillApps) -> ImportSkillSelection {
    ImportSkillSelection {
        directory: skill.directory.clone(),
        apps,
        source_path: Some(skill.path.clone()),
    }
}

/// 嵌套仓库（skills/<repo>/<skill>/SKILL.md）拆成每个 Skill 一条；Claude Code 只认一层，
/// 导入后在 ~/.claude/skills 下投影出能用的那一层。账号同步的 synced/ 不列。
#[test]
fn scan_splits_nested_repositories_and_skips_synced() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    let claude_skills = home.join(".claude").join("skills");
    write_skill(&claude_skills.join("my-repo").join("alpha"), "alpha");
    write_skill(
        &claude_skills.join("my-repo").join("group").join("beta"),
        "beta",
    );
    write_skill(
        &claude_skills.join("synced").join("account-skill"),
        "account",
    );

    let state = create_test_state().expect("create test state");
    let found = SkillService::scan_unmanaged(&state.db).expect("scan");
    let mut names: Vec<&str> = found.iter().map(|s| s.directory.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["alpha", "beta"]);

    let beta = found.iter().find(|s| s.directory == "beta").unwrap();
    SkillService::import_from_apps(
        &state.db,
        vec![select(
            beta,
            SkillApps {
                claude: true,
                ..Default::default()
            },
        )],
    )
    .expect("import nested skill");
    assert!(SkillService::get_ssot_dir()
        .unwrap()
        .join("beta")
        .join("SKILL.md")
        .exists());
    assert!(
        claude_skills.join("beta").join("SKILL.md").exists(),
        "Claude Code only loads one level, so the skill is projected there"
    );
}

/// 同名同内容合成一条，记全出处；同名不同内容各列一条并标冲突，导入用户挑的那份。
#[test]
fn scan_dedupes_by_name_and_content_and_imports_the_chosen_version() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    write_skill(&home.join(".claude").join("skills").join("same"), "same");
    write_skill(&home.join(".agents").join("skills").join("same"), "same");
    write_skill(
        &home.join(".claude").join("skills").join("forked"),
        "forked",
    );
    let agents_forked = home.join(".agents").join("skills").join("forked");
    write_skill(&agents_forked, "forked");
    fs::write(agents_forked.join("notes.md"), "agents edition").unwrap();

    let state = create_test_state().expect("create test state");
    let found = SkillService::scan_unmanaged(&state.db).expect("scan");

    let same: Vec<_> = found.iter().filter(|s| s.directory == "same").collect();
    assert_eq!(same.len(), 1);
    assert!(!same[0].conflict);
    assert!(same[0].found_in.contains(&"claude".to_string()));
    assert!(same[0].found_in.contains(&"agents".to_string()));

    let forked: Vec<_> = found.iter().filter(|s| s.directory == "forked").collect();
    assert_eq!(forked.len(), 2);
    assert!(forked.iter().all(|s| s.conflict));
    let agents_version = forked
        .iter()
        .find(|s| s.found_in == vec!["agents".to_string()])
        .unwrap();
    SkillService::import_from_apps(
        &state.db,
        vec![select(agents_version, SkillApps::default())],
    )
    .expect("import the agents version");
    assert_eq!(
        fs::read_to_string(
            SkillService::get_ssot_dir()
                .unwrap()
                .join("forked")
                .join("notes.md")
        )
        .unwrap(),
        "agents edition"
    );
    clean_native_roots(home);
}

/// CC Switch 目录里残留了没有记录、内容不同的同名目录：用户挑定版本导入时，残留做成备份
/// （能在「恢复备份」里找回），导入的是用户挑的那份。
#[test]
fn importing_a_chosen_version_backs_up_a_stale_ssot_leftover() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    let ssot_leftover = SkillService::get_ssot_dir().unwrap().join("codex-skill");
    write_skill(&ssot_leftover, "Stale SSOT Skill");
    let codex_skill_dir = home.join(".codex").join("skills").join("codex-skill");
    write_skill(&codex_skill_dir, "Live Codex Skill");

    let state = create_test_state().expect("create test state");
    let found = SkillService::scan_unmanaged(&state.db).expect("scan");
    let live = found
        .iter()
        .find(|s| s.name == "Live Codex Skill")
        .expect("live version listed");
    assert!(live.conflict);

    SkillService::import_from_apps(&state.db, vec![select(live, SkillApps::default())])
        .expect("import the live version");
    assert!(fs::read_to_string(ssot_leftover.join("SKILL.md"))
        .unwrap()
        .contains("Live Codex Skill"));
    let backups = SkillService::list_backups().expect("list backups");
    assert!(
        backups
            .iter()
            .any(|backup| backup.skill.name == "Stale SSOT Skill"),
        "the leftover is restorable"
    );
}

/// 项目扫描：列出项目里的 Skills，导入即复制进 CC Switch（项目文件不动）；Skills 目录以外的
/// 路径不接受。
#[test]
fn project_scan_lists_project_skills_and_rejects_foreign_paths() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    let project = tempfile::tempdir().expect("project dir");
    write_skill(
        &project
            .path()
            .join(".claude")
            .join("skills")
            .join("proj-skill"),
        "proj",
    );
    write_skill(
        &project
            .path()
            .join(".agents")
            .join("skills")
            .join("shared-proj"),
        "shared",
    );
    let stray = project.path().join("docs").join("stray");
    write_skill(&stray, "stray");

    let state = create_test_state().expect("create test state");
    let found = SkillService::scan_project(&state.db, project.path()).expect("scan project");
    let proj = found
        .iter()
        .find(|s| s.directory == "proj-skill")
        .expect("project skill listed");
    assert_eq!(proj.found_in, vec!["project:.claude/skills".to_string()]);
    assert!(found.iter().any(|s| s.directory == "shared-proj"));
    assert!(!found.iter().any(|s| s.directory == "stray"));

    SkillService::import_from_apps(&state.db, vec![select(proj, SkillApps::default())])
        .expect("promote project skill");
    assert!(SkillService::get_ssot_dir()
        .unwrap()
        .join("proj-skill")
        .exists());
    assert!(project
        .path()
        .join(".claude/skills/proj-skill/SKILL.md")
        .exists());

    let err = SkillService::import_from_apps(
        &state.db,
        vec![ImportSkillSelection {
            directory: "stray".to_string(),
            apps: SkillApps::default(),
            source_path: Some(stray.display().to_string()),
        }],
    )
    .expect_err("a path outside any skills directory is refused");
    assert!(err.to_string().contains("stray"), "{err}");
    assert!(!SkillService::get_ssot_dir().unwrap().join("stray").exists());
}

/// 卡片上的提示：勾了却读不到、没勾却仍会加载、读到的是用户自己的同名目录。
#[test]
fn app_notes_report_where_switches_and_loading_disagree() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    let state = create_test_state().expect("create test state");

    for dir in ["missing", "agents-only", "users-own"] {
        write_skill(&SkillService::get_ssot_dir().unwrap().join(dir), dir);
    }
    // 勾了 Claude，但 ~/.claude/skills 里没有它。
    state
        .db
        .save_skill(&native_test_skill(
            "local:missing",
            "missing",
            SkillApps {
                claude: true,
                ..Default::default()
            },
        ))
        .unwrap();
    // 没勾 Codex，可 ~/.agents/skills 里有它，Codex 又没装（写不了关闭配置）。
    write_skill(
        &home.join(".agents").join("skills").join("agents-only"),
        "agents-only",
    );
    state
        .db
        .save_skill(&native_test_skill(
            "local:agents-only",
            "agents-only",
            SkillApps::default(),
        ))
        .unwrap();
    // 勾了 Claude，但 ~/.claude/skills 里是用户自己的同名目录。
    let users_own = home.join(".claude").join("skills").join("users-own");
    write_skill(&users_own, "users-own");
    fs::write(users_own.join("mine.md"), "edited by the user").unwrap();
    state
        .db
        .save_skill(&native_test_skill(
            "local:users-own",
            "users-own",
            SkillApps {
                claude: true,
                ..Default::default()
            },
        ))
        .unwrap();

    let notes = SkillService::app_notes(&state.db).expect("notes");
    let has = |id: &str, app: &str, state: &str| {
        notes
            .iter()
            .any(|note| note.id == id && note.app == app && note.state == state)
    };
    assert!(has("local:missing", "claude", "notLoaded"), "{notes:?}");
    assert!(
        has("local:agents-only", "codex", "stillLoaded"),
        "{notes:?}"
    );
    assert!(
        has("local:agents-only", "gemini", "stillLoaded"),
        "{notes:?}"
    );
    assert!(has("local:users-own", "claude", "external"), "{notes:?}");
    // OpenCode 也读 ~/.claude/skills。
    assert!(
        has("local:users-own", "opencode", "stillLoaded"),
        "{notes:?}"
    );

    // 装了 Codex 之后，关掉就会写进它的配置，提示随之消失。
    fs::create_dir_all(home.join(".codex")).unwrap();
    state
        .db
        .save_skill(&native_test_skill(
            "local:agents-only",
            "agents-only",
            SkillApps {
                codex: true,
                ..Default::default()
            },
        ))
        .unwrap();
    SkillService::toggle_app(&state.db, "local:agents-only", &AppType::Codex, false).unwrap();
    let notes = SkillService::app_notes(&state.db).expect("notes");
    assert!(
        !notes
            .iter()
            .any(|note| note.id == "local:agents-only" && note.app == "codex"),
        "{notes:?}"
    );
    clean_native_roots(home);
}

/// 旧版本取消勾选只是不投影：~/.agents/skills 里的 Skill Codex 照样加载。装了 Codex 时提示
/// 「能关」，「立即重新同步」把关闭项补进 Codex 的配置。
#[test]
fn resync_disables_leftovers_an_app_still_loads() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let home = ensure_test_home();
    clean_native_roots(home);
    fs::create_dir_all(home.join(".codex")).unwrap();
    let state = create_test_state().expect("create test state");
    write_skill(
        &SkillService::get_ssot_dir().unwrap().join("legacy"),
        "legacy",
    );
    write_skill(
        &home.join(".agents").join("skills").join("legacy"),
        "legacy",
    );
    state
        .db
        .save_skill(&native_test_skill(
            "local:legacy",
            "legacy",
            SkillApps::default(),
        ))
        .unwrap();

    let codex_note = |state: &cc_switch_lib::AppState| {
        SkillService::app_notes(&state.db)
            .unwrap()
            .into_iter()
            .find(|note| note.id == "local:legacy" && note.app == "codex")
            .map(|note| note.state)
    };
    assert_eq!(codex_note(&state).as_deref(), Some("notDisabled"));

    let outcomes = SkillService::resync_all_apps(&state.db);
    assert!(outcomes.iter().all(|outcome| outcome.ok), "{outcomes:?}");
    assert!(fs::read_to_string(cc_switch_lib::get_codex_config_path())
        .unwrap()
        .contains("name = \"legacy\"\nenabled = false"));
    assert_eq!(codex_note(&state), None);
    clean_native_roots(home);
}
