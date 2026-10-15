# Chek for iOS

Chek's native iOS app consumes the local `ChekReader` Swift package. The current
app imports local EPUB files, retains an owned copy, and reads reflowable chapters
with their local styles, images, and fonts. A contents sheet and previous/next
controls follow the book's navigation and spine. Reading resumes after close or
relaunch using a versioned DOM/text anchor, quote context, and progression fallback.
Text-size controls retain the passage through reflow and restore publisher size.

This is an early reader slice. Fixed-layout pages, font obfuscation, inline data
resources, audio/video, and CBZ reading are not implemented yet. Book scripts and
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
