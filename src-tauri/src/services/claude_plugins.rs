//! Claude Code 插件：列表和启用 / 停用，全部经 Claude Code 自己的 CLI
//! （`claude plugin list|enable|disable --json`）。
//!
//! 不直接改 `enabledPlugins`：启用要连带启用依赖、要过组织的插件策略，停用要先确认没有
//! 别的插件依赖它，这些只有 Claude Code 自己知道。安装、升级、市场管理也留给 `/plugin`。
//! 这里能开关的只有用户级（`user`）和账号同步（`synced`）的插件，项目级、托管的只列出。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;

const LIST_TIMEOUT: Duration = Duration::from_secs(30);
const TOGGLE_TIMEOUT: Duration = Duration::from_secs(60);

/// 一个已安装的 Claude Code 插件。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePlugin {
    /// `name@marketplace`（账号同步的是 `name@synced`）
    pub id: String,
    pub version: String,
    /// `user`、`project`、`local`、`managed`、`synced`、`session`
    pub scope: String,
    pub enabled: bool,
    /// CC Switch 能不能开关它：只有 `user` 和 `synced`
    pub toggleable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    /// 插件带的 Skills（`skills/<name>/SKILL.md`），以 `/插件名:技能名` 调用
    pub skills: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedPlugin {
    id: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    install_path: Option<String>,
    #[serde(default)]
    read_from_folder: Option<String>,
    #[serde(default)]
    project_path: Option<String>,
    #[serde(default)]
    errors: Vec<String>,
}

fn toggleable(scope: &str) -> bool {
    matches!(scope, "user" | "synced")
}

/// 插件 id 只认 `name@marketplace` 这类字符，免得被当成命令行选项。
fn validate_id(id: &str) -> Result<(), AppError> {
    let valid = !id.is_empty()
        && !id.starts_with('-')
        && id.len() <= 200
        && id.contains('@')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'));
    if valid {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!("不是有效的插件 id: {id}")))
    }
}

fn run_claude(args: &[&str], timeout: Duration) -> Result<std::process::Output, AppError> {
    let home = crate::config::get_home_dir();
    crate::commands::misc::run_detected_tool_command_with_timeout(
        "claude",
        args,
        Some(timeout),
        &[],
        &home,
    )
    .map_err(|err| {
        AppError::localized(
            "claude_plugins.cli_unavailable",
            format!("没能运行 Claude Code 的命令行（claude）：{err}"),
            format!("Could not run the Claude Code CLI (claude): {err}"),
        )
    })
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// 插件目录里带的 Skills：`skills/` 下每个含 `SKILL.md` 的子目录。
fn plugin_skills(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("skills")) else {
        return Vec::new();
    };
    let mut skills: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().join("SKILL.md").is_file())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    skills.sort();
    skills
}

pub(crate) fn parse_list(stdout: &str) -> Result<Vec<ClaudePlugin>, AppError> {
    // `--json` 把数组打在 stdout 上；前面偶尔有提示行，取第一个 `[` 起的部分。
    let start = stdout
        .find('[')
        .ok_or_else(|| AppError::Message(format!("claude plugin list 没有输出 JSON：{stdout}")))?;
    let listed: Vec<ListedPlugin> = serde_json::from_str(&stdout[start..])
        .map_err(|err| AppError::Message(format!("解析 claude plugin list 的输出失败：{err}")))?;
    let mut plugins: Vec<ClaudePlugin> = listed
        .into_iter()
        .map(|plugin| {
            let dir = plugin
                .read_from_folder
                .as_deref()
                .or(plugin.install_path.as_deref())
                .map(PathBuf::from);
            ClaudePlugin {
                toggleable: toggleable(&plugin.scope),
                skills: dir.as_deref().map(plugin_skills).unwrap_or_default(),
                id: plugin.id,
                version: plugin.version,
                scope: plugin.scope,
                enabled: plugin.enabled,
                project_path: plugin.project_path,
                errors: plugin.errors,
            }
        })
        .collect();
    plugins.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.scope.cmp(&b.scope)));
    Ok(plugins)
}

/// 已安装的插件（`claude plugin list --json`）。
pub fn list() -> Result<Vec<ClaudePlugin>, AppError> {
    let output = run_claude(&["plugin", "list", "--json"], LIST_TIMEOUT)?;
    let stdout = output_text(&output.stdout);
    if !output.status.success() {
        let stderr = output_text(&output.stderr);
        return Err(AppError::Message(format!(
            "claude plugin list 失败：{}",
            if stderr.is_empty() { stdout } else { stderr }
        )));
    }
    if stdout.is_empty() || stdout.starts_with("No plugins installed") {
        return Ok(Vec::new());
    }
    parse_list(&stdout)
}

