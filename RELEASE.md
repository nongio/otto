# Release Process

This document describes the process for creating a new release of Otto.

## Prerequisites

- Ensure all changes for the release are merged to `main`
- Update version in `Cargo.toml` if needed
- Have `git-cliff` installed: `cargo install git-cliff`

## Git Worktree Issue

**Important:** `git-cliff` does not work properly in git worktrees. If you're working in a worktree (like `otto_3`), you need to run cliff from the main repository.

## Release Steps

### 1. Update Version (if needed)

Edit `Cargo.toml` and update the version number:

```toml
[package]
version = "0.x.0"
```

### 2. Generate Changelog

Since this is a worktree, run `git cliff` from the main repository:

```bash
# Find the main repository path
git rev-parse --git-common-dir
# Example output: /home/riccardo/dev/otto/.git

# Navigate to main repository
cd /home/riccardo/dev/otto

# Generate changelog with the new version tag
git cliff --tag v0.x.0 -o CHANGELOG.md

# Copy the generated changelog back to the worktree
cp CHANGELOG.md /path/to/worktree/CHANGELOG.md
```

Or as a one-liner from the worktree:

```bash
# From the worktree directory
MAIN_REPO=$(git rev-parse --git-common-dir | sed 's|/.git$||')
cd "$MAIN_REPO" && git cliff --tag v0.x.0 -o CHANGELOG.md && cp CHANGELOG.md -
cd - && cp "$MAIN_REPO/CHANGELOG.md" .
```

### 3. Commit and Tag

```bash
# Add the changelog
git add CHANGELOG.md

# Commit the release (without GPG signing if needed)
git commit --no-gpg-sign -m "chore: release v0.x.0"

# Create annotated tag
git tag v0.x.0 -m "Release v0.x.0"
```

### 4. Verify Release

```bash
# Check the commit and tag
git log --oneline -3
git tag -l | tail -5

# View the changelog
head -100 CHANGELOG.md
```

### 5. Push to Remote

```bash
# Push the commit
git push origin main

# Push the tag
git push origin v0.x.0
```

## Useful Commands

### Preview Changelog Before Release

```bash
# From main repository
cd /home/riccardo/dev/otto
git cliff --unreleased
```

### View Changes Since Last Release

```bash
git log v0.12.0..HEAD --oneline
```

### Check Current Version

```bash
grep '^version' Cargo.toml
git describe --tags --abbrev=0
```

### List All Tags

```bash
git tag -l | sort -V
```

## Cliff Configuration

The changelog is generated using `cliff.toml` configuration. The format follows conventional commits with these groups:

- 🚀 Features (`feat:`)
- 🐛 Bug Fixes (`fix:`)
- 🚜 Refactor (`refactor:`)
- 📚 Documentation (`doc:`)
- 🎨 Styling (`style:`)
- 🧪 Testing (`test:`)
- ⚙️ Miscellaneous Tasks (`chore:`, `ci:`)
- 🛡️ Security (body contains "security")
- ◀️ Revert (`revert:`)

## Troubleshooting

### Cliff produces empty changelog

- **Cause:** Running in a worktree
- **Solution:** Run from main repository (see step 2 above)

### GPG signing fails

```bash
# Commit without GPG signing
git commit --no-gpg-sign -m "message"
```

### Need to amend the release commit

```bash
# Make changes, then:
git add .
git commit --amend --no-edit --no-gpg-sign

# Recreate the tag
git tag -d v0.x.0
git tag v0.x.0 -m "Release v0.x.0"
```
