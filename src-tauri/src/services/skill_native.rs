//! Skills 的原生开关：把「在这个应用里关掉某个 Skill」写进应用自己的配置。
//!
//! 只删投影目录关不掉 Skill：Codex 和 Pi 还会从 `~/.agents/skills` 加载，Claude Code 的
//! 个人目录里也可能有用户自己放的同名目录。三家都有按 Skill 关闭的原生配置，这里只增删
//! CC Switch 自己写的那几条，其余内容原样保留：
//!
//! - Claude Code：`~/.claude/settings.json` 的 `skillOverrides["<name>"] = "off"`；
//! - Codex：`~/.codex/config.toml` 的 `[[skills.config]] name = "<name>", enabled = false`
//!   （只在用户层生效，见 openai/codex#20210）；
//! - Pi：`~/.pi/agent/settings.json` 的 `skills` 数组里的 `-<绝对路径>`，两个全局根
//!   （Pi 自己的目录和 `~/.agents/skills`）各一条；
//! - Gemini CLI：`~/.gemini/settings.json` 的 `skills.disabled` 数组里的技能名（不分大小写，
//!   和 `/skills disable` 写的是同一处）；
//! - OpenCode：`opencode.json(c)` 的 `permission.skill["<name>"] = "deny"`（deny 即对 agent
//!   隐藏）。
//!
//! 写入都和这个应用自己的其他写入方排队：Claude Code、Codex、Pi、Gemini CLI 走写入引擎
//! （[`crate::mode::operation::run_files_only`]），和切换供应商、编辑器、MCP 同步共用同一把
//! 写锁，发布前重读比对；OpenCode 走它自己的 JSONC 编辑器和配置锁，注释原样保留。

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value as JsonValue;
use toml_edit::{
    ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value as TomlValue,
};

use crate::app_config::AppType;
use crate::live::engine::LiveFile;
use crate::live::patch::json::{self, JsonPatch};
use crate::live::patch::{KeyPath, LivePatch, LiveWriteError};
use crate::mode::operation::{run_files_only, FileChange};
use crate::mode::state::op;

/// Claude Code `skillOverrides` 里表示关闭的值。
const CLAUDE_OFF: &str = "off";
const CLAUDE_OVERRIDES_KEY: &str = "skillOverrides";

/// 一个 Skill 在原生配置里的身份。
#[derive(Debug, Clone)]
pub(crate) struct NativeSkill {
    /// 技能目录名（各应用 skills 目录下的那一层）。
    pub directory: String,
    /// 技能名：SKILL.md frontmatter 的 `name`，没有时是目录名。Claude Code 的
    /// `skillOverrides` 和 Codex 的 name 选择器都按它匹配。
    pub name: String,
}

/// 有原生按 Skill 关闭配置、由这里接管开关的应用。
pub(crate) fn supports(app: &AppType) -> bool {
    matches!(
        app,
        AppType::Claude | AppType::Codex | AppType::Pi | AppType::Gemini | AppType::OpenCode
    )
}

/// 在 `app` 的原生配置里关闭（`disabled = true`）或取消关闭这个 Skill。
///
/// 应用的配置目录不存在（没装这个应用）时什么都不写；取消关闭时配置文件不存在也不创建。
pub(crate) fn set_disabled(app: &AppType, skill: &NativeSkill, disabled: bool) -> Result<()> {
    let Some(target) = Target::resolve(app)? else {
        return Ok(());
    };
    let config_dir_exists = target
        .file
        .path
        .parent()
        .is_some_and(|parent| parent.exists());
    if !config_dir_exists || (!disabled && !target.file.path.exists()) {
        return Ok(());
    }
    if matches!(app, AppType::OpenCode) {
        crate::opencode_config::set_skill_denied(&skill.name, disabled)?;
        return Ok(());
    }

    let patch: Box<dyn LivePatch> = match app {
        AppType::Claude => Box::new(ClaudeOverridePatch {
            name: skill.name.clone(),
            disabled,
        }),
        AppType::Codex => Box::new(CodexSkillPatch {
            name: skill.name.clone(),
            paths: codex_skill_paths(&skill.directory),
            disabled,
        }),
        AppType::Pi => Box::new(PiExcludePatch {
            entries: pi_exclude_entries(&skill.directory)?,
            disabled,
        }),
        AppType::Gemini => Box::new(GeminiDisabledPatch {
            name: skill.name.clone(),
            disabled,
        }),
        _ => return Ok(()),
    };

    run_files_only(
        app.as_str(),
        op::SKILLS,
        &[FileChange {
            file: target.file,
            patch: patch.as_ref(),
        }],
    )?;
    Ok(())
}

