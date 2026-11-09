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
  has the entry's id, version, sites and capabilities, and that it reads the
  entry's formats.
- `formats` lists what the extension's chapters come in: `images`, `epub`
  or `pdf`. Leave it out for a source of images, as indexes from before
  formats existed do; `chek ext index` writes it from the component.
- `comments` lists the orders the extension's readers' comments come in:
  `top`, `newest` or `oldest`, the site's default first, or `[]` when it
  gives only a count. Leave it out for an extension without comments.
  `chek ext index` writes it from the component, and Chek refuses a
  component that gives different orders. Comments show only when the admin
  turns on `extensionComments`, and never for a user's own repository.
- `minServer` is the oldest Chek release that runs the build, such as
  `0.6.0`. `chek ext index` writes it from the version of the source
  contract the component was built against. A server older than that
  doesn't offer the build; servers from before the field ignore it and
  fail to link the build instead. Leave it out to mean any server that
  links the build's contract.
- `previous` lists older builds of the same extension, newest first, each
  an entry like this one with the same id. A server installs the newest
  build it runs, so people whose server is too old for the new build keep
  getting one that works. See [Older servers](#older-servers).
- Ids are reverse-DNS names. Ids, names and versions are at most 200 bytes. An index
  is at most 1 MiB and lists at most 1,000 extensions, and a component is at
  most 32 MiB.

`chek ext build` writes this folder from the sources in a workspace, and
`chek ext index` writes `index.json` for any folder of components.

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

Two common hosts don't challenge, so Chek recognizes them: a private
repository's files on `raw.githubusercontent.com` answer `404` to anyone not
signed in, which Chek takes as asking for a token, and Cloudflare Access
redirects to its sign-in page, which Chek takes as asking for headers.

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

## Older servers

When the SDK moves to a new version of the source contract, new builds need
a newer Chek server. To keep serving servers that haven't updated, keep the
last build made for the old contract next to the new one. Put it in a
`previous` folder at the root of your sources' workspace, and `chek ext
build` copies it into the repository. `chek ext index` lists each extension
once, as its newest build, and puts the newest build for each older
`minServer` in `previous`:

```json
{
  "id": "page.example.my-source",
  "version": "2.0.0",
  "file": "my-source-2.0.0.wasm",
  "minServer": "0.7.0",
  "previous": [
    {
      "id": "page.example.my-source",
      "version": "1.4.0",
      "file": "my-source-1.4.0.wasm",
      "minServer": "0.6.0"
    }
  ]
}
```

Entries are shortened here; each build lists every field an entry has. A
server on 0.6.0 installs 1.4.0 and doesn't offer 2.0.0 as an update. Once it
updates to 0.7.0, which links only the new contract, it offers 2.0.0, and
1.4.0 stops loading until the user updates. Servers before 0.5.4 don't read
`minServer` or `previous`, and fail to install a build they can't link.

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

`chek ext deploy cloudflare` does all of this, setting a random token; see
[`extensions/README.md`](../extensions/README.md#publish-a-private-repository).
By hand, a Worker with [static assets](https://developers.cloudflare.com/workers/static-assets/)
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
    const token = env.CHEK_REPOSITORY_TOKEN;
    // Until a token is set, serve nothing rather than accept "Bearer undefined".
    if (!token) return new Response("Not ready.", { status: 503 });
    const expected = new TextEncoder().encode(`Bearer ${token}`);
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

Basic auth with Caddy 2.8 or later (earlier versions spell it `basicauth`):

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
