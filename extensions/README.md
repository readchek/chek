# Source extensions

Sources Chek reads series from, built as WebAssembly components against the
contract in [`crates/chek-sources/wit/source.wit`](../crates/chek-sources/wit/source.wit).
How the server runs them is in [Server design](../docs/server.md#extension-host).

| Path | Contents |
| --- | --- |
| `sdk` | `chek-extension`, the contract's bindings and request helpers |
| `sources/*` | One crate per source |
| `build.sh` | Builds every source into a repository folder |

## Build a repository

Needs Rust with the `wasm32-wasip2` target (`rustup target add wasm32-wasip2`).

```sh
./build.sh                 # writes dist/
./build.sh out "My sources" # another folder and repository name
```

The folder holds one `<crate>-<version>.wasm` per source and an
`index.json` written from each component's own manifest, with its SHA-256
digest. Serve it over HTTPS and add the folder's URL in Chek. A private
repository can sit behind a bearer token, basic auth or a custom header,
which Chek sends only to the repository's own origin.

## Write a source

Copy `sources/mangadex`, rename the crate, and implement `Guest`:

- `manifest` names the extension's id in reverse-DNS form, its version, and
  every host it fetches from, including image hosts. Requests anywhere else
  fail. Add `Capability::Browser` for sites behind Cloudflare; the server's
  admin runs the solver that gets past the challenge.
- `search`, `series`, `chapters` and `pages` return the source's own ids,
  and images as URLs with any headers the site needs. Chek fetches and
  caches images itself.
- `Get::new(url).cache(seconds).json()` fetches and parses a response, and
  lets Chek answer repeats from its cache. Cache listings; don't cache
  anything with a short-lived token in it.
- `cache` keeps values between calls and `cookies` holds the jar every
  request already sends. Both survive restarts.

Bump the crate's version for every change: installs update when the index
lists a new version.
