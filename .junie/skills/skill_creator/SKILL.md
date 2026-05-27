---
name: skill-creator
description: Tools and guidelines for creating and validating AI skills according to the norm.
---

# Skill Creator

Use this skill to create new AI skills or validate existing ones for conformity with the official [Agent Skills](https://agentskills.io/specification) specification.

## Trigger
React to commands like:
- "create a new skill for [topic]"
- "validate existing skills"
- "check if [skill] follows the norm"

## The Norm (Skill Specification)

A valid skill must follow these rules:
1. **Location**: Placed in `.junie/skills/<skill-name>/`. (Alternatively `.agents/<skill-name>/` for some tools).
2. **Main File**: Must contain a `SKILL.md` file.
3. **Frontmatter**: `SKILL.md` must start with YAML frontmatter containing:
   - `name`: Unique identifier (kebab-case).
   - `description`: Short summary of the skill's purpose.
4. **Structure**: The body should be clear and actionable, using standard Markdown.

## Guidelines

- **Simplicity**: Keep skills focused on a single responsibility.
- **Actionability**: Instructions must be clear enough for an AI to follow without ambiguity.
- **Standardization**: Always use the YAML frontmatter and standard sections.

## Procedures

### Create a New Skill

1. Create the directory: `mkdir -p .junie/skills/<name>/scripts`
2. Create `SKILL.md` with the required frontmatter.
3. Add mandatory sections: `## Purpose`, `## Trigger`, `## Procedures`.
4. Add optional but recommended sections: `## Guidelines`, `## Examples`.

### Validate Existing Skills

Run the validation script to check for conformity:
```bash
./.junie/skills/skill_creator/scripts/validate_skills.sh
```

If the script reports failures, update the `SKILL.md` files to include missing frontmatter or required fields.

## Quality Checklist

- [ ] Does `SKILL.md` have `---` frontmatter?
- [ ] Are `name` and `description` present in frontmatter?
- [ ] Is the `name` kebab-case?
- [ ] Are instructions actionable?
- [ ] Are there clear triggers for the skill?
