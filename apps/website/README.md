# Chek website

The selected landing page for Chek, the ground-up successor to Chibi Manga: an e-reader for books and comics from local files or a preferred library server. The existing Chek logo is selected; later refinements may improve spacing and small-size clarity.

## Preview

From the repository root:

```sh
bun run dev:website
bun run build:website
```

Open http://127.0.0.1:18357/. Override with `PORT` if necessary. The development server binds to localhost. The static build is written to `apps/website/dist/`; production is hosted at https://chek.page.

## Deployment

From the repository root, after signing in with `cf auth login`:

```sh
bun run deploy:website
```

The command builds the static site with Bun, then publishes its assets with `cf deploy --prebuilt`.
`cloudflare.config.ts` defines the `chek-website` Worker and `chek.page` custom domain.
`scripts/build.ts` cleans and builds `dist/`, then uses Cloudflare’s build-output
utility to prepare assets-only output in `.cloudflare/output/v0/`. There is no
Worker bundle, Wrangler dependency, or deployed server code. The Bun development
server stays local. Generated Cloudflare output is ignored.

The installed `cf` CLI calls its static hosting “Pages on Workers”;
`cf pages deploy` explicitly directs users to `cf deploy`. The `worker` config
names that hosting resource and its domain, not an application server.

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

## Design

The opening behaves like the cover of a manga volume: loud blue, large type, a printed page, and a reader you can touch. Everything below it becomes calmer: a short project introduction, the available source choices being planned, and a development note. There is no feature pitch, signup, pricing, waitlist, or invented download link.

- Cobalt `#2448e8`: the primary brand surface.
- Paper `#fafaf7`: reading and information surfaces.
- Ink `#171820`: text and the phone.
- Mist `#eef0f8`: the library section.
- Butter `#eff3ad`: the primary action and speech balloon.
- Bricolage Grotesque 700 for display; DM Sans 400 and 600 for body and controls. All fonts are local, with OFL licenses in `public/assets/fonts/`.

The artwork is the main expressive element. There are no automatic reveals or perpetual animations. The reader dialog has whole-page and vertical-panel modes, native focus containment and Escape dismissal. The source diagram gives local files, Komga, Kavita, and the planned Chek server equal standing. Chek’s own server is a standalone open-source library server in development, not a required service. The page identifies iOS as the first native client, wider platforms as planned, and broad format support as a goal without promising a finalized format list.

## Artwork

The artwork was generated with the built-in imagegen tool. The page uses `public/assets/images/last-train.webp`: 652 KB instead of 4.23 MB, at the same 1024 × 1536 dimensions (about 85% less data). The other two sample covers and the logo are code-native graphics. Sample titles are fictional, not catalog claims.

The landing page uses 512px (151 KB) and 768px (287 KB) WebP versions of the artwork, selected by screen density. Both hero illustrations share the same image URL. The shelf uses a lazy-loaded 256px cover (39 KB). The original 1024px artwork loads only when the reader preview opens; it is retained for detailed reading. These smaller derivatives use WebP quality 82.

Production uses three WOFF2 fonts (72.7 KB combined, versus 178.8 KB for the TTFs), preloaded with matching anonymous CORS mode. Glyph coverage and horizontal metrics are unchanged. The original TTF files remain as masters; only the WOFF2 files are deployed. Fonts remain separate from the roughly 25 KB stylesheet. The local development bundler may still embed fonts.

Font filenames include the first 12 characters of their SHA-256 content hash. Regenerate their names, CSS references, and preload links together if the font files change. The build generates `_headers` rules with `Cache-Control: public, max-age=31536000, immutable` for versioned fonts, CSS, JavaScript, artwork, provider marks, and favicon assets. HTML and the conventional `/favicon.ico` keep Cloudflare's default revalidation policy, so future releases still appear. The detailed artwork also receives a content-hashed URL during the build.

The reader shows the already loaded hero artwork immediately, then swaps to the detailed page after decoding. If loading fails, the preview stays readable and reopening retries. The dialog keeps focus contained, restores focus to its trigger, supports Escape, and sizes its canvas around the controls. Close and mode controls have 44px hit areas. Custom buttons disable native appearance so Safari preserves their intended colors. Pointer hover effects are gated to suitable devices; reduced motion disables button movement.

The full-size WebP was encoded with `cwebp -q 88 -m 6 -sharp_yuv`. The original PNG is not retained in this repository.

Final generation prompt:

