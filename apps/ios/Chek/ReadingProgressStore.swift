import ChekReader
import Foundation
import Observation

nonisolated struct BookReadingProgress: Codable, Sendable {
  var location: ReadingLocation?
  var textScale = 1.0
}

@Observable final class ReadingProgressStore {
  private let id: UUID
  private var progress: BookReadingProgress
  private var pending: Task<Void, Never>?
  var errorMessage: String?

  init(id: UUID, progress: BookReadingProgress) {
    self.id = id
    self.progress = progress
  }

  func update(location: ReadingLocation?, textScale: Double) {
    if let location { progress.location = location }
    progress.textScale = textScale
    pending?.cancel()
    pending = Task {
      do {
        try await Task.sleep(for: .milliseconds(400))
        try Task.checkCancellation()
        try await ReadingProgressDisk.shared.save(progress, id: id)
      } catch is CancellationError {
      } catch { errorMessage = error.localizedDescription }
    }
  }

  func flush() async throws {
    pending?.cancel()
    pending = nil
    try await ReadingProgressDisk.shared.save(progress, id: id)
  }
}

actor ReadingProgressDisk {
  static let shared = ReadingProgressDisk()

  private func fileURL(id: UUID) -> URL {
    URL.applicationSupportDirectory.appending(path: "Chek/Progress", directoryHint: .isDirectory)
      .appending(path: id.uuidString + ".json")
  }

  func load(id: UUID) throws -> BookReadingProgress {
    let url = fileURL(id: id)
    guard FileManager.default.fileExists(atPath: url.path) else { return BookReadingProgress() }
    let values = try url.resourceValues(forKeys: [.fileSizeKey])
    guard let size = values.fileSize, size <= 16 * 1024 else {
      throw ReaderError.invalid("Saved reading progress is too large.")
    }
    return try JSONDecoder().decode(BookReadingProgress.self, from: Data(contentsOf: url))
  }

  func save(_ progress: BookReadingProgress, id: UUID) throws {
    try Task.checkCancellation()
    let url = fileURL(id: id)
    try FileManager.default.createDirectory(
      at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try JSONEncoder().encode(progress).write(to: url, options: .atomic)
  }
}
