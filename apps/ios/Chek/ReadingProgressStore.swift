import ChekReader
import Foundation
import Observation

nonisolated struct BookReadingProgress: Codable, Sendable {
  var location: ReadingLocation?
  var textScale = 1.0
  var comicLocation: CBZLocation?
  var comicDirection: CBZReadingDirection?
}

nonisolated struct LoadedReadingProgress: Sendable {
  let progress: BookReadingProgress
  let warning: String?
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
    scheduleSave()
  }

  func update(comicLocation: CBZLocation, readingDirection: CBZReadingDirection) {
    progress.comicLocation = comicLocation
    progress.comicDirection = readingDirection
    scheduleSave()
  }

  private func scheduleSave() {
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

  func load(id: UUID) throws -> LoadedReadingProgress {
    let url = fileURL(id: id)
    guard FileManager.default.fileExists(atPath: url.path) else {
      return LoadedReadingProgress(progress: BookReadingProgress(), warning: nil)
    }
    let values = try url.resourceValues(forKeys: [.fileSizeKey])
    guard let size = values.fileSize, size <= 16 * 1024 else {
      return try preserveDamagedProgress(at: url)
    }
    let data = try Data(contentsOf: url)
    do {
      let progress = try JSONDecoder().decode(BookReadingProgress.self, from: data)
      return LoadedReadingProgress(progress: progress, warning: nil)
    } catch is DecodingError {
      return try preserveDamagedProgress(at: url)
    }
  }

  private func preserveDamagedProgress(at url: URL) throws -> LoadedReadingProgress {
    let preserved = url.deletingPathExtension().appendingPathExtension(
      "damaged-" + UUID().uuidString + ".json")
    do { try FileManager.default.moveItem(at: url, to: preserved) } catch {
      throw ReaderError.invalid(
        "Saved reading progress could not be read or preserved. Opening stopped so the existing file stays intact."
      )
    }
    return LoadedReadingProgress(
      progress: BookReadingProgress(),
      warning: String(
        localized:
          "Saved reading progress could not be read. The previous file was preserved separately. This book starts at the beginning."
      ))
  }

  func save(_ progress: BookReadingProgress, id: UUID) throws {
    try Task.checkCancellation()
    let url = fileURL(id: id)
    try FileManager.default.createDirectory(
      at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try JSONEncoder().encode(progress).write(to: url, options: .atomic)
  }
}
