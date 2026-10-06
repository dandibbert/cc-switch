import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { SkillImportDialog } from "@/components/skills/SkillImportDialog";
import type { UnmanagedSkill } from "@/lib/api/skills";

const skills: UnmanagedSkill[] = [
  {
    directory: "plain",
    name: "plain",
    foundIn: ["claude"],
    path: "/home/u/.claude/skills/plain",
  },
  {
    directory: "forked",
    name: "forked",
    foundIn: ["claude"],
    path: "/home/u/.claude/skills/forked",
    conflict: true,
  },
  {
    directory: "forked",
    name: "forked",
    foundIn: ["agents"],
    path: "/home/u/.agents/skills/forked",
    conflict: true,
  },
];

function renderDialog(onImport = vi.fn()) {
  render(
    <SkillImportDialog
      skills={skills}
      visibleAppIds={["claude", "codex"]}
      isImporting={false}
      onImport={onImport}
      onClose={() => {}}
    />,
  );
  return onImport;
}

describe("SkillImportDialog", () => {
  it("leaves conflicting versions unticked and imports by source path", async () => {
    const onImport = renderDialog();
    const boxes = screen.getAllByRole("checkbox").slice(0, 3);
    expect(boxes.map((box) => (box as HTMLInputElement).checked)).toEqual([
      true,
      false,
      false,
    ]);
    expect(screen.getAllByText("skillsPage.import.conflict")).toHaveLength(2);

    await userEvent.click(screen.getByText("skillsPage.import.submit"));
    expect(onImport).toHaveBeenCalledWith([
      expect.objectContaining({
        directory: "plain",
        sourcePath: "/home/u/.claude/skills/plain",
      }),
    ]);
  });

  it("keeps only one version of a directory selected", async () => {
    const onImport = renderDialog();
    const [, claudeForked, agentsForked] = screen.getAllByRole("checkbox");
    await userEvent.click(claudeForked);
    await userEvent.click(agentsForked);
    expect((claudeForked as HTMLInputElement).checked).toBe(false);
    expect((agentsForked as HTMLInputElement).checked).toBe(true);

    await userEvent.click(screen.getByText("skillsPage.import.submit"));
    const sent = onImport.mock.calls[0][0] as Array<{ sourcePath: string }>;
    expect(sent.map((item) => item.sourcePath)).toEqual([
      "/home/u/.claude/skills/plain",
      "/home/u/.agents/skills/forked",
    ]);
  });
});
