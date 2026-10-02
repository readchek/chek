# Manga

Bun monorepo with one workspace: `apps/api`, the Effect backend.

Run from the repository root:

```sh
bun install
bun run dev
bun run start
bun run typecheck
```

Dependencies for all workspaces use the root `bun.lock`. Add API dependencies
from `apps/api` with `bun add <package>`.

Use `bun run lint` and `bun run format:check` to check code, or `bun run lint:fix`
and `bun run format` to apply fixes and formatting. VS Code and Cursor workspace
settings enable format on save and lint fixes on save with the recommended
`oxc.oxc-vscode` extension installed.