/// `plugin enable|disable --json` 的结果：stdout 最后一行是 `{command, outcome, message,
/// failureCode?, alreadyInGoalState?}`。已经是目标状态也算成功；其余 `failed` 交出它的
/// `message`。没有这一行（老版本不认 `--json`）返回 `None`。
pub(crate) fn parse_toggle_result(stdout: &str) -> Option<Result<(), String>> {
    let line = stdout.lines().last()?.trim();
    let value: Value = serde_json::from_str(line).ok()?;
    let outcome = value.get("outcome").and_then(Value::as_str)?;
    if outcome == "ok" || value.get("alreadyInGoalState").and_then(Value::as_bool) == Some(true) {
        return Some(Ok(()));
    }
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("failureCode").and_then(Value::as_str))
        .unwrap_or(outcome)
        .to_string();
    Some(Err(message))
}

/// 启用或停用一个用户级 / 账号同步的插件。依赖、组织策略由 Claude Code 检查，拒绝时把
/// 它的原话交给界面。
pub fn set_enabled(id: &str, scope: &str, enabled: bool) -> Result<(), AppError> {
    validate_id(id)?;
    if !toggleable(scope) {
        return Err(AppError::InvalidInput(format!(
            "{id} 是 {scope} 级的插件，请在 Claude Code 里用 /plugin 管理"
        )));
    }
    let action = if enabled { "enable" } else { "disable" };
    let mut args = vec!["plugin", action, id];
    if scope == "user" {
        args.extend(["--scope", "user"]);
    }
    args.push("--json");
    let output = run_claude(&args, TOGGLE_TIMEOUT)?;
    let stdout = output_text(&output.stdout);
    match parse_toggle_result(&stdout) {
        Some(Ok(())) => Ok(()),
        Some(Err(message)) => Err(AppError::Message(message)),
        // 老版本不认 `--json`：只看退出码。
        None if output.status.success() => Ok(()),
        None => {
            let stderr = output_text(&output.stderr);
            Err(AppError::Message(if stderr.is_empty() {
                stdout
            } else {
                stderr
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_list_and_reads_bundled_skills() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("skills").join("review");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: review\n---\n").unwrap();
        std::fs::create_dir_all(dir.path().join("skills").join("notes")).unwrap();
        let stdout = format!(
            "[{{\"id\":\"fmt@acme\",\"version\":\"1.2.0\",\"scope\":\"user\",\"enabled\":true,\"installPath\":{}}},\
             {{\"id\":\"team@corp\",\"version\":\"unknown\",\"scope\":\"project\",\"enabled\":false,\"installPath\":\"/nope\",\"projectPath\":\"/repo\",\"errors\":[\"boom\"]}}]",
            serde_json::to_string(&dir.path().display().to_string()).unwrap()
        );
        let plugins = parse_list(&stdout).unwrap();
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].id, "fmt@acme");
        assert!(plugins[0].toggleable);
        assert_eq!(plugins[0].skills, vec!["review".to_string()]);
        assert_eq!(plugins[1].scope, "project");
        assert!(!plugins[1].toggleable);
        assert_eq!(plugins[1].project_path.as_deref(), Some("/repo"));
        assert_eq!(plugins[1].errors, vec!["boom".to_string()]);
    }

    #[test]
    fn toggle_results_accept_goal_state_and_surface_refusals() {
        assert_eq!(
            parse_toggle_result(
                "{\"command\":\"disable\",\"outcome\":\"ok\",\"message\":\"Successfully disabled plugin: fmt\"}"
            ),
            Some(Ok(()))
        );
        assert_eq!(
            parse_toggle_result(
                "{\"command\":\"enable\",\"outcome\":\"failed\",\"message\":\"Plugin \\\"fmt\\\" is already enabled\",\"failureCode\":\"already_in_goal_state\",\"alreadyInGoalState\":true}"
            ),
            Some(Ok(()))
        );
        assert_eq!(
            parse_toggle_result(
                "$ npm install\n{\"command\":\"disable\",\"outcome\":\"failed\",\"message\":\"lint is required by fmt\"}"
            ),
            Some(Err("lint is required by fmt".to_string()))
        );
        assert_eq!(
            parse_toggle_result("Successfully disabled plugin: fmt"),
            None
        );
    }

    #[test]
    fn rejects_ids_that_could_be_read_as_options() {
        assert!(validate_id("fmt@acme").is_ok());
        assert!(validate_id("--all").is_err());
        assert!(validate_id("fmt").is_err());
        assert!(validate_id("a b@c").is_err());
        assert!(set_enabled("team@corp", "project", false).is_err());
    }
}
