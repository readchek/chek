import Foundation

public enum CBZReadingDirection: String, Codable, Sendable {
  case leftToRight
  case rightToLeft
}

public enum CBZReadingMode: String, Codable, Sendable {
  case automatic
  case paged
  case continuous
}

public struct CBZPage: Identifiable, Hashable, Sendable {
  /// The literal, validated archive entry path is stable across opens.
  public let id: String
}

public struct CBZLocation: Codable, Equatable, Sendable {
  public let version: Int
  public let pageID: String
  /// Position within this page, independent of decoded size or viewport width.
  public let progression: Double

  public init(pageID: String, progression: Double = 0) {
    version = 1
    self.pageID = pageID
    self.progression = progression.isFinite ? min(1, max(0, progression)) : 0
  }

  public var isValid: Bool {
    version == 1 && !pageID.isEmpty && pageID.utf8.count <= 4096
      && progression.isFinite && (0...1).contains(progression)
  }
}

public struct CBZPublication: Sendable {
  public let title: String
  public let pages: [CBZPage]
  public let readingDirection: CBZReadingDirection
  let archive: ArchiveStore
  private let pageIndices: [String: Int]

  /// Indexes names and optional metadata; page bytes are read only when requested.
  @concurrent public static func open(_ url: URL, fallbackTitle: String? = nil) async throws
    -> CBZPublication
  {
    let archive = try ArchiveStore(url: url)
    let paths = await archive.paths()
    let contentPaths = paths.filter { path in
      let pieces = path.split(separator: "/")
      return !pieces.contains(where: { $0.hasPrefix(".") || $0 == "__MACOSX" })
    }
    let unsupportedImages: Set<String> = [
      "gif", "webp", "avif", "heic", "heif", "tif", "tiff", "bmp", "jxl", "jp2", "jpf", "jpx",
      "jpm", "mj2", "svg", "apng",
    ]
    if contentPaths.contains(where: {
      unsupportedImages.contains(($0 as NSString).pathExtension.lowercased())
    }) {
      throw ReaderError.unsupported(
        "This CBZ includes unsupported image pages. Comic reading currently supports JPEG and PNG only; no pages were imported."
      )
    }
    let imagePaths = contentPaths.filter {
      ["jpg", "jpeg", "png"].contains(($0 as NSString).pathExtension.lowercased())
    }.sorted { first, second in
      let order = first.compare(
        second, options: [.numeric, .caseInsensitive], locale: Locale(identifier: "en_US_POSIX"))
      return order == .orderedSame
        ? first.utf8.lexicographicallyPrecedes(second.utf8) : order == .orderedAscending
    }
    guard !imagePaths.isEmpty else {
      throw ReaderError.unsupported("This CBZ has no JPEG or PNG pages.")
    }
    let providedTitle = fallbackTitle?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    var title =
      providedTitle.isEmpty ? url.deletingPathExtension().lastPathComponent : providedTitle
    var direction = CBZReadingDirection.leftToRight
    if let infoPath = paths.first(where: { $0.lowercased() == "comicinfo.xml" }) {
      let metadata = try PublicationXML.parse(
        await archive.data(at: infoPath, maximumBytes: 1024 * 1024))
      if let value = metadata.children.first(where: { $0.name == "Title" })?.value,
        !value.isEmpty
      {
        title = value
      }
      if metadata.children.first(where: { $0.name == "Manga" })?.value.lowercased()
        == "yesandrighttoleft"
      {
        direction = .rightToLeft
      }
    }
    return CBZPublication(
      title: title, pages: imagePaths.map { CBZPage(id: $0) }, readingDirection: direction,
      archive: archive,
      pageIndices: Dictionary(uniqueKeysWithValues: imagePaths.enumerated().map { ($1, $0) }))
  }

  public func pageIndex(for location: CBZLocation) -> Int? {
    guard location.isValid else { return nil }
    return pageIndices[location.pageID]
  }
}