/// 这个 Skill 是否被 CC Switch 写的原生配置关掉了。读不出或解析不了时按未关闭算。
pub(crate) fn is_disabled(app: &AppType, skill: &NativeSkill) -> bool {
    if matches!(app, AppType::OpenCode) {
        return crate::opencode_config::skill_denied(&skill.name);
    }
    let Ok(Some(target)) = Target::resolve(app) else {
        return false;
    };
    let Ok(Some(bytes)) = crate::live::engine::read_current(&target.file.path) else {
        return false;
    };
    let path = target.file.path.as_path();
    match app {
        AppType::Claude => json::parse(path, Some(&bytes))
            .ok()
            .and_then(|(doc, _)| {
                json::value_at(
                    &doc,
                    &KeyPath::new(&[CLAUDE_OVERRIDES_KEY, skill.name.as_str()]),
                )
                .cloned()
            })
            .is_some_and(|value| value == CLAUDE_OFF),
        AppType::Codex => crate::live::patch::toml::parse(path, Some(&bytes))
            .ok()
            .is_some_and(|doc| codex_name_disabled(&doc, &skill.name)),
        AppType::Pi => {
            let Ok(entries) = pi_exclude_entries(&skill.directory) else {
                return false;
            };
            json::parse(path, Some(&bytes))
                .ok()
                .and_then(|(doc, _)| doc.get("skills").cloned())
                .and_then(|skills| skills.as_array().cloned())
                .is_some_and(|skills| {
                    entries
                        .iter()
                        .all(|entry| skills.iter().any(|value| value.as_str() == Some(entry)))
                })
        }
        AppType::Gemini => json::parse(path, Some(&bytes))
            .ok()
            .and_then(|(doc, _)| {
                json::value_at(&doc, &KeyPath::new(&["skills", "disabled"])).cloned()
            })
            .and_then(|disabled| disabled.as_array().cloned())
            .is_some_and(|disabled| {
                disabled
                    .iter()
                    .any(|value| same_gemini_name(value, &skill.name))
            }),
        _ => false,
    }
}

struct Target {
    file: LiveFile,
}

impl Target {
    fn resolve(app: &AppType) -> Result<Option<Self>> {
        Ok(Some(match app {
            AppType::Claude => Self {
                file: LiveFile::private(crate::config::get_claude_settings_path()),
            },
            AppType::Codex => Self {
                file: LiveFile::private(crate::codex_config::get_codex_config_path()),
            },
            AppType::Pi => Self {
                file: LiveFile::shared(crate::pi_config::get_pi_settings_path()?),
            },
            AppType::Gemini => Self {
                file: LiveFile::shared(crate::gemini_config::get_gemini_settings_path()),
            },
            // 只用来判断配置目录、文件在不在；写入走 OpenCode 自己的编辑器。
            AppType::OpenCode => Self {
                file: LiveFile::shared(crate::opencode_config::get_opencode_config_path()?),
            },
            _ => return Ok(None),
        }))
    }
}

/// 内容没变就原样交回写前的字节：重新序列化会规范化空白和转义，不该为此算一次写入。
fn unchanged_or_serialize(
    path: &Path,
    pre: Option<&[u8]>,
    before: &JsonValue,
    after: &JsonValue,
    style: &json::JsonStyle,
) -> Result<Vec<u8>, LiveWriteError> {
    match pre {
        Some(bytes) if before == after => Ok(bytes.to_vec()),
        _ => json::serialize(path, after, style),
    }
}

