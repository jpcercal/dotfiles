---
name: builder
description: Execute the implementation plan from conversation context autonomously with atomic commits and PR creation. Use ONLY when the user invokes bare @builder with the plan already in the immediate previous message, or to build a branch for a PR end-to-end.
---

# Skill: Autonomous Execution & PR Engine

## Objective
Execute the plan found in the conversation context strictly, commit changes atomically with human-like progression, and create a Pull Request.

## Plan Source
- This skill is invoked as bare `@builder` with NO arguments.
- The plan is the implementation plan in the immediate previous message/iteration already present in the context window. Use that as the sole source of truth.
- Do NOT expect `$ARGUMENTS` or `@builder + <plan>`. Do NOT ask the user to re-paste the plan if one is already in context.
- If no plan is found in context, STOP and ask the user for the plan. Do NOT invent one.

## Strict Rules
1. **Follow the plan from context exactly.** Do not add unrequested features or refactor unrelated code.
2. **No conversational filler.** Do not say "I will do this" or "I have done that". Execute tools directly and sequentially.
3. **Act, don't talk.** You MUST actually run the `git` and `gh` CLI commands. Do not simulate or hallucinate their outputs.
4. **Standard Branch Prefixes:** The branch name MUST start with a standard branch type. Allowed branch types:
   - `feat/` : New features or enhancements
   - `fix/` : Bug fixes
   - `hotfix/` : Urgent fixes for production issues
   - `release/` : Release preparation
   - `docs/` : Documentation changes
   - `chore/` : Maintenance, tooling, and dependency updates
   - `refactor/` : Code restructuring without behavior change

## Execution Steps

### Step 1: Branch Creation
- Analyze the plan from context to determine the correct branch `<type>` (e.g., `feat` for new features, `fix` for bugs, `chore` for maintenance).
- Execute: `git checkout -b <type>/<kebab-case-plan-name>`

### Step 2: Implement, Test & Commit Atomically
Do not make a single massive commit. Break the plan into logical, human-like development phases. For each phase:
1. **Implement:** Write/modify the files exactly as planned.
2. **Test:** Run the project's test suite. If tests are broken, fix the tests and related code until they pass before proceeding.
3. **Stage:** Execute `git add <specific-files-changed>`
4. **Commit:** Execute `git commit -m "<type>: <imperative subject>"`
   - *Allowed Commit Types:* `feat`, `fix`, `docs`, `chore`, `refactor`
   - *Example progression:*
     - `chore: scaffold project structure`
     - `feat: implement core data models`
     - `feat: add user authentication endpoint`
     - `refactor: optimize auth middleware`

### Step 3: Push & Create PR
1. Execute: `git push -u origin <type>/<kebab-case-plan-name>`
2. Execute the following command exactly as structured to create the PR:
   ```bash
   gh pr create --title "<type>: <Plan Name>" --body "Implements: <Brief summary of plan>

   ===

   <full plan that ended up creating this PR>"
   ```
