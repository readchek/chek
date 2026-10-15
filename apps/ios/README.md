# Chek for iOS

Chek's native iOS app consumes the local `ChekReader` Swift package. The current
app imports local EPUB files, retains an owned copy, and reads reflowable chapters
with their local styles, images, and fonts. A contents sheet and previous/next
controls follow the book's navigation and spine. Reading resumes after close or
relaunch using a versioned DOM/text anchor, quote context, and progression fallback.
Text-size controls retain the passage through reflow and restore publisher size.

Fixed-layout XHTML/SVG pages fit their declared viewport into the reading area,
one spine item at a time. Mixed-layout overrides are retained. Text sizing applies
to reflowable content. The contents sheet also exposes landmarks, printed-page
navigation, and legacy guide/NCX entries when present. Standard IDPF-obfuscated
fonts are decoded on demand after archive integrity checks.

This is an early reader slice. Synthetic spreads, continuous roll layout, Adobe
font obfuscation, DRM, inline data resources, audio/video, and CBZ app reading are
not implemented yet. Book scripts and
remote resources are disabled. EPUB archives are limited to
512 MB compressed/declared expanded data, 10,000 entries, and 32 MB per resource.
Every loaded entry is checked against its actual byte limit, size, and CRC.

The app stores originals and an atomic `library.json` index in Application Support.
Failed imports do not add library records. Imported copies are independent of the
source file provider. Per-book anchors and text size are stored atomically in
`Chek/Progress/<book UUID>.json` beside the library index. Progress is captured
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