// ========== Claude Code ==========

struct ClaudeOverridePatch {
    name: String,
    disabled: bool,
}

impl LivePatch for ClaudeOverridePatch {
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let (mut doc, style) = json::parse(path, pre)?;
        let before = doc.clone();
        let key = KeyPath::new(&[CLAUDE_OVERRIDES_KEY, self.name.as_str()]);
        let patch = if self.disabled {
            JsonPatch {
                set: vec![(key, JsonValue::from(CLAUDE_OFF))],
                ..JsonPatch::default()
            }
        } else {
            // 只撤掉 CC Switch 写的 "off"；用户自己设的 name-only 等档位不动。
            JsonPatch {
                remove_if: vec![(key, vec![JsonValue::from(CLAUDE_OFF)])],
                ..JsonPatch::default()
            }
        };
        let had_overrides = doc
            .get(CLAUDE_OVERRIDES_KEY)
            .and_then(JsonValue::as_object)
            .is_some_and(|map| !map.is_empty());
        patch.apply_to(path, &mut doc)?;
        // 撤掉的是最后一条，就把空对象一起删掉，不在用户的设置里留空壳。
        if had_overrides
            && doc
                .get(CLAUDE_OVERRIDES_KEY)
                .and_then(JsonValue::as_object)
                .is_some_and(|map| map.is_empty())
        {
            if let Some(root) = doc.as_object_mut() {
                root.shift_remove(CLAUDE_OVERRIDES_KEY);
            }
        }
        unchanged_or_serialize(path, pre, &before, &doc, &style)
    }
}

// ========== Gemini CLI ==========

/// Gemini CLI 按名字不分大小写匹配 `skills.disabled`。
fn same_gemini_name(value: &JsonValue, name: &str) -> bool {
    value
        .as_str()
        .is_some_and(|value| value.to_lowercase() == name.to_lowercase())
}

struct GeminiDisabledPatch {
    name: String,
    disabled: bool,
}

impl LivePatch for GeminiDisabledPatch {
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let (mut doc, style) = json::parse(path, pre)?;
        let before = doc.clone();
        let shape = |segments: &[&str], expected: &'static str| LiveWriteError::Shape {
            path: path.to_path_buf(),
            key_path: KeyPath::new(segments),
            expected,
        };
        let root = doc.as_object_mut().ok_or_else(|| shape(&[], "对象"))?;
        if !root.contains_key("skills") {
            if !self.disabled {
                return Ok(pre.unwrap_or_default().to_vec());
            }
            root.insert("skills".to_string(), JsonValue::Object(Default::default()));
        }
        let skills = root
            .get_mut("skills")
            .and_then(JsonValue::as_object_mut)
            .ok_or_else(|| shape(&["skills"], "对象"))?;
        if !skills.contains_key("disabled") {
            if !self.disabled {
                return Ok(pre.unwrap_or_default().to_vec());
            }
            skills.insert("disabled".to_string(), JsonValue::Array(Vec::new()));
        }
        let disabled = skills
            .get_mut("disabled")
            .and_then(JsonValue::as_array_mut)
            .ok_or_else(|| shape(&["skills", "disabled"], "数组"))?;
        if self.disabled {
            if !disabled
                .iter()
                .any(|value| same_gemini_name(value, &self.name))
            {
                disabled.push(JsonValue::from(self.name.as_str()));
            }
        } else {
            let had_entries = !disabled.is_empty();
            disabled.retain(|value| !same_gemini_name(value, &self.name));
            // 删到空就把自己建的空壳一并收掉。
            if had_entries && disabled.is_empty() {
                skills.shift_remove("disabled");
                if skills.is_empty() {
                    root.shift_remove("skills");
                }
            }
        }
        unchanged_or_serialize(path, pre, &before, &doc, &style)
    }
}

// ========== Codex ==========

