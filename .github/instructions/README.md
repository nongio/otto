# GitHub Copilot Instructions

This directory contains instruction files for GitHub Copilot and AI agents working on this repository.

## Available Instructions

### [documentation.instructions.md](./documentation.instructions.md)
Guidelines for writing and updating documentation.

### [review.instructions.md](./review.instructions.md)
Code review guidelines and best practices.

### [hello-design-component.instructions.md](./hello-design-component.instructions.md)
**Skill for creating HelloDesign UI components**

Defines the standard 4-layer architecture:
- **State Layer** - Component state/model
- **Style Layer** - Visual configuration  
- **Renderer Layer** - Pure drawing functions
- **Component Layer** - Public API

Use when creating new components in `components/hello-design/src/components/`

## Usage

These instruction files are automatically read by GitHub Copilot to provide context-aware assistance. The `.instructions.md` suffix is recognized by Copilot as instruction content.

## Naming Convention

- Use descriptive names: `{topic}.instructions.md`
- Use kebab-case for file names
- Include a clear header with scope and usage information
