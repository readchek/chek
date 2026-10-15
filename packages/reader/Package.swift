// swift-tools-version: 6.2

import PackageDescription

let package = Package(
  name: "ChekReader",
  platforms: [.iOS(.v26)],
  products: [
    .library(name: "ChekReader", targets: ["ChekReader"])
  ],
  targets: [
    .target(
      name: "ChekReader",
      swiftSettings: [
        .enableUpcomingFeature("NonisolatedNonsendingByDefault")
      ]
    )
  ],
  swiftLanguageModes: [.v6]
)
