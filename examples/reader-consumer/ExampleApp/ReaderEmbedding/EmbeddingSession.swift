import ChekReader
import Foundation
import Observation

/// Host-owned state. Reader instances can change without removing the canvas.
@MainActor @Observable
final class EmbeddingSession {
  enum Reader {
    case epub(EPUBReader)
    case cbz(CBZReader)
  }

  var reader: Reader?
  var files: [URL] = []
  var filename = "No publication"
  var error: String?
  var isOpening = false
  var generation = 0
  var tapCount = 0
  var callbackVersion = 1
  var lastCallbackVersion: Int?
  var allowsSwipes = true {
    didSet {
      switch reader {
      case .epub(let reader): reader.allowsChapterSwipe = allowsSwipes
      case .cbz(let reader): reader.allowsPageSwipe = allowsSwipes
      case nil: break
      }
    }
  }
  @ObservationIgnored private var operation: Task<Void, Never>?
  @ObservationIgnored private var request = UUID()
  @ObservationIgnored private var currentURL: URL?

  func refreshFiles() {
    do {
      files = try FileManager.default.contentsOfDirectory(
        at: .documentsDirectory, includingPropertiesForKeys: nil,
        options: [.skipsHiddenFiles]
      ).filter { ["epub", "cbz"].contains($0.pathExtension.lowercased()) }
        .sorted {
          $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending
        }
    } catch {
      self.error = error.localizedDescription
    }
  }

  func importFile(_ source: URL) {
    begin { [self] in
      let copied = try await Self.copyImport(source)
      try Task.checkCancellation()
      refreshFiles()
      return try await makeReader(for: copied)
    }
  }

  func open(_ url: URL) {
    begin { [self] in try await makeReader(for: url) }
  }

  /// Reuses the publication but constructs a new session in the same view branch.
  func replace() {
    guard let reader, let currentURL else { return }
    begin { [self] in
      switch reader {
      case .epub(let previous):
        let location = await previous.captureLocation()
        return (
          .epub(
            EPUBReader(
              publication: previous.publication, location: location,
              textScale: previous.textScale, allowsChapterSwipe: allowsSwipes)), currentURL
        )
      case .cbz(let previous):
        return (
          .cbz(
            CBZReader(
              publication: previous.publication, location: previous.captureLocation(),
              direction: previous.readingDirection, readingMode: previous.readingMode,
              allowsPageSwipe: allowsSwipes)), currentURL
        )
      }
    }
  }

  func close() {
    operation?.cancel()
    let token = UUID()
    request = token
    let previous = reader
    let url = currentURL
    isOpening = false
    operation = Task { [weak self] in
      guard let self else { return }
      await save(previous, url: url)
      guard request == token else { return }
      reader = nil
      currentURL = nil
      filename = "No publication"
    }
  }

  func save() async { await save(reader, url: currentURL) }

  private func begin(_ open: @escaping @MainActor () async throws -> (Reader, URL)) {
    operation?.cancel()
    let token = UUID()
    request = token
    isOpening = true
    error = nil
    operation = Task { [weak self] in
      guard let self else { return }
      do {
        await save()
        try Task.checkCancellation()
        let (opened, url) = try await open()
        try Task.checkCancellation()
        guard request == token else { return }
        reader = opened
        currentURL = url
        filename = url.lastPathComponent
        generation += 1
      } catch is CancellationError {
      } catch {
        if request == token { self.error = error.localizedDescription }
      }
      if request == token { isOpening = false }
    }
  }

  private func makeReader(for url: URL) async throws -> (Reader, URL) {
    let saved = UserDefaults.standard.data(forKey: url.lastPathComponent)
    switch url.pathExtension.lowercased() {
    case "epub":
      let publication = try await EPUBPublication.open(url)
      let location = saved.flatMap { try? JSONDecoder().decode(ReadingLocation.self, from: $0) }
      return (
        .epub(
          EPUBReader(
            publication: publication, location: location, allowsChapterSwipe: allowsSwipes)), url
      )
    case "cbz":
      let publication = try await CBZPublication.open(url)
      let location = saved.flatMap { try? JSONDecoder().decode(CBZLocation.self, from: $0) }
      return (
        .cbz(
          CBZReader(
            publication: publication, location: location, allowsPageSwipe: allowsSwipes)), url
      )
    default:
      throw CocoaError(.fileReadUnsupportedScheme)
    }
  }

  private func save(_ reader: Reader?, url: URL?) async {
    guard let url else { return }
    let encoded: Data?
    switch reader {
    case .epub(let reader):
      encoded = await reader.captureLocation().flatMap { try? JSONEncoder().encode($0) }
    case .cbz(let reader): encoded = try? JSONEncoder().encode(reader.captureLocation())
    case nil: return
    }
    if let encoded { UserDefaults.standard.set(encoded, forKey: url.lastPathComponent) }
  }

  @concurrent private static func copyImport(_ source: URL) async throws -> URL {
    let scoped = source.startAccessingSecurityScopedResource()
    defer { if scoped { source.stopAccessingSecurityScopedResource() } }
    let destination = URL.documentsDirectory.appendingPathComponent(
      "\(UUID().uuidString)-\(source.lastPathComponent)")
    try FileManager.default.copyItem(at: source, to: destination)
    return destination
  }
}
