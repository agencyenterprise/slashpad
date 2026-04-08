import { invoke } from "@tauri-apps/api/core";
import { readDir, readTextFile, writeTextFile, remove } from "@tauri-apps/plugin-fs";
import * as yaml from "js-yaml";
import type { Skill } from "./types";

const EXAMPLE_SKILLS: Skill[] = [
  {
    name: "Summarize Emails",
    trigger: "/emails",
    description: "Summarize unread emails from today",
    prompt: `Fetch my unread emails from today using Gmail.
Summarize each in 1-2 sentences.
Group by: Needs Response, FYI, Can Ignore.
Be concise.`,
    tools: ["composio:gmail"],
  },
  {
    name: "Git Standup",
    trigger: "/standup",
    description: "Generate standup from git activity",
    prompt: `Look at my git commits from the last 24 hours across all repos.
Write a standup update with: Yesterday, Today (inferred from context), Blockers.
Keep it under 100 words.`,
    tools: ["composio:github"],
  },
  {
    name: "PR Review",
    trigger: "/prs",
    description: "Check open PRs needing review",
    prompt: `Find all open pull requests assigned to me or where I'm requested as a reviewer.
For each, show: repo, title, author, how old it is, and a 1-line summary of the changes.
Sort by urgency.`,
    tools: ["composio:github"],
  },
];

export async function getSkillsDir(): Promise<string> {
  return await invoke<string>("get_skills_dir");
}

export async function loadSkills(): Promise<Skill[]> {
  try {
    const dir = await getSkillsDir();
    const entries = await readDir(dir);

    const skills: Skill[] = [];
    for (const entry of entries) {
      if (entry.name?.endsWith(".yaml") || entry.name?.endsWith(".yml")) {
        try {
          const content = await readTextFile(`${dir}/${entry.name}`);
          const skill = yaml.load(content) as Skill;
          if (skill && skill.trigger && skill.prompt) {
            skills.push(skill);
          }
        } catch (e) {
          console.warn(`Failed to load skill ${entry.name}:`, e);
        }
      }
    }

    return skills;
  } catch {
    // If directory doesn't exist or is empty, seed with examples
    console.log("No skills found, seeding examples...");
    for (const skill of EXAMPLE_SKILLS) {
      await saveSkill(skill);
    }
    return EXAMPLE_SKILLS;
  }
}

export async function saveSkill(skill: Skill): Promise<void> {
  const dir = await getSkillsDir();
  const filename = skill.trigger.replace("/", "") + ".yaml";
  const content = yaml.dump(skill, { lineWidth: 100, noRefs: true });
  await writeTextFile(`${dir}/${filename}`, content);
}

export async function deleteSkill(skill: Skill): Promise<void> {
  const dir = await getSkillsDir();
  const filename = skill.trigger.replace("/", "") + ".yaml";
  await remove(`${dir}/${filename}`);
}

/**
 * Generate a skill definition from a session transcript.
 * This is called when the user clicks "Save as Skill" after an ad-hoc session.
 */
export function buildSkillFromSession(
  trigger: string,
  prompt: string,
  toolsUsed: string[]
): Skill {
  const name = trigger
    .replace("/", "")
    .split("-")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");

  return {
    name,
    trigger: trigger.startsWith("/") ? trigger : `/${trigger}`,
    description: prompt.slice(0, 80) + (prompt.length > 80 ? "..." : ""),
    prompt,
    tools: toolsUsed,
    createdAt: new Date().toISOString(),
  };
}
