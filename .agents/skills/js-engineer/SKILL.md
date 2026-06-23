---
name: js-engineer
description: Enforce JavaScript project policy, formatting, and quality standards for Sanshain UI.
---

# JS Engineer — Policy & Pitfalls

This skill encodes the project policy for JavaScript development in Sanshain. It ensures UI consistency, performance, and adherence to modern standards and internal patterns.

## Purpose
To provide proactive guidance for frontend development, ensuring clean, maintainable, and well-formatted JavaScript code.

## Trigger
Use this skill whenever:
- Creating or modifying JavaScript files (`.js` files) in `static/js/`.
- Modifying HTML templates (`.html`) or Tailwind CSS styles.
- Updating `package.json` or related build configurations.

## Setup Check

1. **Node.js Environment** — Verify Node.js version if required by the project.
2. **Dependencies** — Check `package.json` for used libraries (e.g., Mermaid.js, Dagre, Tailwind CSS).
3. **Linting & Formatting** — Ensure `eslint.config.js` and Prettier are correctly configured.

## MUST DO

- **Format Code** — Always run `npx prettier --write static/js/` after modifying `.js` files.
- **Lint Code** — Always run `npx eslint static/js/` and fix all warnings and errors.
- **Tailwind Consistency** — Use Tailwind CSS classes for styling. Avoid custom CSS unless absolutely necessary (documented in `static/graph.html` for specific graph needs).
- **DOM Safety** — Use `_graphEscapeHtml` (or equivalent) when rendering dynamic content to prevent XSS.
- **Modularity** — Keep `common.js` for shared utilities and separate feature-specific logic into dedicated files (e.g., `graph.js`, `discovery.js`).
- **Modern Syntax** — Use ES6+ features like `const`/`let`, arrow functions, and template literals.

## MUST NOT DO

- **No Global Pollutions** — Avoid adding variables to the `window` object unless necessary for inter-script communication (e.g., `report` data).
- **No Direct Style Mutation** — Prefer toggling Tailwind classes (`element.classList.toggle`) over direct `element.style` modifications where possible.
- **No Inline Event Handlers** — Avoid `onclick="..."` in HTML; use `addEventListener` in JS instead.
- **No Console Logs in Production** — Remove or comment out `console.log` before committing, unless it's essential for diagnostics.

## Procedures

### 1. Pre-Implementation Review
- Check `static/js/common.js` for existing utility functions.
- Verify the target HTML file for correct container IDs and existing script imports.

### 2. Implementation
- Apply logic changes using modern JavaScript patterns.
- Use Tailwind classes for any new UI elements.
- Ensure interactive elements handle edge cases (e.g., missing data, network errors).

### 3. Post-Implementation Check
- Run `npx prettier --write static/js/`.
- Run `npx eslint static/js/`.
- Perform manual UI verification to ensure interactivity works as expected.

## Quality Checklist
- [ ] Code is formatted with Prettier.
- [ ] No ESLint warnings or errors.
- [ ] UI elements use Tailwind CSS consistently.
- [ ] Dynamic content is properly escaped.
- [ ] Event listeners are correctly attached and cleaned up if necessary.
