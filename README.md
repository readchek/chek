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
