#!/bin/bash

# Script to validate Junie skills in the project
# Conformity according to official AI SKILL definitions (https://agentskills.io/specification)

set -e

SKILLS_DIR=".junie/skills"
FAILED=0

echo "Validating skills in $SKILLS_DIR..."

if [ ! -d "$SKILLS_DIR" ]; then
    echo "Error: $SKILLS_DIR directory not found."
    exit 1
fi

for skill_dir in "$SKILLS_DIR"/*; do
    if [ -d "$skill_dir" ]; then
        skill_name=$(basename "$skill_dir")
        echo "Checking skill: $skill_name"
        
        # 1. Check for SKILL.md
        if [ ! -f "$skill_dir/SKILL.md" ]; then
            echo "  [FAIL] Missing SKILL.md"
            FAILED=1
            continue
        fi
        
        # 2. Check for frontmatter
        if ! head -n 1 "$skill_dir/SKILL.md" | grep -q "^---$"; then
            echo "  [FAIL] Missing YAML frontmatter start (---)"
            FAILED=1
        else
            # Check for name and description in frontmatter
            # We look for the second --- and grep in between
            FM_CONTENT=$(sed -n '2,/^---$/p' "$skill_dir/SKILL.md" | head -n -1)
            
            if ! echo "$FM_CONTENT" | grep -q "^name:"; then
                echo "  [FAIL] Missing 'name' in frontmatter"
                FAILED=1
            fi
            
            if ! echo "$FM_CONTENT" | grep -q "^description:"; then
                echo "  [FAIL] Missing 'description' in frontmatter"
                FAILED=1
            fi
        fi
        
        # 3. Check for recommended sections (Optional but recommended)
        # We just warn for these
        for section in "Procedures" "Guidelines" "Trigger"; do
            if ! grep -q "^## $section" "$skill_dir/SKILL.md"; then
                echo "  [WARN] Missing recommended section: ## $section"
            fi
        done
        
        echo "  [OK] Done checking $skill_name"
    fi
done

if [ $FAILED -eq 1 ]; then
    echo "Validation FAILED for one or more skills."
    exit 1
else
    echo "All skills validated successfully (conformity checks passed)."
    exit 0
fi
