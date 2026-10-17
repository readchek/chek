# Reader corpus

Real downloaded publications and publisher samples for reader development. The
local corpus and download cache are ignored by Git. The catalog, rights evidence,
and checksum lockfile stay in Git.

```sh
cf auth login
bun run corpus:setup
bun run corpus:verify
```

The setup CLI uses Effect v4 for flags, schema validation, streams, retries,
child processes, and scoped cleanup. Setup requires Bun, `tar`, and the Cloudflare `cf` CLI with read access to the
private `chek-test-corpus` R2 bucket. An existing valid corpus needs no login or
network access. `--destination /absolute/path` restores to another directory.

The immutable archive is split into parts below the CLI upload limit. Setup
reuses verified parts after interruption, checks every SHA-256 digest, validates
archive paths and entry types, and extracts into a fresh directory before moving
it into place. It refuses to overwrite an altered or incomplete local corpus;
move that directory aside to restore. Nested publication archives are kept intact.

See [the catalog](corpus-catalog/README.md) for sources and rights,
[coverage](corpus-catalog/COVERAGE.md) for formats and gaps, and
[the lockfile](corpus.lock.json) for the exact snapshot. Public-domain status can
depend on jurisdiction; keep source notices and attribution with these samples.

The R2 bucket has no public download endpoint. Uploads and new snapshots are
maintainer operations; setup only reads the pinned objects. Old versioned objects
must remain available while a checked-in lockfile refers to them.
