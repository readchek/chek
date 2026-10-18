# Deploying a demo instance

[`deploy/demo`](../deploy/demo) runs the server with the reader test corpus
already loaded, for trying apps against a real server. It's a Docker Compose
file written for [Dokploy](https://dokploy.com), and it works with plain
`docker compose` too.

## What it sets up

- **`chek`** is the server, built from this repository's Dockerfile. Its
  database and covers live in the `chek-data` volume.
- **`seed`** runs once per deploy and exits. On the first run it downloads
  the pinned corpus snapshot (about 600 MB) from `assets.chek.page`, checks
  every part's SHA-256, and unpacks it into the `library` volume. It then
  creates the admin and three libraries:
  - **Comics**: Pepper & Carrot episodes and books, and Little Nemo.
  - **Books**: one copy of each EPUB title plus the text PDFs.
  - **Test corpus**: the whole corpus as it is, with every variant and
    format, including files the server can't read yet.

  Comics and Books are hard links into the corpus, so they take no extra
  space. Later runs skip what's already done, so redeploying is quick.
- A reader account named `demo` that can open all three libraries, for
  handing to testers.

The corpus is public-domain and Creative Commons material; its sources and
licenses are in the `provenance` folder of the Test corpus library.

## Dokploy

1. Create a **Compose** application from `readchek/chek`, branch `main`,
   with compose path `./deploy/demo/compose.yaml`.
2. In **Environment**, set three values. Generate the token with
   `openssl rand -base64 32`.

   ```
   CHEK_SETUP_TOKEN=<at least 32 characters>
   DEMO_ADMIN_PASSWORD=<password for the admin account>
   DEMO_READER_PASSWORD=<password for the demo account>
   ```

3. In **Domains**, add your domain for service `chek` on port `8080`, with
   HTTPS on. Point the domain's DNS at the server first.
4. Deploy. The `seed` container's log shows the download and finishes with
   `seed: done`. Scans continue in the server after that.

Sign in as `admin` or `demo`. OPDS apps use
`https://<domain>/opds/v2/catalog` with the same names and passwords.

`DEMO_READER_PASSWORD` is only used when the `demo` account is created, so
changing it later doesn't sign testers out. Change it through the API instead.

## Behind Cloudflare

With the domain proxied by Cloudflare or served through a Cloudflare tunnel,
every request reaches the server from the same place, so all clients share
one limit on wrong passwords. Add `CHEK_CLIENT_IP_HEADER=CF-Connecting-IP`
to the environment so each client is limited on its own. Only do this if
the server's Traefik can't be reached except through Cloudflare, for
example with ports 80 and 443 closed to the internet; otherwise a client
can send its own `CF-Connecting-IP` and dodge the limit.

## Logs and traces

The server logs every request with its route, status and duration. To send
those logs and request traces to SigNoz or another OpenTelemetry collector,
add `OTEL_EXPORTER_OTLP_ENDPOINT` to the environment with the collector's
OTLP/HTTP address, such as `http://signoz-otel-collector:4318`, and redeploy.
The collector must be on a Docker network the `chek` container shares;
Dokploy puts `chek` on `dokploy-network`. Services show up as `chek-demo`
unless `OTEL_SERVICE_NAME` says otherwise.

## Without Dokploy

The same file runs anywhere with Docker. Put the three values in
`deploy/demo/.env`, then:

```sh
cd deploy/demo
docker compose up -d --build
docker compose logs -f seed
```

The server isn't published on a host port, since Dokploy's Traefik reaches
it over Docker's network. Add a reverse proxy with HTTPS on the same network,
or a `ports` entry for a quick local look.

## Starting over

Deleting the `chek-data` volume removes accounts, progress and covers; the
next deploy sets everything up again from the corpus already downloaded.
Deleting the `library` volume as well downloads the corpus again.
