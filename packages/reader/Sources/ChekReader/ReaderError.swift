import Foundation

public enum ReaderError: LocalizedError, Sendable {
  case invalid(String)
  case unsupported(String)
  case limit(String)

  public var errorDescription: String? {
    switch self {
    case .invalid(let reason): "This EPUB could not be opened. \(reason)"
    case .unsupported(let reason): reason
    case .limit(let reason): "This book exceeds the current reader limits. \(reason)"
    }
  }
}
