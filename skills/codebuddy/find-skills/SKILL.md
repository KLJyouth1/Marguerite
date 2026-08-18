---
name: find-skills
description: "Helps users discover and install agent skills when they ask questions like \"how do I do X\", \"find a skill for X\", \"is there a skill that can...\", or express interest in extending capabilities. This skill should be used when the user is looking for functionality that might exist as an installable skill."
description_zh: "发现和安装新技能，扩展智能体能力"
description_en: "Discover and install new skills"
version: 0.1.0
---

# Find Skills

This skill helps you discover and install skills from the open agent skills ecosystem.

## When to Use This Skill

Use this skill when the user:

- Asks "how do I do X" where X might be a common task with an existing skill
- Says "find a skill for X" or "is there a skill for X"
- Asks "can you do X" where X is a specialized capability
- Expresses interest in extending agent capabilities
- Wants to search for tools, templates, or workflows
- Mentions they wish they had help with a specific domain (design, testing, deployment, etc.)

## What is the Skills CLI?

The Skills CLI (`npx skills`) is the package manager for the open agent skills ecosystem. Skills are modular packages that extend agent capabilities with specialized knowledge, workflows, and tools.

**Key commands:**

- `npx skills find [query]` - Search for skills interactively or by keyword
- `npx skills add <package>` - Install a skill from GitHub or other sources
- `npx skills check` - Check for skill updates
- `npx skills update` - Update all installed skills

**Browse skills at:** https://skills.sh/

## How to Help Users Find Skills

### Step 1: Understand What They Need

When a user asks for help with something, identify:

1. The domain (e.g., React, testing, design, deployment)
2. The specific task (e.g., writing tests, creating animations, reviewing PRs)
3. Whether this is a common enough task that a skill likely exists

### Step 2: Search for Skills

Run the find command with a relevant query:

```bash
npx skills find [query]
```

For example:

- User asks "how do I make my React app faster?" → `npx skills find react performance`
- User asks "can you help me with PR reviews?" → `npx skills find pr review`
- User asks "I need to create a changelog" → `npx skills find changelog`

The command will return results like:

```
Install with npx skills add <owner/repo@skill>

vercel-labs/agent-skills@vercel-react-best-practices
└ https://skills.sh/vercel-labs/agent-skills/vercel-react-best-practices
```

If no relevant results are found, try ClawHub as a fallback (see "ClawHub Fallback" section below).

### Step 3: Present Options to the User

When you find relevant skills, present them to the user with:

1. The skill name and what it does
2. The install command they can run
3. A link to learn more at skills.sh

Example response:

```
I found a skill that might help! The "vercel-react-best-practices" skill provides
React and Next.js performance optimization guidelines from Vercel Engineering.

To install it:
npx skills add vercel-labs/agent-skills@vercel-react-best-practices

Learn more: https://skills.sh/vercel-labs/agent-skills/vercel-react-best-practices
```

### Step 4: Detect Current Client

Before installing, detect which client is running by checking the `__CFBundleIdentifier` environment variable:

```bash
echo $__CFBundleIdentifier
```

Determine the target skills directory based on the result:

| `__CFBundleIdentifier` contains | Client    | Target skills dir        |
| ------------------------------- | --------- | ------------------------ |
| `codebuddy`                     | CodeBuddy | `~/.codebuddy/skills/`   |
| anything else / empty / unset   | WorkBuddy | `~/.workbuddy/skills/`   |

**Default to WorkBuddy**: If the variable is empty, unset, or contains any value other than `codebuddy`, treat the current client as WorkBuddy.

### Step 5: Check the Local WorkBuddy Skills Marketplace First (Required)

Before installing from the network (`npx skills add`, ClawHub, etc.), you **must** look in WorkBuddy’s official on-disk skills marketplace:

**Local marketplace folder:** `~/.workbuddy/skills-marketplace/skills`

1. List or inspect that directory and identify a skill subfolder that matches the user’s need (by folder name, `SKILL.md`, or metadata).
2. If a match exists, **copy** the entire skill folder into the **target skills directory** from Step 4 (do not remove the original marketplace copy).

