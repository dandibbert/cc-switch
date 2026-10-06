import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ClaudePluginsDialog } from "@/components/skills/ClaudePluginsDialog";
import type { ClaudePlugin } from "@/lib/api/skills";

const m = vi.hoisted(() => ({
  list: vi.fn(),
  setEnabled: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock("@/lib/api/skills", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api/skills")>();
  return {
    ...actual,
    skillsApi: {
      ...actual.skillsApi,
      listClaudePlugins: m.list,
      setClaudePluginEnabled: m.setEnabled,
    },
  };
});

vi.mock("@/lib/toast", () => ({ toast: { error: m.toastError } }));

const plugin = (rest: Partial<ClaudePlugin>): ClaudePlugin => ({
  id: "fmt@acme",
  version: "1.0.0",
  scope: "user",
  enabled: true,
  toggleable: true,
  skills: [],
  errors: [],
  ...rest,
});

function renderDialog() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <ClaudePluginsDialog open onOpenChange={() => {}} />
    </QueryClientProvider>,
  );
}

describe("ClaudePluginsDialog", () => {
  beforeEach(() => {
    m.list.mockReset();
    m.setEnabled.mockReset().mockResolvedValue(undefined);
  });

  it("lists plugins, switches personal ones and leaves project ones read-only", async () => {
    m.list.mockResolvedValue([
      plugin({ skills: ["review", "lint"] }),
      plugin({
        id: "team@corp",
        scope: "project",
        toggleable: false,
        projectPath: "/repo",
      }),
    ]);
    renderDialog();
    expect(await screen.findByText("fmt@acme")).toBeInTheDocument();
    expect(screen.getByText("skillsPage.plugins.skills")).toBeInTheDocument();
    const [personal, project] = screen.getAllByRole("checkbox");
    expect(project).toBeDisabled();
    expect(
      screen.getByText("skillsPage.plugins.readOnlyProject"),
    ).toBeInTheDocument();

    await userEvent.click(personal);
    await waitFor(() =>
      expect(m.setEnabled).toHaveBeenCalledWith("fmt@acme", "user", false),
    );
    await waitFor(() => expect(m.list).toHaveBeenCalledTimes(2));
  });

  it("shows Claude Code's own refusal", async () => {
    m.list.mockResolvedValue([plugin({})]);
    m.setEnabled.mockRejectedValue("lint is required by fmt");
    renderDialog();
    await userEvent.click(await screen.findByRole("checkbox"));
    await waitFor(() =>
      expect(m.toastError).toHaveBeenCalledWith("common.error", {
        description: "lint is required by fmt",
      }),
    );
  });

  it("explains when the CLI can't be run", async () => {
    m.list.mockRejectedValue("claude is not installed");
    renderDialog();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "claude is not installed",
    );
  });
});
