# Chek for iOS

Chek's native iOS app consumes the local `ChekReader` Swift package. The current
app imports local EPUB and CBZ files, retains owned copies, and reads reflowable chapters
with their local styles, images, and fonts. A contents sheet and previous/next
controls follow the book's navigation and spine. Reading resumes after close or
relaunch using a versioned DOM/text anchor, quote context, and progression fallback.
Text-size controls retain the passage through reflow and restore publisher size.

Fixed-layout XHTML/SVG pages fit their declared viewport into the reading area,
one spine item at a time. Mixed-layout overrides are retained. Text sizing applies
to reflowable content. The contents sheet also exposes landmarks, printed-page
navigation, and legacy guide/NCX entries when present. Standard IDPF-obfuscated
fonts are decoded on demand after archive integrity checks.

CBZ reading supports single-frame JPEG/PNG pages, numeric-aware archive ordering,
ComicInfo title/direction hints, native pinch zoom, and vertical scrolling within
long pages. Native page controls, a page-jump list, and a reading-direction menu
remain outside the page canvas. Page identity, within-page position, and direction
resume after reopening; zoom magnification itself is not persisted.

This is an early reader slice. Synthetic spreads, continuous EPUB roll layout,
Adobe font obfuscation, DRM, inline data resources, and audio/video are not
implemented yet. Comics currently display one image at a time; automatic movement
between vertically stacked pages is not implemented. Book scripts and
remote resources are disabled. EPUB archives are limited to
512 MB compressed/declared expanded data, 10,000 entries, and 32 MB per resource.
Every loaded entry is checked against its actual byte limit, size, and CRC.

The app stores originals and an atomic `library.json` index in Application Support.
Failed imports do not add library records. Imported copies are independent of the
source file provider. Per-book anchors and text size are stored atomically in
`Chek/Progress/<book UUID>.json` beside the library index. Comic progress uses
optional `comicLocation` and `comicDirection` fields; EPUB fields stay unchanged.
Library records without a format still open as EPUB, keeping their original IDs
and copies. Progress is captured
while scrolling and flushed on close and app inactivity/background transitions.
This is Chek's internal location format, not full EPUB CFI support. If a text
anchor cannot be resolved, reading falls back to the saved resource progression.
Text sizing changes the root and body font sizes; it preserves explicit sizes
inside publisher content. Device-only lifecycle and accessibility checks remain
separate from simulator verification.

Fixed pages need valid XHTML viewport dimensions or an SVG viewport/viewBox;
the reader currently limits either dimension to 16,384 and area to 32 million
CSS pixels. Malformed or excessive viewports produce a visible error. Wrong
obfuscation identifiers reject the affected font and report its failure; publisher
font fallback is not a successful font-fidelity check. See the
[package coverage](../../packages/reader/README.md) for format limits.

Comic decodes serialize and cancel obsolete work. The decoded cache is limited to
64 MiB, each decoded page to 16 million pixels, and source dimensions to 32,768
per axis / 80 million pixels. These are cache/decode limits, not a claim that total
app memory stays below 64 MiB: displayed images and ImageIO workspace also consume
memory. Unsupported formats and strips that cannot fit a readable decode within
these limits produce visible errors.

Requires Xcode 26 or later and iOS 26 or later. The checked-in Xcode project opens
directly in Xcode. To regenerate it with XcodeGen 2.44 or later:

```sh
xcodegen generate --spec apps/ios/project.yml
```

Build for the simulator from the repository root:

```sh
xcodebuild -project apps/ios/Chek.xcodeproj -scheme Chek \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chek-derived-data CODE_SIGNING_ALLOWED=NO build
```

Use Serve Sim with an explicit simulator device for interactive verification.
Device signing uses your own Apple development team configured in Xcode.