```bash
ls ~/.workbuddy/skills-marketplace/skills
# WorkBuddy target:
cp -r ~/.workbuddy/skills-marketplace/skills/<skill-folder-name> ~/.workbuddy/skills/<skill-folder-name>
# CodeBuddy target:
cp -r ~/.workbuddy/skills-marketplace/skills/<skill-folder-name> ~/.codebuddy/skills/<skill-folder-name>
```

3. If no suitable skill exists locally, continue with Step 6 (remote install).

Always confirm the copied skill is present under the target directory before reporting success. If the user already has that skill name in the target dir, resolve the conflict (skip, replace, or rename) explicitly.

### Step 6: Install the Skill (Remote)

If Step 5 did not yield a local match and the user still wants to proceed, install from the registry:

```bash
npx skills add <owner/repo@skill> -g -y
```

The `-g` flag installs globally (user-level) and `-y` skips confirmation prompts.

### Step 7: Ensure Skill Is in the Correct Directory

After a remote install (Step 6), if the current client is WorkBuddy (detected in Step 4), verify the skill exists at `~/.workbuddy/skills/<skill-name>`. The CLI typically installs to `~/.agents/skills/<skill-name>/` and may not automatically link to WorkBuddy’s directory.

```bash
# Check if skill already exists in WorkBuddy's directory
ls -la ~/.workbuddy/skills/<skill-name>
```

If the skill is missing from `~/.workbuddy/skills/`, check `~/.agents/skills/<skill-name>/` and create a symlink:

```bash
# If installed at ~/.agents/skills/<skill-name>, create symlink
ln -s ../../.agents/skills/<skill-name> ~/.workbuddy/skills/<skill-name>
```

If the skill is not in `~/.agents/skills/` either, find where it was actually installed (e.g., `~/.codebuddy/skills/<skill-name>`) and copy it:

```bash
cp -r <installed-path> ~/.workbuddy/skills/<skill-name>
```

Always confirm the skill is accessible at the target directory before reporting success to the user.

## ClawHub Fallback

If `npx skills find` returns no relevant results, try ClawHub (the OpenClaw skill registry) as a secondary source:

```bash
npx clawhub search [query]
```

If a match is found, install it directly to the target directory (detected in Step 4):

```bash
# For WorkBuddy (default):
npx clawhub install <slug> --workdir ~ --dir .workbuddy/skills

# For CodeBuddy:
npx clawhub install <slug> --workdir ~ --dir .codebuddy/skills
```

Before `npx clawhub install`, complete Step 5 (local marketplace check). After installation, verify the skill is in the target directory (same as Step 7).

**Browse ClawHub skills at:** https://clawhub.com/

## Common Skill Categories

When searching, consider these common categories:

| Category        | Example Queries                          |
| --------------- | ---------------------------------------- |
| Web Development | react, nextjs, typescript, css, tailwind |
| Testing         | testing, jest, playwright, e2e           |
| DevOps          | deploy, docker, kubernetes, ci-cd        |
| Documentation   | docs, readme, changelog, api-docs        |
| Code Quality    | review, lint, refactor, best-practices   |
| Design          | ui, ux, design-system, accessibility     |
| Productivity    | workflow, automation, git                |

## Tips for Effective Searches

1. **Use specific keywords**: "react testing" is better than just "testing"
2. **Try alternative terms**: If "deploy" doesn't work, try "deployment" or "ci-cd"
3. **Check popular sources**: Many skills come from `vercel-labs/agent-skills` or `ComposioHQ/awesome-claude-skills`

## When No Skills Are Found

If neither Vercel Skills nor ClawHub has relevant results:

1. Acknowledge that no existing skill was found in either registry
2. Offer to help with the task directly using your general capabilities
3. Suggest the user could create their own skill with `npx skills init`

Example:

```
I searched for skills related to "xyz" in both Vercel Skills and ClawHub
but didn't find any matches.
I can still help you with this task directly! Would you like me to proceed?

If this is something you do often, you could create your own skill:
npx skills init my-xyz-skill
```
