import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { Loader2, RefreshCw } from "lucide-react";
import { toast } from "@/lib/toast";
import { Button } from "@/components/ui/button";
import { DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { V7Dialog } from "@/components/mcp/formBits";
import { skillsApi, type ClaudePlugin } from "@/lib/api/skills";
import { extractErrorMessage } from "@/utils/errorUtils";
import { cn } from "@/lib/utils";

interface ClaudePluginsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const KNOWN_SCOPES = [
  "user",
  "project",
  "local",
  "managed",
  "synced",
  "session",
] as const;

/**
 * Skills 页 ⋯ →「Claude Code 插件」：列出已安装的插件和它们带的 Skills。
 * 列表和开关都经 Claude Code 自己的 CLI；只有个人（user）和账号同步的插件能在这里开关，
 * 其余的只列出。安装、升级、市场留给 Claude Code 的 /plugin。
 */
export function ClaudePluginsDialog({
  open,
  onOpenChange,
}: ClaudePluginsDialogProps) {
  const { t } = useTranslation();
  const [busyId, setBusyId] = useState<string | null>(null);
  const {
    data: plugins,
    error,
    isFetching,
    refetch,
  } = useQuery({
    queryKey: ["claudePlugins"],
    queryFn: () => skillsApi.listClaudePlugins(),
    enabled: open,
    staleTime: 0,
  });

  const scopeLabel = (scope: string) =>
    (KNOWN_SCOPES as readonly string[]).includes(scope)
      ? t(`skillsPage.plugins.scope.${scope}`)
      : scope;

  const toggle = async (plugin: ClaudePlugin) => {
    setBusyId(plugin.id);
    try {
      await skillsApi.setClaudePluginEnabled(
        plugin.id,
        plugin.scope,
        !plugin.enabled,
      );
      await refetch();
    } catch (err) {
      toast.error(t("common.error"), {
        description: extractErrorMessage(err) || String(err),
      });
    } finally {
      setBusyId(null);
    }
  };

  const key = (plugin: ClaudePlugin) =>
    `${plugin.id}\u0000${plugin.scope}\u0000${plugin.projectPath ?? ""}`;

  return (
    <V7Dialog open={open} onOpenChange={onOpenChange} width={560}>
      <div className="flex shrink-0 items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <DialogTitle className="text-section">
            {t("skillsPage.plugins.title")}
          </DialogTitle>
          <DialogDescription className="text-caption text-fg-2">
            {t("skillsPage.plugins.lead")}
          </DialogDescription>
        </div>
        <Button
          type="button"
          variant="quiet"
          size="icon-compact"
          aria-label={t("skillsPage.plugins.refresh")}
          disabled={isFetching}
          onClick={() => void refetch()}
        >
          <RefreshCw
            className={cn("h-4 w-4", isFetching && "animate-spin")}
            strokeWidth={1.5}
          />
        </Button>
      </div>

      {error ? (
        <p role="alert" className="m-0 text-body text-warning-text">
          {extractErrorMessage(error) || String(error)}
        </p>
      ) : !plugins ? (
        <div className="flex justify-center py-6">
          <Loader2 className="h-5 w-5 animate-spin text-fg-3" />
        </div>
      ) : plugins.length === 0 ? (
        <p className="m-0 text-body text-fg-2">
          {t("skillsPage.plugins.empty")}
        </p>
      ) : (
        <ul className="m-0 max-h-[360px] min-h-0 list-none overflow-y-auto rounded-panel border border-border p-0">
          {plugins.map((plugin, index) => {
            const id = `claude-plugin-${index}`;
            return (
              <li
                key={key(plugin)}
                className={cn(
                  "flex items-start gap-2.5 py-2 pe-3 ps-3.5",
                  index > 0 && "border-t border-border",
                )}
              >
                <span className="flex h-5 w-4 shrink-0 items-center">
                  {busyId === plugin.id ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-fg-3" />
                  ) : (
                    <input
                      id={id}
                      type="checkbox"
                      className="ui-checkbox"
                      checked={plugin.enabled}
                      disabled={!plugin.toggleable || busyId !== null}
                      onChange={() => void toggle(plugin)}
                    />
                  )}
                </span>
                <label htmlFor={id} className="flex min-w-0 flex-1 flex-col">
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="truncate text-body font-medium">
                      {plugin.id}
                    </span>
                    <span className="shrink-0 font-mono text-caption text-fg-3">
                      {plugin.version}
                    </span>
                    <span className="shrink-0 rounded-full bg-subtle px-1.5 text-badge text-fg-2">
                      {scopeLabel(plugin.scope)}
                    </span>
                  </span>
                  {plugin.skills.length > 0 && (
                    <span className="truncate text-caption text-fg-2">
                      {t("skillsPage.plugins.skills", {
                        skills: plugin.skills.join(t("mcpPage.listSeparator")),
                      })}
                    </span>
                  )}
                  {!plugin.toggleable && (
                    <span className="truncate text-caption text-fg-3">
                      {plugin.projectPath
                        ? t("skillsPage.plugins.readOnlyProject", {
                            path: plugin.projectPath,
                          })
                        : t("skillsPage.plugins.readOnly")}
                    </span>
                  )}
                  {plugin.errors.map((message) => (
                    <span
                      key={message}
                      className="text-caption text-warning-text"
                    >
                      {message}
                    </span>
                  ))}
                </label>
              </li>
            );
          })}
        </ul>
      )}
    </V7Dialog>
  );
}
