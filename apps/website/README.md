# Chek website

The selected landing page for Chek, the ground-up successor to Chibi Manga: an e-reader for books and comics from local files or a preferred library server. The existing Chek logo is selected; later refinements may improve spacing and small-size clarity.

## Preview

From the repository root:

```sh
bun run dev:website
bun run build:website
```

Open http://127.0.0.1:18357/. Override with `PORT` if necessary. The development server binds to localhost. The static build is written to `apps/website/dist/`; deployment is not configured yet.

## Structure

- `src/`: HTML entry, reader interactions, styles, and Bun development server.
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

The WebP was encoded with `cwebp -q 88 -m 6 -sharp_yuv`. The original PNG is not retained in this repository.

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

## Provider logo provenance

The source chips use the existing Chek mark (`public/assets/logos/mark.svg`) and unmodified official provider SVGs saved locally on 2026-10-08. Logos retain their original colors; each sits beside its visible provider name.

- `public/assets/logos/kavita-logo.svg`: [Kareadita/Kavita official app logo](https://github.com/Kareadita/Kavita/blob/f75863cb77c0f6aa26e0794a1848294e63c9d07d/UI/Web/src/assets/images/logo.svg), revision `f75863cb77c0f6aa26e0794a1848294e63c9d07d`. Downloaded from [the revision-pinned raw SVG](https://raw.githubusercontent.com/Kareadita/Kavita/f75863cb77c0f6aa26e0794a1848294e63c9d07d/UI/Web/src/assets/images/logo.svg).
- `public/assets/logos/komga-logo.svg`: [gotson/komga official web UI logo](https://github.com/gotson/komga/blob/519e352aa894b2fdc80660f1dde21b98cf328e3e/komga-webui/src/assets/logo.svg), revision `519e352aa894b2fdc80660f1dde21b98cf328e3e`. Downloaded from [the revision-pinned raw SVG](https://raw.githubusercontent.com/gotson/komga/519e352aa894b2fdc80660f1dde21b98cf328e3e/komga-webui/src/assets/logo.svg).