/// Codex 可能加载到这个 Skill 的 `SKILL.md` 路径：用来清掉指向它的旧 path 选择器。
fn codex_skill_paths(directory: &str) -> Vec<PathBuf> {
    let mut roots = vec![crate::config::get_home_dir().join(".agents").join("skills")];
    if let Ok(dir) = crate::services::skill::SkillService::get_app_skills_dir(&AppType::Codex) {
        roots.push(dir);
    }
    if let Ok(dir) = crate::services::skill::SkillService::get_ssot_dir() {
        roots.push(dir);
    }
    roots
        .into_iter()
        .map(|root| root.join(directory).join("SKILL.md"))
        .collect()
}

struct CodexSkillPatch {
    name: String,
    /// 指向这个 Skill 的 `SKILL.md` 的路径；打开时一并清掉关闭它的 path 选择器。
    paths: Vec<PathBuf>,
    disabled: bool,
}

impl CodexSkillPatch {
    /// 这条 `[[skills.config]]` 是否该在这次写入里删掉。
    ///
    /// 关闭时删掉同名的所有 name 选择器，再在末尾追加一条 `enabled = false`（后写的规则
    /// 覆盖先写的）；打开时删掉同名的 `enabled = false`，以及指向这个 Skill 的
    /// `enabled = false` path 选择器。
    fn doomed(&self, entry: &dyn TableLike) -> bool {
        let name = entry.get("name").and_then(Item::as_str);
        let enabled = entry.get("enabled").and_then(Item::as_bool);
        if name == Some(self.name.as_str()) {
            return self.disabled || enabled == Some(false);
        }
        if self.disabled || enabled != Some(false) {
            return false;
        }
        entry
            .get("path")
            .and_then(Item::as_str)
            .is_some_and(|path| self.points_here(Path::new(path)))
    }

    fn points_here(&self, path: &Path) -> bool {
        let canonical = path.canonicalize().ok();
        self.paths.iter().any(|candidate| {
            candidate == path
                || canonical.as_deref().is_some_and(|canonical| {
                    candidate.canonicalize().ok().as_deref() == Some(canonical)
                })
        })
    }

    fn new_entry(&self) -> Table {
        let mut table = Table::new();
        table.insert("name", toml_edit::value(self.name.as_str()));
        table.insert("enabled", toml_edit::value(false));
        table
    }
}

impl LivePatch for CodexSkillPatch {
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let mut doc = crate::live::patch::toml::parse(path, pre)?;
        self.apply_to(path, &mut doc)?;
        Ok(doc.to_string().into_bytes())
    }
}

impl CodexSkillPatch {
    fn apply_to(&self, path: &Path, doc: &mut DocumentMut) -> Result<(), LiveWriteError> {
        let shape = |segments: &[&str], expected: &'static str| LiveWriteError::Shape {
            path: path.to_path_buf(),
            key_path: KeyPath::new(segments),
            expected,
        };
        let root = doc.as_table_mut();

        if !root.contains_key("skills") {
            if !self.disabled {
                return Ok(());
            }
            let mut skills = Table::new();
            skills.set_implicit(true);
            root.insert("skills", Item::Table(skills));
        }
        let skills = root
            .get_mut("skills")
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| shape(&["skills"], "表"))?;

        if !skills.contains_key("config") {
            if !self.disabled {
                return Ok(());
            }
            skills.insert("config", Item::ArrayOfTables(ArrayOfTables::new()));
        }
        let config = skills
            .get_mut("config")
            .expect("skills.config was just ensured");
        let now_empty = match config {
            Item::ArrayOfTables(entries) => {
                let doomed: Vec<usize> = entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| self.doomed(*entry))
                    .map(|(index, _)| index)
                    .collect();
                for index in doomed.into_iter().rev() {
                    entries.remove(index);
                }
                if self.disabled {
                    entries.push(self.new_entry());
                }
                entries.is_empty()
            }
            Item::Value(TomlValue::Array(entries)) => {
                let mut doomed = Vec::new();
                for (index, entry) in entries.iter().enumerate() {
                    let table = entry
                        .as_inline_table()
                        .ok_or_else(|| shape(&["skills", "config"], "表的数组"))?;
                    if self.doomed(table) {
                        doomed.push(index);
                    }
                }
                for index in doomed.into_iter().rev() {
                    entries.remove(index);
                }
                if self.disabled {
                    let mut inline = InlineTable::new();
                    inline.insert("name", TomlValue::from(self.name.as_str()));
                    inline.insert("enabled", TomlValue::from(false));
                    entries.push(inline);
                }
                entries.is_empty()
            }
            _ => return Err(shape(&["skills", "config"], "表的数组")),
        };

        // 删到空就把 `config` 收掉；`skills` 里也没别的键了，再把它收掉。
        if now_empty {
            skills.remove("config");
            if skills.is_empty() {
                root.remove("skills");
            }
        }
        Ok(())
    }
}

