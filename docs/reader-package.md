# Installing and embedding ChekReader

ChekReader is an iOS 27 Swift package built with Swift 6.4. Its only external
dependency is ZIPFoundation 0.9.20. Bun, the API, website, and Chek app are not
dependencies of the reader product.

## Installation

For internal development, add this repository root as a local package in Xcode
and link the `ChekReader` product. A Swift package consumer can use:

```swift
dependencies: [
  .package(name: "ChekReader", path: "/path/to/chek")
],
targets: [
  .target(
    name: "YourReaderFeature",
    dependencies: [.product(name: "ChekReader", package: "ChekReader")]
  )
]
```

The root `Package.swift` is also the entry point for Git-based SwiftPM installs.
Once the packaging commit is pushed to an accessible repository, pin that exact
revision of `https://github.com/chibi-app/chek.git`, or use an approved semantic
version tag when one exists. No release or version tag is implied by this guide.
SwiftPM cannot install a nested package using a repository URL plus subdirectory.

`packages/reader/Package.swift` remains available for a standalone copy of that
directory. Both manifests build the same source and resources; keep their platform,
dependency, language, and compiler settings aligned. Do not add both to one graph.

## Embedding

Import `ChekReader`, open `EPUBPublication` or `CBZPublication` asynchronously,
then retain its `EPUBReader` or `CBZReader` in SwiftUI state. Render the matching
`EPUBReaderView` or `CBZReaderView`. The views contain the reading canvas and
recoverable loading UI; the host supplies its own surrounding navigation.

See the [independent consumer](../examples/reader-consumer/) for a minimal library
view and a separate runnable embedding app. Both import only the public reader
product. The example app demonstrates host-owned import, persistence, callbacks,
and session replacement without depending on the Chek app.

The host must keep an app-owned source file accessible while its publication is
open. File picking, security-scoped access, imported copies, publication identity,
library storage, and persistence policy belong to the host. Opening a file does
not copy it into reader-owned storage.

Pass a saved `ReadingLocation` or `CBZLocation` to the reader initializer. Observe
`location` during reading and encode it with `JSONEncoder`. Before close or
background persistence, call `captureLocation()`; the EPUB version is asynchronous.
Store preferences alongside the location, including comic direction and mode.
Validate decoded locations with `isValid` before use. The host decides how to
recover from invalid or incompatible persisted data.

Use the reader's public navigation and preference methods for host controls.
`onTap` and `onInteraction` on each canvas let the host control chrome visibility.
Keep an exit control available while `isReadyForInteraction` is false. Native page
content and its own accessibility remain distinct from host navigation controls.

For EPUB, `allowsChapterSwipe` controls whether the reader handles horizontal
chapter-swipe callbacks. Set it in the initializer or on the retained reader when
the host provides another chapter-navigation policy. This does not disable native
scrolling, text selection, or tap/interaction callbacks. Observe `loadFailed` and
`failureDescription` for host error presentation, and call `retry()` to reload
after a failed page load. The comic canvas presents its own page-load failure UI
and exposes `retry()` and read-only `failureDescription`. Set `allowsPageSwipe`
to false when the host owns horizontal page gestures; continuous vertical
scrolling remains available.

Keep one reader instance per mounted canvas. Publications may be opened by
multiple independent readers, but navigation state and capture callbacks belong
to a single reading session. Replacing a reader in a persistent SwiftUI host
detaches the old canvas state and binds the new session.

## Building independently

From `examples/reader-consumer`, build the public consumer without the app:

```sh
xcodebuild -scheme ReaderConsumer -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/chek-package-derived-data CODE_SIGNING_ALLOWED=NO build
```

The manifest includes the reader JavaScript, localized strings, and third-party
notice as processed resources. Runtime resource lookup uses `Bundle.module`;
consumers should not copy reader resources into the host bundle manually.

## Distribution status

The source package supports internal local installation. Public release still
needs an explicit license for ChekReader's own source, an approved version tag,
and the documented performance/reliability verification. The repository currently
has no project license; the bundled MIT notice covers ZIPFoundation only. Do not
interpret it as a license for ChekReader.

Current format coverage and resource limits are listed in the
[package README](../packages/reader/README.md). Installation does not imply full
EPUB conformance, release readiness, or verified performance on every device.
See the [performance and reliability record](reader-package-validation.md) for
measured evidence and the remaining release checks.
