# Normal server deployment

In Dokploy, create a Compose application from `readchek/chek` and select
`deploy/server/compose.yaml`. Route your domain to service `chek`, port `8080`.
Set `CHEK_SETUP_TOKEN` to at least 32 random characters, then deploy. Create the
first admin through `/api/v1/setup` as described in the root README.

This uses the Dockerfile's `browser` target: Chromium, Xvfb, fonts and process
cleanup are included. No browser path, display or launch arguments are needed.
The container runs as the `chek` user. Chromium's own sandbox is disabled for
container compatibility; use this image only for trusted source extensions.
The plain Docker build remains browser-free. The App Store demo is unchanged.

The server keeps its default official extension repository and uses its own
`chek-data` volume. Install extensions for your account after setup. There is
no demo account or corpus seed. To expose local books, add your own read-only
bind mount, such as `/srv/books:/library/books:ro`, and add that library in Chek.
Do not reuse the demo's data volume.

## Outbound proxy

Leave the proxy variables unset for direct access. For the existing Chibi
Fly.io HTTP proxy, use these values in Dokploy's Environment:

```ini
CHEK_OUTBOUND_PROXY_URL=http://chibi-flaresolverr-proxy.fly.dev:8888
CHEK_OUTBOUND_PROXY_USERNAME=<your COMIX_CHROME_PROXY_USERNAME value>
CHEK_OUTBOUND_PROXY_PASSWORD=<your COMIX_CHROME_PROXY_PASSWORD value>
```

The credentials must match the proxy service's `PROXY_USERNAME` and
`PROXY_PASSWORD`. Its `PROXY_ALLOW_IP` must allow this server's outbound IP.
Only HTTP CONNECT proxies are supported; SOCKS and HTTPS proxy endpoints are
not supported. Keep credentials in their separate variables, not in the URL.

These settings cover source HTTP requests, images, relayed browser requests
and native browser tunnels. Native tunnels retain declared-host/public-IP
checks and CONNECT to the checked destination IP through the upstream proxy.
There is no direct-connection fallback if the proxy fails. Repository downloads,
OIDC, telemetry and other server integrations do not use this proxy. Without an
explicit proxy, source transports connect directly rather than reading ambient
`HTTP_PROXY` or `HTTPS_PROXY` settings.

The Fly.io service on port 8888 is a proxy, not a solver. Leave
`CHEK_BROWSER_SOLVER_URL` unset for that setup. If you separately run
FlareSolverr, set it to its reachable API endpoint, for example
`http://flaresolverr:8191/v1`. Chek sends the same proxy configuration in solver
requests; the solver itself must be able to reach that proxy. Its API connection
is direct. A separate solver container is not required by this Compose file.

The browser runtime packages dependencies; it does not guarantee that a site
accepts a particular outbound IP. If verification fails, inspect the source
error rather than replacing the solver URL with a proxy address.

For an interactive challenge, open `/verify` on your Chek instance and sign in.
Enter source ID `page.chek.comix` and website `https://comix.to`, open verification,
and complete the site's checkbox. Retry search once the catalog is visible.
Clearance cookies are saved in the data volume; they can expire or be rejected
by the site later. No remote desktop or exposed Chrome debugging port is needed.

## Updating

Redeploy the same Compose application to rebuild from the selected revision.
Keep its data volume. This repository does not currently publish prebuilt images.
For the raw Compose equivalent, use the Git build context with `target: browser`.
