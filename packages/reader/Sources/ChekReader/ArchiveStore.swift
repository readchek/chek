import Foundation
import ImageIO
import ZIPFoundation

/// Archive access is serialized because ZIPFoundation shares a seekable file handle.
actor ArchiveStore {
  static let maximumFileBytes = 512 * 1024 * 1024
  static let maximumResourceBytes = 32 * 1024 * 1024
  private let archive: Archive
  private let entries: [String: Entry]

  init(url: URL) throws {
    let count = try ZIPEnvelope.validate(url: url)
    let archive = try Archive(url: url, accessMode: .read)
    var entries: [String: Entry] = [:]
    var names: Set<String> = []
    var expanded = 0
    var visited = 0
    for entry in archive {
      try Task.checkCancellation()
      visited += 1
      guard visited <= 10_000, entry.type != .symlink else {
        throw ReaderError.invalid("The archive contains too many entries or a symbolic link.")
      }
      let path = try Self.canonicalPath(entry.path, directory: entry.type == .directory)
      guard Self.pathAliases(path).allSatisfy({ names.insert($0.lowercased()).inserted }) else {
        throw ReaderError.invalid("The archive contains ambiguous duplicate paths.")
      }
      guard entry.uncompressedSize <= Self.maximumResourceBytes else {
        throw ReaderError.limit("An archive entry is larger than 32 MB.")
      }
      if entry.type == .file {
        // Preflight aggregate declarations; reads independently enforce actual bytes and CRC.
        expanded += Int(entry.uncompressedSize)
        guard expanded <= Self.maximumFileBytes else {
          throw ReaderError.limit("Declared expanded book data exceeds 512 MB.")
        }
        entries[path] = entry
      }
    }
    guard visited == count else {
      throw ReaderError.invalid("The ZIP directory is incomplete or uses unsupported encryption.")
    }
    self.archive = archive
    self.entries = entries
  }

  func data(at path: String, maximumBytes: Int = maximumResourceBytes) throws -> Data {
    try Task.checkCancellation()
    guard let entry = entries[path], entry.uncompressedSize <= maximumBytes else {
      throw ReaderError.invalid("A required resource is missing or too large: \(path)")
    }
    var data = Data()
    let crc = try archive.extract(entry, bufferSize: 32 * 1024) { chunk in
      try Task.checkCancellation()
      guard chunk.count <= maximumBytes - data.count else {
        throw ReaderError.limit("A resource exceeded its expanded byte limit.")
      }
      data.append(chunk)
    }
    guard data.count == entry.uncompressedSize, crc == entry.checksum else {
      throw ReaderError.invalid("A resource failed its integrity check.")
    }
    return data
  }

  func contains(_ path: String) -> Bool { entries[path] != nil }

  func paths() -> [String] { entries.keys.sorted() }

  static func checkImage(_ data: Data, mediaType: String) throws {
    // A manifest MIME label is untrusted: sniff raster data regardless of its label.
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
      CGImageSourceGetType(source) != nil, CGImageSourceGetCount(source) > 0
    else {
      if mediaType.hasPrefix("image/"), mediaType != "image/svg+xml" {
        throw ReaderError.invalid("A declared image has an unsupported or invalid raster encoding.")
      }
      return
    }
    guard
      let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
      let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
      let height = properties[kCGImagePropertyPixelHeight] as? NSNumber,
      width.doubleValue > 0, height.doubleValue > 0,
      width.doubleValue <= 32_768, height.doubleValue <= 32_768,
      width.doubleValue * height.doubleValue <= 80_000_000
    else { throw ReaderError.limit("An image has unsupported or excessive dimensions.") }
  }

  static func canonicalPath(_ raw: String, directory: Bool = false) throws -> String {
    var path = raw
    if directory, path.hasSuffix("/") { path.removeLast() }
    for candidate in pathAliases(path) {
      let pieces = candidate.split(separator: "/", omittingEmptySubsequences: false)
      guard !candidate.isEmpty, !candidate.contains("\\"), !candidate.contains(":"),
        !candidate.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 }),
        pieces.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." })
      else { throw ReaderError.invalid("An archive path is unsafe.") }
    }
    // ZIP names are literal names, not URLs. Never replace their percent sequences.
    return path.precomposedStringWithCanonicalMapping
  }

  private static func pathAliases(_ path: String) -> Set<String> {
    var aliases: Set<String> = [path]
    var candidate = path
    for _ in 0..<4 {
      guard let decoded = candidate.removingPercentEncoding, decoded != candidate else { break }
      aliases.insert(decoded.precomposedStringWithCanonicalMapping)
      candidate = decoded
    }
    return aliases
  }
}

