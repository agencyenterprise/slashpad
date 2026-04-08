import { invoke } from "@tauri-apps/api/core";
import { readDir, readTextFile, remove } from "@tauri-apps/plugin-fs";
import * as yaml from "js-yaml";
import type { Skill } from "./types";

async function getLaunchpadDir(): Promise<string> {
  return await invoke<string>("get_launchpad_dir");
}

function parseFrontmatter(content: string): Record<string, unknown> | null {
  const match = content.match(/^---\n([\s\S]*?)\n---/);
  if (!match) return null;
  try {
    return yaml.load(match[1]) as Record<string, unknown>;
  } catch {
    return null;
  }
}

export async function loadSkills(): Promise<Skill[]> {
  try {
    const launchpadDir = await getLaunchpadDir();
    const skillsDir = `${launchpadDir}/.claude/skills`;
    const entries = await readDir(skillsDir);

    const skills: Skill[] = [];
    for (const entry of entries) {
      if (!entry.isDirectory) continue;
      const skillPath = `${skillsDir}/${entry.name}/SKILL.md`;
      try {
        const content = await readTextFile(skillPath);
        const fm = parseFrontmatter(content);
        if (!fm || !fm.name || !fm.description) continue;
        if (fm["user-invocable"] === false) continue;

        skills.push({
          name: fm.name as string,
          description: fm.description as string,
          path: skillPath,
          args: Array.isArray(fm.args) ? fm.args : undefined,
        });
      } catch {
        // Skip unreadable skill directories
      }
    }

    return skills;
  } catch {
    return [];
  }
}

export async function deleteSkill(skill: Skill): Promise<void> {
  // Remove the skill directory (parent of SKILL.md)
  const skillDir = skill.path.replace(/\/SKILL\.md$/, "");
  await remove(skillDir, { recursive: true });
}
