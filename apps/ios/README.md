# Chek for iOS

Chek's native iOS app consumes the local `ChekReader` Swift package. The current
foundation shows an empty local library. Import and reading are in development.

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
