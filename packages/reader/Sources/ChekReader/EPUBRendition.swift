import Foundation

struct FixedPageViewport: Sendable, Equatable {
  let width: Double
  let height: Double

  static func parse(_ data: Data, mediaType: String) throws -> FixedPageViewport {
    let root = try PublicationXML.parse(data)
    var width: Double?
    var height: Double?
    if mediaType == "image/svg+xml" {
      guard root.name == "svg", root.namespace == "http://www.w3.org/2000/svg" else {
        throw ReaderError.invalid("An SVG spine document has an invalid root element.")
      }
      width = absoluteLength(root.attributes["width"])
      height = absoluteLength(root.attributes["height"])
      let box = root.attributes["viewBox"]?.split(whereSeparator: { $0.isWhitespace || $0 == "," })
        .compactMap { Double($0) }
      if let box, box.count == 4 {
        if let knownWidth = width, height == nil {
          height = knownWidth * box[3] / box[2]
        } else if let knownHeight = height, width == nil {
          width = knownHeight * box[2] / box[3]
        } else {
          width = width ?? box[2]
          height = height ?? box[3]
        }
      }
    } else {
      guard
        let value = root.descendants("meta", namespace: "http://www.w3.org/1999/xhtml")
          .first(where: { $0.attributes["name"]?.lowercased() == "viewport" })?.attributes[
            "content"]
      else {
        throw ReaderError.unsupported("A fixed-layout page is missing its viewport dimensions.")
      }
      for pair in value.split(whereSeparator: { $0 == "," || $0 == ";" }) {
        let parts = pair.split(separator: "=", maxSplits: 1).map {
          $0.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        guard parts.count == 2 else { continue }
        if parts[0].lowercased() == "width" { width = Double(parts[1]) }
        if parts[0].lowercased() == "height" { height = Double(parts[1]) }
      }
    }
    guard let width, let height, width.isFinite, height.isFinite,
      width > 0, height > 0, width <= 16_384, height <= 16_384, width * height <= 32_000_000
    else {
      throw ReaderError.unsupported(
        "A fixed-layout page has missing, invalid, or excessive viewport dimensions.")
    }
    return FixedPageViewport(width: width, height: height)
  }

  private static func absoluteLength(_ value: String?) -> Double? {
    guard let value else { return nil }
    let scanner = Scanner(string: value)
    guard let number = scanner.scanDouble() else { return nil }
    let unit = String(value[scanner.currentIndex...]).trimmingCharacters(
      in: .whitespacesAndNewlines
    ).lowercased()
    let multiplier: Double
    switch unit {
    case "", "px": multiplier = 1
    case "pt": multiplier = 96 / 72
    case "pc": multiplier = 16
    case "in": multiplier = 96
    case "cm": multiplier = 96 / 2.54
    case "mm": multiplier = 96 / 25.4
    default: return nil
    }
    return number * multiplier
  }
}
