// swift-tools-version: 6.4

import PackageDescription

let package = Package(
  name: "ReaderConsumer",
  platforms: [.iOS(.v27)],
  products: [
    .library(name: "ReaderConsumer", targets: ["ReaderConsumer"])
  ],
  dependencies: [
    .package(name: "ChekReader", path: "../..")
  ],
  targets: [
    .target(
      name: "ReaderConsumer",
      dependencies: [.product(name: "ChekReader", package: "ChekReader")]
    )
  ],
  swiftLanguageModes: [.v6]
)
