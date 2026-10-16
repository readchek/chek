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

  struct ActiveSession {
    let id = UUID()
    let reader: Reader
    let url: URL
  }

  /// The previous canvas stays mounted while a replacement opens or saves on
  /// close. A failed replacement returns to that valid session with an advisory.
  enum State {
    case idle
    case opening(previous: ActiveSession?)
    case reading(ActiveSession)
    case closing(ActiveSession?)

    var activeSession: ActiveSession? {
      switch self {
      case .idle: nil
      case .opening(let previous), .closing(let previous): previous
      case .reading(let session): session
      }
    }
  }

  private(set) var state: State = .idle
  var reader: Reader? { state.activeSession?.reader }
  var filename: String { currentURL?.lastPathComponent ?? "No publication" }
  var isOpening: Bool {
    if case .opening = state { true } else { false }
  }
  var files: [URL] = []
  var error: String?
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
  @ObservationIgnored private var autosave: Task<Void, Never>?
  @ObservationIgnored private var request = UUID()
  private var currentURL: URL? { state.activeSession?.url }

  nonisolated private static var originalsDirectory: URL {
    #if os(macOS)
      URL.applicationSupportDirectory.appending(
        path: "ReaderEmbedding/Originals", directoryHint: .isDirectory)
    #else
      .documentsDirectory
    #endif
  }

  func refreshFiles() {
    do {
      try FileManager.default.createDirectory(
        at: Self.originalsDirectory, withIntermediateDirectories: true)
      files = try FileManager.default.contentsOfDirectory(
        at: Self.originalsDirectory, includingPropertiesForKeys: nil,
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
    cancelAutosave()
    operation?.cancel()
    let token = UUID()
    request = token
    let previous = state.activeSession
    state = .closing(previous)
    operation = Task { [weak self] in
      guard let self else { return }
      await save(previous)
      guard !Task.isCancelled, request == token else { return }
      state = .idle
      operation = nil
    }
  }

  func save() async { await save(state.activeSession) }

  /// Coalesce progress updates without asking the canvas to capture again.
  /// A fixed interval also saves during a continuous comic scroll.
  func scheduleAutosave() {
    guard autosave == nil, case .reading(let session) = state else { return }
    let token = request
    autosave = Task { [weak self] in
      do {
        try await Task.sleep(for: .seconds(1))
      } catch { return }
      guard !Task.isCancelled, let self, request == token,
        case .reading(let current) = state, current.id == session.id
      else { return }
      autosave = nil
      switch session.reader {
      case .epub(let reader): persist(reader.location, url: session.url)
      case .cbz(let reader): persist(reader.location, url: session.url)
      }
    }
  }

  private func cancelAutosave() {
    autosave?.cancel()
    autosave = nil
  }

  private func begin(_ open: @escaping @MainActor () async throws -> (Reader, URL)) {
    cancelAutosave()
    operation?.cancel()
    let token = UUID()
    request = token
    let previous = state.activeSession
    state = .opening(previous: previous)
    error = nil
    operation = Task { [weak self] in
      guard let self else { return }
      defer {
        if request == token {
          operation = nil
          // Cancellation or failure cannot leave an opening spinner behind.
          if case .opening = state {
            state = previous.map(State.reading) ?? .idle
          }
        }
      }
      do {
        await save(previous)
        try Task.checkCancellation()
        let (opened, url) = try await open()
        try Task.checkCancellation()
        guard request == token else { return }
        state = .reading(ActiveSession(reader: opened, url: url))
        generation += 1
      } catch is CancellationError {
      } catch {
        if request == token { self.error = error.localizedDescription }
      }
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

  private func save(_ session: ActiveSession?) async {
    guard let session, state.activeSession?.id == session.id else { return }
    switch session.reader {
    case .epub(let reader):
      let location = await reader.captureLocation()
      // Background capture can finish after a replacement has been mounted.
      guard !Task.isCancelled, state.activeSession?.id == session.id else { return }
      persist(location, url: session.url)
    case .cbz(let reader): persist(reader.captureLocation(), url: session.url)
    }
  }

  private func persist<Location: Encodable>(_ location: Location?, url: URL) {
    let encoded = location.flatMap { try? JSONEncoder().encode($0) }
    if let encoded { UserDefaults.standard.set(encoded, forKey: url.lastPathComponent) }
  }

  @concurrent private static func copyImport(_ source: URL) async throws -> URL {
    let scoped = source.startAccessingSecurityScopedResource()
    defer { if scoped { source.stopAccessingSecurityScopedResource() } }
    try FileManager.default.createDirectory(
      at: originalsDirectory, withIntermediateDirectories: true)
    let destination = originalsDirectory.appendingPathComponent(
      "\(UUID().uuidString)-\(source.lastPathComponent)")
    try FileManager.default.copyItem(at: source, to: destination)
    return destination
  }
}
