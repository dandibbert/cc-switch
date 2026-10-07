import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { Loader2, RefreshCw } from "lucide-react";
import { toast } from "@/lib/toast";
import { Button } from "@/components/ui/button";
import { HoverTip } from "@/components/ui/hover-tip";
import { AppGlyph } from "@/components/shell/AppGlyph";
import { MatrixCell } from "@/components/mcp/AppMatrix";
import { skillsApi, type ClaudePlugin } from "@/lib/api/skills";
import { extractErrorMessage } from "@/utils/errorUtils";
import { cn } from "@/lib/utils";

const KNOWN_SCOPES = [
  "user",
  "project",
  "local",
  "managed",
  "synced",
  "session",
] as const;

interface ClaudePluginsViewProps {
  /** 页签行右侧的槽（和「已安装」「发现」共用一行页签） */
  renderViewTabs: (trailing: React.ReactNode) => React.ReactNode;
}

const splitId = (id: string) => {
  const at = id.lastIndexOf("@");
  return at > 0
    ? { name: id.slice(0, at), marketplace: id.slice(at + 1) }
    : { name: id, marketplace: "" };
};

/**
 * Skills 页「Claude Code 插件」页签：和已安装 Skill 同一套表格，一行一个插件，
 * 右侧 Claude Code 一列勾选 = 在 Claude Code 里启用。
 *
 * 列表和开关都经 Claude Code 自己的 CLI；只有个人（user）和账号同步的插件能在这里开关，
 * 项目级、托管的只列出。安装、升级、市场留给 Claude Code 的 /plugin。
 */
