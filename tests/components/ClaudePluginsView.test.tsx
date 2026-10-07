import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ClaudePluginsView } from "@/components/skills/ClaudePluginsView";
import type { ClaudePlugin } from "@/lib/api/skills";

const m = vi.hoisted(() => ({
  list: vi.fn(),
  setEnabled: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
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

vi.mock("@/lib/toast", () => ({
  toast: { error: m.toastError, success: m.toastSuccess },
}));

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

function renderView() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <ClaudePluginsView renderViewTabs={(trailing) => trailing} />
    </QueryClientProvider>,
  );
}

const cells = () =>
  screen.getAllByRole("button", { name: /skillsPage.plugins.cell/ });

describe("ClaudePluginsView", () => {
  beforeEach(() => {
    m.list.mockReset();
    m.setEnabled.mockReset().mockResolvedValue(undefined);
  });

  it("switches personal plugins and lists project ones read-only", async () => {
    m.list.mockResolvedValue([
      plugin({ skills: ["review", "lint"] }),
      plugin({
        id: "team@corp",
        scope: "project",
        toggleable: false,
        projectPath: "/repo",
      }),
    ]);
    renderView();
    expect(await screen.findByText("fmt")).toBeInTheDocument();
    expect(screen.getByText("@acme")).toBeInTheDocument();
    expect(screen.getByText("skillsPage.plugins.skills")).toBeInTheDocument();
    // 只读那一行没有开关
    expect(cells()).toHaveLength(1);
    expect(
      screen.getByLabelText("skillsPage.plugins.readOnlyOn"),
    ).toBeInTheDocument();

    await userEvent.click(cells()[0]);
    await waitFor(() =>
      expect(m.setEnabled).toHaveBeenCalledWith("fmt@acme", "user", false),
    );
    await waitFor(() => expect(m.list).toHaveBeenCalledTimes(2));
    expect(m.toastSuccess).toHaveBeenCalledWith(
      "skillsPage.plugins.toastDisabled",
      { closeButton: true },
    );
  });

  it("shows Claude Code's own refusal", async () => {
    m.list.mockResolvedValue([plugin({})]);
    m.setEnabled.mockRejectedValue("lint is required by fmt");
    renderView();
    await screen.findByText("fmt");
    await userEvent.click(cells()[0]);
    await waitFor(() =>
      expect(m.toastError).toHaveBeenCalledWith("common.error", {
        description: "lint is required by fmt",
      }),
    );
  });

  it("explains when the CLI can't be run and offers a retry", async () => {
    m.list.mockRejectedValue("claude is not installed");
    renderView();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "claude is not installed",
    );
    m.list.mockResolvedValue([]);
    await userEvent.click(screen.getByRole("button", { name: "common.retry" }));
    expect(
      await screen.findByText("skillsPage.plugins.emptyTitle"),
    ).toBeInTheDocument();
  });
});
