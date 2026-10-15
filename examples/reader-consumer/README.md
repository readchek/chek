# Reader consumer

A small Swift package that embeds EPUB and CBZ canvases using only the public
`ChekReader` product. Add this directory as a local package, link `ReaderConsumer`,
and use `ReaderContent(url:format:)` with an app-owned local file.

The example is a canvas integration, not a complete app. Its host supplies close
controls, file import, saved locations, preferences, and library persistence.
It contains no sample books and has no dependency on `apps/ios`.

Build it independently with Xcode 27:

```sh
xcodebuild -scheme ReaderConsumer -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chek-package-derived-data CODE_SIGNING_ALLOWED=NO build
```

See [installation and embedding](../../docs/reader-package.md) for the ownership
and persistence contract, supported deployment target, and distribution status.
