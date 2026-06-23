---
name: git-operations
description: Guidelines for Git usage, commit message formatting, and write access restrictions.
---

## Purpose
Ensure that Git is used safely and that commit messages follow a consistent, imperative style without unnecessary metadata.

## Trigger
- User asks to commit changes.
- User asks to use Git for any operation (branching, merging, etc.).
- Agent identifies a need to perform Git operations.

## Guidelines
- **Read-only by Default**: Never perform Git write operations (commit, push, delete branches, etc.) unless the user has explicitly requested it in the current task.
- **Commit Message Format**:
    - Use the **imperative mood** (e.g., "Add feature" instead of "Added feature" or "Adds feature").
    - Describe what the commit does to the codebase when applied.
    - **No Metadata**: Never add "Co-authored-by", trailers, or other metadata to the commit message.
    - Keep the first line short and descriptive.
- **Atomic Commits**: Prefer small, focused commits that address a single concern.

## Procedures

### 1. Verification of Intent
Before executing any command that modifies the Git repository state (e.g., `git commit`, `git push`, `git checkout -b`, `git branch -d`):
- Verify that the user has explicitly asked for this operation in the `Effective Issue`.
- If not explicitly asked, inform the user that you are refraining from Git modifications per guidelines.

### 2. Crafting Commit Messages
When the user asks for a commit:
1. Identify the core change made to the codebase.
2. Formulate a message in the imperative form: `[Verb] [Object] [Context/Reason]`.
3. Example: `Fix unit test for spec parsing` or `Add support for icecream endpoints`.
4. Execute the commit without any trailers or extra flags that add metadata.

## Examples

**Good Commit Messages:**
- `Refactor spec service to use new repository port`
- `Fix race condition in SSE handler`
- `Add validation for OpenAPI deprecated field`

**Bad Commit Messages:**
- `Fixed the bug` (Past tense)
- `Adds new features` (Present tense)
- `Update stuff` (Vague)
- `Co-authored-by: Junie <junie@jetbrains.com>` (Contains metadata)