> Use case: illustration-story. Asset type: original monochrome manga page for an independent manga reader landing page, displayed as printed artwork and inside a phone. Create an exceptionally beautiful black ink and white paper manga page, portrait 2:3 composition. A young adult courier with short tousled dark hair, wearing a roomy black jacket and a messenger bag, stands on the roof of a train passing through a fantastical dense Japanese coastal city. Huge billowing clouds, tiny birds, overhead cables, stacked little houses, tall distant towers and sparkling sea. Sense of freedom, adventure, contemplative wonder. Make a full finished comic page with three asymmetrical panels: top roughly 65% is a spectacular panoramic view looking past the courier at the city; bottom left close-up of their confident eyes and wind in hair; bottom right train tracks flying past toward distant mountains. Intricate confident fine pen linework, strong black shadow shapes, stipple and screentone texture, high contrast. Refined professional manga craftsmanship. No speech bubbles, no text, no lettering, no logos, no watermarks. Pure black and white only, white thin panel gutters. The page artwork should extend to image edges.

## Validation

The design study was checked before promotion:

- Browser checked at 1440, 1280, 768, 390, and 320px widths.
- No page horizontal overflow at the measured desktop, tablet, and mobile widths.
- Reader dialog triggers, page/scroll mode selection, close button, Escape dismissal, focus restoration, and section navigation checked.
- All artwork loaded and no browser console warnings or errors observed.
- `bunx oxlint --deny-warnings apps/website`
- `bunx oxfmt --check apps/website`
- `bun run build:website`

After restructuring, both workspaces passed TypeScript checks. The website build, lint, and formatting checks passed.
A fresh browser load verified the moved assets and reader dialog without console errors.

The reader preview is an interactive concept, not the app or a connected server demo. No deployment or release links were added. Physical iOS/Safari behavior and screen-reader speech were not tested.

## Mark exports and site icons

`public/assets/logos/mark.svg` is the square-cornered master. PNG exports have no baked-in corner radius:

- `mark-180.png`: Apple touch icon, about 1.6 KB.
- `mark-192.png` and `mark-512.png`: smaller general-purpose exports.
- `mark-1024.png`: high-resolution export, about 8 KB.
- `mark-2048.png`: high-resolution export, about 20 KB.

The PNGs are rasterized from the SVG with an optimized 256-color palette. The HTML declares the SVG favicon, a 32px PNG fallback, a multi-size ICO (16/32/48px), and the 180px Apple touch icon. The build also writes `/favicon.ico` for browsers that request that conventional path. Apple applies its own rounded mask.

## Provider logo provenance

The source chips use the existing Chek mark (`public/assets/logos/mark.svg`) and unmodified official provider SVGs saved locally on 2026-10-08. Logos retain their original colors; each sits beside its visible provider name.

- `public/assets/logos/kavita-logo.svg`: [Kareadita/Kavita official app logo](https://github.com/Kareadita/Kavita/blob/f75863cb77c0f6aa26e0794a1848294e63c9d07d/UI/Web/src/assets/images/logo.svg), revision `f75863cb77c0f6aa26e0794a1848294e63c9d07d`. Downloaded from [the revision-pinned raw SVG](https://raw.githubusercontent.com/Kareadita/Kavita/f75863cb77c0f6aa26e0794a1848294e63c9d07d/UI/Web/src/assets/images/logo.svg).
- `public/assets/logos/komga-logo.svg`: [gotson/komga official web UI logo](https://github.com/gotson/komga/blob/519e352aa894b2fdc80660f1dde21b98cf328e3e/komga-webui/src/assets/logo.svg), revision `519e352aa894b2fdc80660f1dde21b98cf328e3e`. Downloaded from [the revision-pinned raw SVG](https://raw.githubusercontent.com/gotson/komga/519e352aa894b2fdc80660f1dde21b98cf328e3e/komga-webui/src/assets/logo.svg).

### Performance and interaction verification (2026-10-11)

- WOFF2 conversion preserved each font's character map and horizontal metrics.
- Live fonts, CSS, and versioned artwork return one-year immutable browser-cache headers; HTML retains revalidation. Browser network inspection observed cached font reuse on reload.
- Production layouts checked at 320, 390, 768, and 1440px without horizontal overflow.
- Reader page/scroll modes, Escape dismissal, focus restoration, and full artwork loading verified locally and in macOS Safari against the live site.
- Blocking the detailed artwork request preserved a readable preview and showed a retry message; reopening retried the request.
- Reduced-motion inspection confirmed no transform or transition on hovered/pressed actions.
- Build, typecheck, lint, formatting, and whitespace checks passed. No test files were added.

These checks do not measure font-swap timing on a physical iPhone or guarantee zero fallback-font frames on a first visit.