/// 文档里是否有关闭这个名字的 name 选择器（按顺序，后面的覆盖前面的）。
fn codex_name_disabled(doc: &DocumentMut, name: &str) -> bool {
    let Some(config) = doc
        .get("skills")
        .and_then(Item::as_table_like)
        .and_then(|skills| skills.get("config"))
    else {
        return false;
    };
    let entries: Vec<&dyn TableLike> = match config {
        Item::ArrayOfTables(entries) => entries.iter().map(|t| t as &dyn TableLike).collect(),
        Item::Value(TomlValue::Array(entries)) => entries
            .iter()
            .filter_map(TomlValue::as_inline_table)
            .map(|t| t as &dyn TableLike)
            .collect(),
        _ => return false,
    };
    entries
        .into_iter()
        .filter(|entry| entry.get("name").and_then(Item::as_str) == Some(name))
        .filter_map(|entry| entry.get("enabled").and_then(Item::as_bool))
        .last()
        .is_some_and(|enabled| !enabled)
}

// ========== Pi ==========

/// 这个 Skill 在 Pi 用户 settings 里的排除项：两个全局根各一条精确路径（`-` 前缀）。
///
/// Pi 对用户层自动发现的 `~/.pi/agent/skills` 和 `~/.agents/skills` 都套用用户 settings
/// 的 `skills` 覆盖规则；`-path` 按 `SKILL.md` 所在目录的绝对路径精确匹配。
fn pi_exclude_entries(directory: &str) -> Result<Vec<String>> {
    let roots = [
        crate::pi_config::get_pi_agent_dir()?.join("skills"),
        crate::config::get_home_dir().join(".agents").join("skills"),
    ];
    let mut entries: Vec<String> = Vec::new();
    for root in roots {
        let entry = format!("-{}", posix(&root.join(directory)));
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

struct PiExcludePatch {
    entries: Vec<String>,
    disabled: bool,
}

impl LivePatch for PiExcludePatch {
    fn apply(&self, path: &Path, pre: Option<&[u8]>) -> Result<Vec<u8>, LiveWriteError> {
        let (mut doc, style) = json::parse(path, pre)?;
        let before = doc.clone();
        let root = doc.as_object_mut().ok_or_else(|| LiveWriteError::Shape {
            path: path.to_path_buf(),
            key_path: KeyPath::root(),
            expected: "对象",
        })?;
        if !root.contains_key("skills") {
            if !self.disabled {
                return Ok(pre.unwrap_or_default().to_vec());
            }
            root.insert("skills".to_string(), JsonValue::Array(Vec::new()));
        }
        let skills = root
            .get_mut("skills")
            .and_then(JsonValue::as_array_mut)
            .ok_or_else(|| LiveWriteError::Shape {
                path: path.to_path_buf(),
                key_path: KeyPath::new(&["skills"]),
                expected: "数组",
            })?;
        let had_entries = !skills.is_empty();
        if self.disabled {
            for entry in &self.entries {
                if !skills.iter().any(|value| value.as_str() == Some(entry)) {
                    skills.push(JsonValue::from(entry.as_str()));
                }
            }
        } else {
            skills.retain(|value| {
                !value
                    .as_str()
                    .is_some_and(|value| self.entries.iter().any(|entry| entry == value))
            });
            if had_entries && skills.is_empty() {
                root.shift_remove("skills");
            }
        }
        unchanged_or_serialize(path, pre, &before, &doc, &style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(patch: &dyn LivePatch, pre: &str) -> String {
        String::from_utf8(
            patch
                .apply(Path::new("config"), Some(pre.as_bytes()))
                .expect("apply"),
        )
        .expect("utf8")
    }

    fn codex(name: &str, disabled: bool) -> CodexSkillPatch {
        CodexSkillPatch {
            name: name.to_string(),
            paths: vec![PathBuf::from("/home/u/.agents/skills/demo/SKILL.md")],
            disabled,
        }
    }

    const CODEX_LIVE: &str = r#"# 用户的注释
model_provider = "custom"
model = "gpt-a"

[model_providers.custom]
name = "A"

[[skills.config]]
path = "/opt/other/SKILL.md"
enabled = false

[mcp_servers.fs]
command = "fs-server"
"#;

    #[test]
    fn codex_disable_appends_a_name_rule_and_keeps_everything_else() {
        let out = apply(&codex("demo", true), CODEX_LIVE);
        let doc: DocumentMut = out.parse().unwrap();
        assert!(codex_name_disabled(&doc, "demo"));
        assert!(out.starts_with("# 用户的注释\nmodel_provider = \"custom\"\nmodel = \"gpt-a\"\n"));
        assert!(out.contains("path = \"/opt/other/SKILL.md\""));
        assert!(out.contains("[mcp_servers.fs]\ncommand = \"fs-server\""));
        // 重复关闭不会堆出第二条。
        assert_eq!(apply(&codex("demo", true), &out), out);
    }

    #[test]
    fn codex_enable_removes_only_our_rule_and_restores_the_file() {
        let disabled = apply(&codex("demo", true), CODEX_LIVE);
        let restored = apply(&codex("demo", false), &disabled);
        assert_eq!(restored, CODEX_LIVE);
    }

    #[test]
    fn codex_enable_clears_path_rules_pointing_at_the_skill_and_empty_tables() {
        let pre = "model = \"m\"\n\n[[skills.config]]\npath = \"/home/u/.agents/skills/demo/SKILL.md\"\nenabled = false\n";
        let out = apply(&codex("demo", false), pre);
        assert_eq!(out, "model = \"m\"\n");
    }

    #[test]
    fn codex_keeps_a_user_rule_that_enables_the_skill_on_enable() {
        let pre = "[[skills.config]]\nname = \"demo\"\nenabled = true\n";
        assert_eq!(apply(&codex("demo", false), pre), pre);
    }

    #[test]
    fn codex_handles_the_inline_array_form() {
        let pre = "skills = { config = [{ name = \"other\", enabled = false }] }\n";
        let out = apply(&codex("demo", true), pre);
        let doc: DocumentMut = out.parse().unwrap();
        assert!(codex_name_disabled(&doc, "demo"));
        assert!(codex_name_disabled(&doc, "other"));
        let back = apply(&codex("demo", false), &out);
        let doc: DocumentMut = back.parse().unwrap();
        assert!(!codex_name_disabled(&doc, "demo"));
        assert!(codex_name_disabled(&doc, "other"));
    }

    #[test]
    fn codex_refuses_a_skills_key_that_is_not_a_table() {
        let err = codex("demo", true)
            .apply(Path::new("config.toml"), Some(b"skills = \"x\"\n"))
            .expect_err("shape");
        assert!(matches!(err, LiveWriteError::Shape { .. }));
    }

    #[test]
    fn codex_enable_on_a_file_without_rules_is_a_noop() {
        let pre = "model = \"m\"\n";
        assert_eq!(apply(&codex("demo", false), pre), pre);
    }

    fn claude(disabled: bool) -> ClaudeOverridePatch {
        ClaudeOverridePatch {
            name: "demo".to_string(),
            disabled,
        }
    }

    #[test]
    fn claude_disable_sets_off_and_enable_removes_it_with_the_empty_object() {
        let pre = "{\n  \"env\": {\n    \"A\": \"1\"\n  }\n}\n";
        let off = apply(&claude(true), pre);
        let doc: JsonValue = serde_json::from_str(&off).unwrap();
        assert_eq!(doc["skillOverrides"]["demo"], "off");
        assert_eq!(doc["env"]["A"], "1");
        assert_eq!(apply(&claude(false), &off), pre);
    }

    #[test]
    fn claude_enable_keeps_a_user_chosen_state() {
        let pre = "{\n  \"skillOverrides\": {\n    \"demo\": \"name-only\"\n  }\n}\n";
        assert_eq!(apply(&claude(false), pre), pre);
    }

    #[test]
    fn json_noops_keep_the_original_bytes() {
        let pre = "{\"skillOverrides\":{\"demo\":\"name-only\"},\"x\":\"\\u00e9\"}";
        assert_eq!(apply(&claude(false), pre), pre);
        assert_eq!(apply(&pi(false), pre), pre);
    }

    #[test]
    fn claude_enable_keeps_other_overrides() {
        let pre =
            "{\n  \"skillOverrides\": {\n    \"demo\": \"off\",\n    \"other\": \"off\"\n  }\n}\n";
        let out = apply(&claude(false), pre);
        let doc: JsonValue = serde_json::from_str(&out).unwrap();
        assert!(doc["skillOverrides"].get("demo").is_none());
        assert_eq!(doc["skillOverrides"]["other"], "off");
    }

    fn pi(disabled: bool) -> PiExcludePatch {
        PiExcludePatch {
            entries: vec![
                "-/home/u/.pi/agent/skills/demo".to_string(),
                "-/home/u/.agents/skills/demo".to_string(),
            ],
            disabled,
        }
    }

    #[test]
    fn pi_disable_adds_both_roots_and_enable_restores_the_file() {
        let pre = "{\n  \"defaultModel\": \"m\",\n  \"skills\": [\n    \"!legacy\"\n  ]\n}";
        let off = apply(&pi(true), pre);
        let doc: JsonValue = serde_json::from_str(&off).unwrap();
        assert_eq!(
            doc["skills"],
            serde_json::json!([
                "!legacy",
                "-/home/u/.pi/agent/skills/demo",
                "-/home/u/.agents/skills/demo"
            ])
        );
        assert_eq!(apply(&pi(true), &off), off);
        assert_eq!(apply(&pi(false), &off), pre);
    }

    fn gemini(name: &str, disabled: bool) -> GeminiDisabledPatch {
        GeminiDisabledPatch {
            name: name.to_string(),
            disabled,
        }
    }

    #[test]
    fn gemini_disable_round_trips_and_matches_names_case_insensitively() {
        let pre = "{\n  \"skills\": {\n    \"enabled\": true,\n    \"disabled\": [\n      \"other\"\n    ]\n  }\n}";
        let off = apply(&gemini("Demo", true), pre);
        let doc: JsonValue = serde_json::from_str(&off).unwrap();
        assert_eq!(
            doc["skills"]["disabled"],
            serde_json::json!(["other", "Demo"])
        );
        assert_eq!(apply(&gemini("demo", true), &off), off, "already disabled");
        assert_eq!(apply(&gemini("DEMO", false), &off), pre);
    }

    #[test]
    fn gemini_cleans_up_what_it_created() {
        let pre = "{\n  \"security\": {}\n}";
        let off = apply(&gemini("demo", true), pre);
        assert_eq!(apply(&gemini("demo", false), &off), pre);
        assert_eq!(apply(&gemini("demo", false), pre), pre);
    }

    #[test]
    fn pi_enable_drops_the_array_it_emptied() {
        let pre = "{\n  \"defaultModel\": \"m\"\n}";
        let off = apply(&pi(true), pre);
        assert_eq!(apply(&pi(false), &off), pre);
    }
}
