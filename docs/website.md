# Website guide

The landing page is a static site at [chek.page](https://chek.page). Its reader
preview is a concept, not a connected app or server demo. See [design](design.md)
for the visual direction.

Paths below are relative to `apps/website/` unless stated otherwise.

## Local development

From the repository root:

```sh
bun run dev:website
bun run build:website
```

The preview runs at [localhost:18357](http://127.0.0.1:18357/). Override with
`PORT`. The static build is written to `apps/website/dist/`.

## Deployment

From the repository root, after signing in with `cf auth login`:

```sh
bun run deploy:website
```

The command builds the static site with Bun and publishes it with
`cf deploy --prebuilt`. `cloudflare.config.ts` defines the `chek-website`
hosting resource and `chek.page` custom domain. `scripts/build.ts` prepares
assets-only output in `.cloudflare/output/v0/`. No server code is deployed.
Generated output is ignored by Git.

## Structure

- `src/`: HTML entry, reader interactions, styles, and Bun development server.
- `scripts/`: Bun build and Cloudflare static-output preparation.
- `public/assets/fonts/`: local typefaces and their licenses.
- `public/assets/logos/`: Chek and official provider marks.
- `public/assets/images/`: original reading artwork.
- `dist/`: generated static build, ignored by Git.

`tsconfig.json` enables Bun HTML-import types and browser DOM types. Run
`bun run --filter @chek/website typecheck`, or use the root `bun run typecheck`
to check every workspace.

## Artwork, fonts, and loading

The artwork was generated with the built-in imagegen tool. The page uses `public/assets/images/last-train.webp`: about 652 KB, at 1024 × 1536. The other two sample covers and the logo are code-native graphics. Sample titles are fictional, not catalog claims.

The landing page uses 512px (151 KB) and 768px (287 KB) WebP versions of the artwork, selected by screen density. Both hero illustrations share the same image URL. The shelf uses a lazy-loaded 256px cover (39 KB). The original 1024px artwork loads only when the reader preview opens; it is retained for detailed reading. These smaller derivatives use WebP quality 82.

Production uses three local WOFF2 fonts, about 73 KB combined, preloaded with
matching anonymous CORS mode. Original TTF files remain as masters; only WOFF2
files are deployed. Fonts stay separate from the stylesheet. The local
development bundler may embed fonts.

Font filenames include the first 12 characters of their SHA-256 content hash. Regenerate their names, CSS references, and preload links together if the font files change. The build generates `_headers` rules with `Cache-Control: public, max-age=31536000, immutable` for versioned fonts, CSS, JavaScript, artwork, provider marks, and favicon assets. HTML and the conventional `/favicon.ico` keep Cloudflare's default revalidation policy, so future releases still appear. The detailed artwork also receives a content-hashed URL during the build.

The reader shows the already loaded hero artwork immediately, then swaps to the detailed page after decoding. If loading fails, the preview stays readable and reopening retries. The dialog keeps focus contained, restores focus to its trigger, supports Escape, and sizes its canvas around the controls. Close and mode controls have 44px hit areas. Custom buttons disable native appearance so Safari preserves their intended colors. Pointer hover effects are gated to suitable devices; reduced motion disables button movement.

The full-size WebP was encoded with `cwebp -q 88 -m 6 -sharp_yuv`. The original PNG is not retained in this repository.

## Mark exports and site icons

`public/assets/logos/mark.svg` is the square-cornered master. PNG exports have no baked-in corner radius:

- `mark-180.png`: Apple touch icon, about 1.6 KB.
- `mark-192.png` and `mark-512.png`: smaller general-purpose exports.
- `mark-1024.png`: high-resolution export, about 8 KB.
- `mark-2048.png`: high-resolution export, about 20 KB.

The PNGs are rasterized from the SVG with an optimized 256-color palette. The HTML declares the SVG favicon, a 32px PNG fallback, a multi-size ICO (16/32/48px), and the 180px Apple touch icon. The build also writes `/favicon.ico` for browsers that request that conventional path. Apple applies its own rounded mask.

## Provider logo provenance

The source chips use the existing Chek mark (`public/assets/logos/mark.svg`) and unmodified official provider SVGs. Logos retain their original colors; each sits beside its visible provider name.

- `public/assets/logos/kavita-logo.svg`: [Kareadita/Kavita official app logo](https://github.com/Kareadita/Kavita/blob/f75863cb77c0f6aa26e0794a1848294e63c9d07d/UI/Web/src/assets/images/logo.svg), revision `f75863cb77c0f6aa26e0794a1848294e63c9d07d`. Downloaded from [the revision-pinned raw SVG](https://raw.githubusercontent.com/Kareadita/Kavita/f75863cb77c0f6aa26e0794a1848294e63c9d07d/UI/Web/src/assets/images/logo.svg).
- `public/assets/logos/komga-logo.svg`: [gotson/komga official web UI logo](https://github.com/gotson/komga/blob/519e352aa894b2fdc80660f1dde21b98cf328e3e/komga-webui/src/assets/logo.svg), revision `519e352aa894b2fdc80660f1dde21b98cf328e3e`. Downloaded from [the revision-pinned raw SVG](https://raw.githubusercontent.com/gotson/komga/519e352aa894b2fdc80660f1dde21b98cf328e3e/komga-webui/src/assets/logo.svg).