export function ClaudePluginsView({ renderViewTabs }: ClaudePluginsViewProps) {
  const { t } = useTranslation();
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const {
    data: plugins,
    error,
    isLoading,
    isFetching,
    refetch,
  } = useQuery({
    queryKey: ["claudePlugins"],
    queryFn: () => skillsApi.listClaudePlugins(),
    staleTime: 30_000,
  });

  const scopeLabel = (scope: string) =>
    (KNOWN_SCOPES as readonly string[]).includes(scope)
      ? t(`skillsPage.plugins.scope.${scope}`)
      : scope;
  const key = (plugin: ClaudePlugin) =>
    `${plugin.id}\u0000${plugin.scope}\u0000${plugin.projectPath ?? ""}`;

  const toggle = async (plugin: ClaudePlugin) => {
    if (busyKey) return;
    setBusyKey(key(plugin));
    try {
      await skillsApi.setClaudePluginEnabled(
        plugin.id,
        plugin.scope,
        !plugin.enabled,
      );
      await refetch();
      toast.success(
        t(
          plugin.enabled
            ? "skillsPage.plugins.toastDisabled"
            : "skillsPage.plugins.toastEnabled",
          { name: plugin.id },
        ),
        { closeButton: true },
      );
    } catch (err) {
      toast.error(t("common.error"), {
        description: extractErrorMessage(err) || String(err),
      });
    } finally {
      setBusyKey(null);
    }
  };

  const trailing = (
    <HoverTip content={t("skillsPage.plugins.refresh")}>
      <Button
        type="button"
        variant="quiet"
        size="icon-compact"
        className="h-8 w-8"
        aria-label={t("skillsPage.plugins.refresh")}
        disabled={isFetching}
        onClick={() => void refetch()}
      >
        <RefreshCw
          className={cn("h-4 w-4", isFetching && "animate-spin")}
          strokeWidth={1.5}
        />
      </Button>
    </HoverTip>
  );

  const body = () => {
    if (isLoading) {
      return (
        <div className="py-12 text-center text-body text-fg-2">
          {t("skillsPage.plugins.loading")}
        </div>
      );
    }
    if (error) {
      return (
        <div className="flex flex-1 flex-col items-center justify-center gap-2.5 px-10 pb-12 text-center">
          <h2 className="m-0 text-section">
            {t("skillsPage.plugins.loadFailed")}
          </h2>
          <code
            role="alert"
            className="max-w-[520px] rounded-[8px] bg-subtle px-3 py-2 text-left font-mono text-caption text-fg-2 [overflow-wrap:anywhere]"
          >
            {extractErrorMessage(error) || String(error)}
          </code>
          <Button
            type="button"
            variant="neutral"
            size="regular"
            className="mt-1"
            onClick={() => void refetch()}
          >
            {t("common.retry")}
          </Button>
        </div>
      );
    }
    if (!plugins || plugins.length === 0) {
      return (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 px-10 pb-12 text-center">
          <h2 className="m-0 text-section">
            {t("skillsPage.plugins.emptyTitle")}
          </h2>
          <p className="m-0 text-body text-fg-2">
            {t("skillsPage.plugins.empty")}
          </p>
        </div>
      );
    }
    return (
      <div className="min-h-0 overflow-auto scroll-stable rounded-panel border border-border bg-surface">
        <div className="min-w-[560px]">
          <div className="sticky top-0 z-10 flex h-11 items-center border-b border-border bg-subtle px-2">
            <span className="flex-1 ps-2.5 text-body font-medium">
              {t("skillsPage.plugins.count", { count: plugins.length })}
            </span>
            <span className="flex w-9 shrink-0 justify-center">
              <HoverTip content="Claude Code" side="top">
                <span className="flex h-7 w-7 items-center justify-center">
                  <AppGlyph app="claude" size={16} />
                </span>
              </HoverTip>
            </span>
            <span className="w-2 shrink-0" />
          </div>
          <ul
            aria-label={t("skillsPage.plugins.listLabel")}
            className="m-0 list-none p-0"
          >
            {plugins.map((plugin, index) => {
              const { name, marketplace } = splitId(plugin.id);
              const busy = busyKey === key(plugin);
              const state = plugin.enabled ? "on" : "off";
              return (
                <li
                  key={key(plugin)}
                  className={cn(
                    "flex min-h-14 items-center px-2 py-2",
                    index > 0 && "border-t border-border",
                  )}
                >
                  <div className="flex min-w-0 flex-1 flex-col ps-2.5 pe-3">
                    <span className="flex min-w-0 items-center gap-1.5">
                      <span className="min-w-0 truncate text-body font-medium">
                        {name}
                      </span>
                      {marketplace && (
                        <span className="shrink-0 truncate font-mono text-caption text-fg-3">
                          @{marketplace}
                        </span>
                      )}
                      <span className="shrink-0 font-mono text-caption text-fg-3">
                        {plugin.version}
                      </span>
                      <span className="inline-flex h-[18px] shrink-0 items-center rounded-full bg-subtle px-1.5 text-badge font-medium text-fg-2">
                        {scopeLabel(plugin.scope)}
                      </span>
                    </span>
                    <span className="truncate text-caption text-fg-2">
                      {plugin.skills.length > 0
                        ? t("skillsPage.plugins.skills", {
                            skills: plugin.skills.join(
                              t("mcpPage.listSeparator"),
                            ),
                          })
                        : t("skillsPage.plugins.noSkills")}
                    </span>
                    {plugin.errors.map((message) => (
                      <span
                        key={message}
                        className="text-caption text-warning-text [overflow-wrap:anywhere]"
                      >
                        {message}
                      </span>
                    ))}
                  </div>
                  <span className="flex w-9 shrink-0 justify-center">
                    {busy ? (
                      <span className="flex h-7 w-7 items-center justify-center">
                        <Loader2 className="h-4 w-4 animate-spin text-fg-3" />
                      </span>
                    ) : plugin.toggleable ? (
                      <MatrixCell
                        app="claude"
                        state={state}
                        disabled={busyKey !== null}
                        label={t(`skillsPage.plugins.cell.${state}`, {
                          name: plugin.id,
                        })}
                        onClick={() => void toggle(plugin)}
                      />
                    ) : (
                      <HoverTip
                        disableHoverableContent
                        content={
                          plugin.projectPath
                            ? t("skillsPage.plugins.readOnlyProject", {
                                path: plugin.projectPath,
                              })
                            : t("skillsPage.plugins.readOnly")
                        }
                      >
                        <span
                          tabIndex={0}
                          aria-label={t(
                            plugin.enabled
                              ? "skillsPage.plugins.readOnlyOn"
                              : "skillsPage.plugins.readOnlyOff",
                          )}
                          className="flex h-7 w-7 items-center justify-center rounded-control text-caption text-fg-3"
                        >
                          {plugin.enabled ? "✓" : "–"}
                        </span>
                      </HoverTip>
                    )}
                  </span>
                  <span className="w-2 shrink-0" />
                </li>
              );
            })}
          </ul>
        </div>
      </div>
    );
  };

  return (
    <>
      {renderViewTabs(trailing)}
      <div className="flex min-h-0 flex-1 flex-col px-6 pb-5 pt-3">
        <p className="m-0 mb-3 text-caption text-fg-2">
          {t("skillsPage.plugins.lead")}
        </p>
        {body()}
      </div>
    </>
  );
}
