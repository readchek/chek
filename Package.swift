// swift-tools-version: 6.4

import PackageDescription

let package = Package(
  name: "ChekReader",
  platforms: [.iOS(.v27), .macOS(.v27)],
  products: [
    .library(name: "ChekReader", targets: ["ChekReader"])
  ],
  dependencies: [
    .package(url: "https://github.com/weichsel/ZIPFoundation.git", exact: "0.9.20")
  ],
  targets: [
    .target(
      name: "ChekReader",
      dependencies: ["ZIPFoundation"],
      path: "packages/reader/Sources/ChekReader",
      resources: [.process("Resources")],
      swiftSettings: [
        .enableUpcomingFeature("NonisolatedNonsendingByDefault")
      ]
    )
  ],
  swiftLanguageModes: [.v6]
)
