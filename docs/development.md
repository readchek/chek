# Development

Chek uses Bun workspaces and a single root `bun.lock`. Run these commands from
the repository root unless stated otherwise.

## Setup

Install [Bun](https://bun.sh), then install dependencies:

```sh
bun install
```

Start the landing page with `bun run dev:website` and open
[localhost:18357](http://127.0.0.1:18357/). See the [website guide](website.md)
for builds and deployment.

## Code checks

```sh
bun run typecheck
bun run lint
bun run format:check
```

Use `bun run lint:fix` and `bun run format` to apply fixes. Workspace editor
settings enable formatting and lint fixes on save with the recommended
`oxc.oxc-vscode` extension.

Add dependencies from a workspace directory with `bun add <package>`. All
workspaces share the root lockfile.
