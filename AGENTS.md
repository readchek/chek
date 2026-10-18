# Chek development rules

These instructions apply throughout this repository. Follow the user's current
scope and explicit decisions first. Guidance and skills do not authorize extra work.

## Required references

- [Product direction](docs/product.md): product scope and planned features.
- [Server design](docs/server.md): the server's settled decisions and code layout.

This repository is the Chek server, written in Rust. The landing page lives in
[readchek/chek.page](https://github.com/readchek/chek.page), and the reader and
native Apple apps are developed separately.
Research proposals are starting points to validate, not implemented capabilities
or final choices. Keep approved decisions, proposals, and verified behavior distinct.
Check date-sensitive claims against current primary sources or installed dependencies.

## Scope and conduct

- Complete authorized work end to end. Continue routine, reversible steps without
  repeated approval; ask when a missing decision changes scope, risk, or correctness.
- Research, planning, and documentation requests do not authorize implementation.
  Stop when asked and resume only when the user authorizes it.
- Stay in this repository. Do not inspect or modify the old Chibi checkout without
  explicit authorization.
- Inspect the working tree before editing. Preserve unrelated changes, untracked
  files and exported `.agents/` skills. Never reset or clean the
  checkout to make your task easier.
- Do not commit, push, deploy, publish, or send external messages unless requested
  or already authorized. Use conventional commit messages when commits are requested.
- Delegate only when requested or explicitly called for by applicable instructions.
  Give agents independent scopes and synthesize their findings.
- Write concise, candid English. Separate facts, inference, and unperformed checks.
  Link primary sources for research claims. Use the Visualize skill when helpful.
- Keep README concise and project-facing. Put durable decisions in `docs/`, not
  session journals or speculative architecture documents.

## Editing and checks

1. Read affected code and applicable instructions before changing it.
2. Use `apply_patch` for focused source edits. Do not rewrite source with Python,
   Perl, sed, or ad hoc JavaScript replacements for convenience.
3. Before structural or repeated transformations, check for suitable available
   language-aware codemods. Use them when they fit; otherwise use explicit patches.
   Do not install tooling merely to satisfy this rule.
4. Keep changes focused. Prefer concrete types and simple code; avoid speculative
   abstractions, unrelated refactors, and dependencies without a clear benefit.
5. Inspect the diff. Run the relevant existing formatter and focused checks.
   Format only files within scope and report material validation limits.

Established generators and formatters are allowed. Scripted source rewriting needs
an explicit user request or an established generator/codemod. Data processing and
artifact generation are separate from source rewriting.

**Do not write or add tests unless the user explicitly asks.** Relevant existing
tests, compiler checks, validators, and manual verification may be run. Do not
disguise new tests as temporary verification scripts.

## Rust

- One Cargo workspace; crates live in `crates/` and are split by responsibility,
  as [Server design](docs/server.md) lists. Add a crate only for a new responsibility.
- `openapi.yaml` is the API contract. Change it in the same pull request as the
  handlers in `crates/chek-server/src/api.rs`.
- Run before pushing: `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, and `bunx @redocly/cli lint openapi.yaml`.
- Catalog and scan code blocks; call it from async code through `spawn_blocking`.
- Keep scan work capped below the core count so requests stay fast during scans.
- Every listener sets `TCP_NODELAY`.
- Image work goes through libvips (`libvips-dev` to build). Call its
  operations without `_with_opts`: those set every option of the newest
  libvips and fail on older versions.
- Debug builds keep line tables only for our crates. Run `cargo sweep` now and
  then to clear stale build output rather than deleting `target/` wholesale.

## Bun

- Use Bun instead of Node.js, npm, pnpm, yarn, ts-node, or npx: `bun <file>`,
  `bun run <script>`, `bun install`, and `bunx <package> <command>`.
- Use `bun test` for authorized JavaScript/TypeScript tests. Do not write unrequested tests.
- Prefer Bun builds and HTML imports with Bun.serve for applicable new frontends.
  Do not introduce Vite or webpack; preserve established project tooling.
- Bun loads `.env`; do not add dotenv.
- Prefer Bun.serve over Express, bun:sqlite over better-sqlite3, Bun.redis over
  ioredis, Bun.sql over pg/postgres.js, and built-in WebSocket over ws.
- Prefer Bun.file for reads/writes and Bun's shell API over execa.