/// Check the bounded ZIP envelope before the library's iterator can skip an
/// encrypted/malformed entry. ZIP64 and split archives are explicit limits for now.
private enum ZIPEnvelope {
  static func validate(url: URL) throws -> Int {
    try Task.checkCancellation()
    let handle = try FileHandle(forReadingFrom: url)
    defer { try? handle.close() }
    let size = try handle.seekToEnd()
    guard size >= 22, size <= ArchiveStore.maximumFileBytes else {
      throw ReaderError.limit("The archive must be at most 512 MB.")
    }
    let tailSize = min(Int(size), 65_557)
    try handle.seek(toOffset: size - UInt64(tailSize))
    let tail = try handle.read(upToCount: tailSize) ?? Data()
    guard tail.count == tailSize else { throw ReaderError.invalid("The ZIP is truncated.") }
    var end: Int?
    for index in stride(from: tail.count - 22, through: 0, by: -1) {
      if tail.u32(index) == 0x0605_4b50,
        index + 22 + Int(tail.u16(index + 20)) == tail.count
      {
        end = index
        break
      }
    }
    guard let end else { throw ReaderError.invalid("The ZIP directory is missing.") }
    let count = Int(tail.u16(end + 10))
    let length = Int(tail.u32(end + 12))
    let offset = UInt64(tail.u32(end + 16))
    guard tail.u16(end + 4) == 0, tail.u16(end + 6) == 0,
      tail.u16(end + 8) == count, count > 0, count <= 10_000,
      length <= 4 * 1024 * 1024, offset + UInt64(length) == size - UInt64(tailSize) + UInt64(end)
    else {
      throw ReaderError.unsupported(
        "Split, ZIP64, oversized, or malformed archives are not supported.")
    }
    try handle.seek(toOffset: offset)
    let directory = try handle.read(upToCount: length) ?? Data()
    guard directory.count == length else {
      throw ReaderError.invalid("The ZIP directory is truncated.")
    }
    var cursor = 0
    for _ in 0..<count {
      try Task.checkCancellation()
      guard cursor + 46 <= directory.count, directory.u32(cursor) == 0x0201_4b50 else {
        throw ReaderError.invalid("A ZIP directory entry is malformed.")
      }
      let flags = directory.u16(cursor + 8)
      let method = directory.u16(cursor + 10)
      let localOffset = UInt64(directory.u32(cursor + 42))
      let compressed = UInt64(directory.u32(cursor + 20))
      let nameLength = Int(directory.u16(cursor + 28))
      let next =
        cursor + 46 + nameLength + Int(directory.u16(cursor + 30)) + Int(directory.u16(cursor + 32))
      guard flags & 0x2041 == 0, method == 0 || method == 8,
        directory.u16(cursor + 34) == 0, next <= directory.count,
        localOffset + 30 + compressed <= offset
      else {
        throw ReaderError.unsupported(
          "The ZIP uses unsupported encryption, compression, or directory data.")
      }
      try handle.seek(toOffset: localOffset)
      let local = try handle.read(upToCount: 30) ?? Data()
      guard local.count == 30, local.u32(0) == 0x0403_4b50,
        local.u16(6) == flags, local.u16(8) == method,
        local.u16(26) == nameLength,
        localOffset + 30 + UInt64(nameLength) + UInt64(local.u16(28)) + compressed <= offset
      else { throw ReaderError.invalid("A ZIP local header disagrees with its directory.") }
      let name = try handle.read(upToCount: nameLength) ?? Data()
      guard name == directory.subdata(in: (cursor + 46)..<(cursor + 46 + nameLength)) else {
        throw ReaderError.invalid("A ZIP entry has ambiguous filenames.")
      }
      cursor = next
    }
    guard cursor == directory.count else {
      throw ReaderError.invalid("The ZIP directory has extra data.")
    }
    return count
  }
}

extension Data {
  fileprivate func u16(_ index: Int) -> UInt16 {
    UInt16(self[index]) | UInt16(self[index + 1]) << 8
  }

  fileprivate func u32(_ index: Int) -> UInt32 {
    UInt32(u16(index)) | UInt32(u16(index + 2)) << 16
  }
}
