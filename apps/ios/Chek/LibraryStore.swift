import ChekReader
import Foundation
import Observation

nonisolated struct LibraryBook: Codable, Identifiable, Sendable {
  let id: UUID
  let title: String
  let author: String
  let importedAt: Date
  let format: PublicationFormat

  private enum CodingKeys: String, CodingKey { case id, title, author, importedAt, format }

  init(id: UUID, title: String, author: String, importedAt: Date, format: PublicationFormat) {
    self.id = id
    self.title = title
    self.author = author
    self.importedAt = importedAt
    self.format = format
  }

  init(from decoder: any Decoder) throws {
    let values = try decoder.container(keyedBy: CodingKeys.self)
    id = try values.decode(UUID.self, forKey: .id)
    title = try values.decode(String.self, forKey: .title)
    author = try values.decode(String.self, forKey: .author)
    importedAt = try values.decode(Date.self, forKey: .importedAt)
    // Older library records contain EPUBs only. Keep their IDs and original paths.
    format = try values.decodeIfPresent(PublicationFormat.self, forKey: .format) ?? .epub
  }
}

nonisolated enum OpenedPublication: Sendable {
  case epub(EPUBPublication)
  case cbz(CBZPublication)

  var title: String {
    switch self {
    case .epub(let book): book.title
    case .cbz(let comic): comic.title
    }
  }

  var author: String {
    switch self {
    case .epub(let book): book.author
    case .cbz: ""
    }
  }

  static func open(_ url: URL, format: PublicationFormat, title: String) async throws
    -> OpenedPublication
  {
    switch format {
    case .epub: .epub(try await EPUBPublication.open(url))
    case .cbz: .cbz(try await CBZPublication.open(url, fallbackTitle: title))
    }
  }
}

struct OpenBook: Identifiable {
  let id: UUID
  let publication: OpenedPublication
  let progress: BookReadingProgress
  let progressWarning: String?
}

@Observable final class LibraryStore {
  private(set) var books: [LibraryBook] = []
  private(set) var isBusy = false
  var openBook: OpenBook?
  var message: String?
  private var loaded = false
  private var loading: (id: UUID, task: Task<[LibraryBook], any Error>)?

  var hasMessage: Bool {
    get { message != nil }
    set { if !newValue { message = nil } }
  }

  func load() async {
    do { try await ensureLoaded() } catch { message = error.localizedDescription }
  }

  private func ensureLoaded() async throws {
    guard !loaded else { return }
    if loading == nil { loading = (UUID(), Task { try await LibraryDisk.load() }) }
    guard let request = loading else { return }
    let records: [LibraryBook]
    do { records = try await request.task.value } catch {
      // An older failed waiter must not clear a newer retry's task.
      if loading?.id == request.id { loading = nil }
      throw error
    }
    if !loaded {
      books = records
      loaded = true
    }
  }

  func importBook(_ url: URL) async {
    guard !isBusy else {
      message = String(localized: "Wait for the current book to finish opening.")
      return
    }
    isBusy = true
    defer { isBusy = false }
    var staged: URL?
    do {
      try await ensureLoaded()
      guard url.isFileURL, let format = PublicationFormat(rawValue: url.pathExtension.lowercased())
      else {
        throw ReaderError.unsupported(
          "Choose a local EPUB or CBZ file.")
      }
      let id = UUID()
      let owned = try await LibraryDisk.stage(url, id: id, format: format)
      staged = owned
      let publication = try await OpenedPublication.open(
        owned, format: format,
        title: url.deletingPathExtension().lastPathComponent)
      let record = LibraryBook(
        id: id, title: publication.title, author: publication.author, importedAt: Date(),
        format: format)
      let updated = books + [record]
      try await LibraryDisk.save(updated)
      books = updated
      staged = nil
      openBook = OpenBook(
        id: id, publication: publication, progress: BookReadingProgress(), progressWarning: nil)
    } catch {
      if let staged { await LibraryDisk.removeStaged(staged) }
      message = error.localizedDescription
    }
  }

  func open(_ book: LibraryBook) async {
    guard !isBusy else { return }
    isBusy = true
    defer { isBusy = false }
    do {
      let publication = try await OpenedPublication.open(
        LibraryDisk.originalURL(id: book.id, format: book.format),
        format: book.format, title: book.title)
      let progress = try await ReadingProgressDisk.shared.load(id: book.id)
      openBook = OpenBook(
        id: book.id, publication: publication, progress: progress.progress,
        progressWarning: progress.warning)
    } catch { message = error.localizedDescription }
  }
}

private nonisolated enum LibraryDisk {
  static var directory: URL {
    URL.applicationSupportDirectory.appending(path: "Chek", directoryHint: .isDirectory)
  }

  static func originalURL(id: UUID, format: PublicationFormat) -> URL {
    directory.appending(path: "Originals", directoryHint: .isDirectory).appending(
      path: id.uuidString + "." + format.rawValue)
  }

  @concurrent static func load() async throws -> [LibraryBook] {
    let index = directory.appending(path: "library.json")
    guard FileManager.default.fileExists(atPath: index.path) else { return [] }
    return try JSONDecoder().decode([LibraryBook].self, from: Data(contentsOf: index))
  }

  @concurrent static func save(_ books: [LibraryBook]) async throws {
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    try JSONEncoder().encode(books).write(
      to: directory.appending(path: "library.json"), options: .atomic)
  }

  @concurrent static func removeStaged(_ url: URL) async {
    try? FileManager.default.removeItem(at: url)
  }

  @concurrent static func stage(_ source: URL, id: UUID, format: PublicationFormat) async throws
    -> URL
  {
    let scoped = source.startAccessingSecurityScopedResource()
    defer { if scoped { source.stopAccessingSecurityScopedResource() } }
    let destination = originalURL(id: id, format: format)
    try FileManager.default.createDirectory(
      at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
    var coordinationError: NSError?
    var copyError: (any Error)?
    let coordinator = NSFileCoordinator()
    coordinator.coordinate(
      readingItemAt: source, options: .withoutChanges, error: &coordinationError
    ) { input in
      do {
        let values = try input.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
        guard values.isRegularFile == true, values.isSymbolicLink != true else {
          throw ReaderError.invalid("Choose a regular book or comic file.")
        }
        let reader = try FileHandle(forReadingFrom: input)
        defer { try? reader.close() }
        guard FileManager.default.createFile(atPath: destination.path, contents: nil) else {
          throw ReaderError.invalid("The imported copy could not be created.")
        }
        let writer = try FileHandle(forWritingTo: destination)
        defer { try? writer.close() }
        var bytes = 0
        while let chunk = try reader.read(upToCount: 64 * 1024), !chunk.isEmpty {
          try Task.checkCancellation()
          bytes += chunk.count
          guard bytes <= 512 * 1024 * 1024 else {
            throw ReaderError.limit("The book exceeds 512 MB.")
          }
          try writer.write(contentsOf: chunk)
        }
        try writer.synchronize()
      } catch { copyError = error }
    }
    if let error = coordinationError ?? copyError {
      try? FileManager.default.removeItem(at: destination)
      throw error
    }
    return destination
  }
}
