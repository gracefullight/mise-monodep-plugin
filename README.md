# mise-monodep-plugin

`monodep` is a `mise` tool plugin that deduplicates dependencies across monorepo workspaces.

## What it does

- Reads the nearest `mise.toml` with `[monorepo].config_roots`
- Discovers workspace packages (Node.js + Python)
- Delegates dependency installation to your existing package manager (bun/pnpm/npm, uv)
- Creates symlinked `node_modules` entries for `workspace:*` and `file:` dependencies
- **Deduplicates** packages shared across workspaces via hardlinks to `.monodep/store`
- Supports Node.js (`node_modules/.bun/`) and Python (`.venv/site-packages/`)
- Commands: `sync`, `plan`, `doctor`, `why`, `remove`

## How it works

```
mise.toml [monorepo]        monodep sync .
┌─────────────────┐        ┌────────────────────────────┐
│ config_roots =  │        │ 1. Discover workspaces     │
│   ["apps/*",    │  ────→ │ 2. PM install (bun/uv)     │
│    "packages/*"]│        │ 3. Symlink workspace deps   │
│ package_manager │        │ 4. Deduplicate via hardlink │
│   = "bun"       │        └────────────────────────────┘
└─────────────────┘
                           .monodep/store/
                           ├── node/zod/4.3.6/          ← one copy
                           └── python/pydantic/2.13.1/  ← one copy

                           apps/web/.bun/.../zod/
                             package.json  ← hardlink (same inode)
                           apps/api/.venv/.../pydantic/
                             __init__.py   ← hardlink (same inode)
```

## Configuration

```toml
# mise.toml
[monorepo]
config_roots = ["apps/*", "packages/*"]
package_manager = "bun"  # optional: auto-detected from lockfile or PATH
```

## Usage

```bash
mise plugins link monodep ./packages/mise-monodep-plugin

# Full sync: PM install + workspace linking + dedup
mise x monodep@0.1.0 -- monodep sync .

# Re-link + dedup only (skip PM install)
mise x monodep@0.1.0 -- monodep sync . --skip-install

# Check workspace symlink health
mise x monodep@0.1.0 -- monodep doctor .

# Trace why a dependency exists
mise x monodep@0.1.0 -- monodep why shared-lib .

# Remove a dependency from a workspace
mise x monodep@0.1.0 -- monodep remove app-a dev-tool --group devDependencies
```
