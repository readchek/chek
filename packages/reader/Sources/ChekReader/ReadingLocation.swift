import Foundation

/// Chek's own versioned anchor, not an implementation of EPUB CFI.
public struct ReadingLocation: Codable, Equatable, Sendable {
  public let version: Int
  /// Canonically percent-encoded, archive-relative resource href without a fragment.
  public let resourceHref: String
  public let nodePath: [Int]
  public let textOffset: Int
  public let quote: String
  public let prefix: String
  public let suffix: String
  public let progression: Double

  /// Whether this location has a supported version and bounded anchor values.
  /// Publication-specific resource validation still occurs when a reader opens it.
  public var isValid: Bool {
    version == 1 && !resourceHref.isEmpty && resourceHref.utf8.count <= 4096
      && nodePath.count <= 128 && nodePath.allSatisfy { (0...1_000_000).contains($0) }
      && (0...32 * 1024 * 1024).contains(textOffset)
      && quote.utf16.count <= 64 && prefix.utf16.count <= 32 && suffix.utf16.count <= 32
      && progression.isFinite && (0...1).contains(progression)
  }
}
