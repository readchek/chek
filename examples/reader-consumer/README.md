# Reader consumer

A small Swift package that embeds EPUB and CBZ canvases using only the public
`ChekReader` product. Add this directory as a local package, link `ReaderConsumer`,
and use `ReaderContent(url:format:)` with an app-owned local file.

The library target is a canvas integration. Its host supplies close controls,
file import, saved locations, preferences, and library persistence. It contains
no sample books and has no dependency on `apps/ios`.

Build it independently with Xcode 27:

```sh
xcodebuild -scheme ReaderConsumer -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chek-package-derived-data CODE_SIGNING_ALLOWED=NO build
```

See [installation and embedding](../../docs/reader-package.md) for the ownership
and persistence contract, supported deployment target, and distribution status.

`ExampleApp` is a separate runnable host using only the public reader product.
It imports app-owned copies, opens EPUB/CBZ files from Documents, and saves
locations during reading (coalesced to once per second), on replacement, close,
and background. Autosave uses the reader’s captured location without requesting
another DOM capture. It deliberately keeps the same
canvas view mounted when replacing a reader session of the same format.

```sh
xcodegen generate --spec examples/reader-consumer/ExampleApp/project.yml
xcodebuild -project examples/reader-consumer/ExampleApp/ReaderEmbedding.xcodeproj \
  -scheme ReaderEmbedding -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chek-embedding-derived-data CODE_SIGNING_ALLOWED=NO build
```

The app bundle identifier is `dev.chek.reader.embedding`. Use **Files** to open
a publication; use the **Session** menu to import, replace, close, change the tap
callback, or toggle swipe navigation. EPUB text controls change scale; the CBZ
reading-mode menu chooses automatic, paged, or continuous layout. The host
shows session generation, readiness, location, and the callback used by the last
tap. These are ordinary embedding controls; the app does not run assertions or
automated checks. No publications are bundled.

The package also targets native macOS 27. Run the independent demo on Apple
Silicon without a phone or simulator from the repository root:

```sh
./script/build_and_run.sh
```

This builds the `ReaderEmbeddingMac` scheme and opens its `.app` bundle. Optional
`--debug`, `--logs`, `--telemetry`, and `--verify` modes support local diagnosis;
`--verify` checks process launch only, not rendered content or reading behavior.
The Mac demo uses `dev.chek.reader.embedding.mac` and retains its own imported
copies and saved locations. iPhone and iPad checks continue through Serve Sim.
