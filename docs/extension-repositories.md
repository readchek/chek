# Chek extension repositories

Version 1 of the format Chek reads extensions from. A repository is a static
folder served over HTTPS. Keeping one private means using ordinary HTTP
authentication, so any web server, CDN or storage bucket can host one, and
nothing about it is specific to Chek's server.

## Layout

```text
index.json
mangadex-0.1.0.wasm
my-source-1.2.0.wasm
```

`index.json` lists what the repository offers. [`extensions/index.schema.json`](../extensions/index.schema.json)
is its JSON Schema.

```json
{
  "spec": 1,
  "name": "My sources",
  "extensions": [
    {
      "id": "page.example.my-source",
      "name": "My Source",
      "version": "1.2.0",
      "file": "my-source-1.2.0.wasm",
      "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
      "sites": ["example.org"],
      "languages": ["en"],
      "capabilities": []
    }
  ]
}
```

- `spec` is the format version, 1 when absent. A server refuses an index
  written in a newer version than it reads, rather than misreading it.
- `file` is relative to the index. Put components in the same folder, or
  below it, so a private repository's credentials reach them.
- `sha256` is the component's digest in lowercase hex. Chek refuses a
  download that doesn't match, and checks that the component's own manifest
  has the entry's id, version, sites and capabilities.
- Ids are reverse-DNS names. Ids, names and versions are at most 200 bytes. An index
  is at most 1 MiB and lists at most 1,000 extensions, and a component is at
  most 32 MiB.

`extensions/build.sh` writes this folder from the sources in `extensions/sources`.

## Private repositories

A public repository answers requests with `200`. A private one answers
without credentials with `401 Unauthorized` and a
[`WWW-Authenticate`](https://www.rfc-editor.org/rfc/rfc9110#field.www-authenticate)
header naming its scheme:

| Header | Chek asks for | Sends |
| --- | --- | --- |
| `WWW-Authenticate: Bearer realm="…"` | A token | `Authorization: Bearer <token>` |
| `WWW-Authenticate: Basic realm="…"` | A user name and password | `Authorization: Basic …` |
| Another scheme, none, or `403` | One or more headers of your choosing | Those headers as given |

The last row covers hosts that check their own headers, such as
[Cloudflare Access service tokens](https://developers.cloudflare.com/cloudflare-one/identity/service-tokens/),
which take `CF-Access-Client-Id` and `CF-Access-Client-Secret`.

Chek reads the challenge when you add a repository and asks for those
details, then stores them on the server, never in the app. It sends them
only to the index's own origin (scheme, host and port), and it won't follow
a redirect that carries them to another origin. So components hosted
somewhere else are fetched without them.

## Updates

The index is the only source of truth. To publish a new version, upload the
new component under a new file name and rewrite `index.json` with its version
and digest. The server reads every repository's index again every six hours,
and when you refresh by hand, and then offers the new version as an update.
Keep old components until nobody needs them; a file name with its version in
it keeps caches from serving an old component as a new one.

## Who sees a repository

- An admin's repositories are on the server's allow list, and everyone on
  the server can install from them. The official one,
  `https://chek.page/extensions/index.json`, is there to start.
- Other users can add their own once the admin turns on
  `allowAnyRepository`, and only they see those. Turning it off again keeps
  them but stops their extensions running.
- Each person installs only the extensions they choose.

## Hosting

### Cloudflare Workers

A Worker with [static assets](https://developers.cloudflare.com/workers/static-assets/)
serves the folder and checks a token. Next to `dist/`, add `wrangler.jsonc`:

```jsonc
{
  "name": "my-chek-extensions",
  "main": "worker.js",
  "compatibility_date": "2026-10-01",
  "assets": { "directory": "./dist", "binding": "ASSETS", "run_worker_first": true }
}
```

and `worker.js`:

```js
export default {
  async fetch(request, env) {
    const expected = new TextEncoder().encode(`Bearer ${env.CHEK_REPOSITORY_TOKEN}`);
    const given = new TextEncoder().encode(request.headers.get("Authorization") ?? "");
    const ok = given.byteLength === expected.byteLength
      && crypto.subtle.timingSafeEqual(given, expected);
    if (!ok) {
      return new Response("Sign in to this Chek repository.", {
        status: 401,
        headers: { "WWW-Authenticate": 'Bearer realm="chek"' },
      });
    }
    return env.ASSETS.fetch(request);
  },
};
```

Then set a long random token and deploy:

```sh
openssl rand -hex 32 | npx wrangler secret put CHEK_REPOSITORY_TOKEN
npx wrangler deploy
```

Add `https://my-chek-extensions.<account>.workers.dev` in Chek with that
token.

### A private GitHub repository

Commit the folder to a private repository and add it in Chek as
`github:owner/repo`, with `@branch` and `/folder` when it isn't at the root
of the default branch. Use a
[fine-grained token](https://github.com/settings/personal-access-tokens/new)
limited to that repository with read-only Contents access. Chek reads
`raw.githubusercontent.com`, which takes it as a bearer token.

### Any web server

Basic auth with Caddy:

```caddyfile
extensions.example.org {
    basic_auth {
        reader <hash from `caddy hash-password`>
    }
    root * /srv/chek-extensions
    file_server
}
```

Nginx's `auth_basic` works the same way. Both answer `401` with a Basic
challenge, so Chek asks for a user name and password.
