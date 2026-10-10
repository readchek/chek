# Chek website

The landing page at [chek.page](https://chek.page), with an interactive reading
preview. The preview is a concept, not a connected reader client.

Run from the repository root:

```sh
bun install
bun run dev:website
```

Open [localhost:18357](http://127.0.0.1:18357/). Set `PORT` to use another port.

```sh
bun run build:website
```

The static build is written to `apps/website/dist/`.

See the [website guide](../../docs/website.md) for deployment, asset maintenance,
font licenses, and provider logo sources.
