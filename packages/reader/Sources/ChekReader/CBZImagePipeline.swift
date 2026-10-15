import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

public struct CBZDecodedPage: Sendable {
  public let image: CGImage
  /// Authored dimensions after applying the image's orientation.
  public let sourceWidth: Int
  public let sourceHeight: Int

  public var isLongStrip: Bool { Double(sourceHeight) / Double(sourceWidth) >= 3 }
}

/// Own one pipeline per open comic. Decodes serialize, and cancellation follows
/// the caller's task. The byte budget covers retained cache images, not images
/// retained by a consumer after eviction or ImageIO's transient decode workspace.
public actor CBZImagePipeline {
  public static let maximumDecodedPixels = 16_000_000
  public static let maximumCacheBytes = 64 * 1024 * 1024
  private let publication: CBZPublication
  private let pageIDs: Set<String>
  private var cache: [Key: Cached] = [:]
  private var cacheBytes = 0
  private var clock: UInt64 = 0
  private var decoding = false
  private var waiters: [Waiter] = []
  private var cacheGeneration: UInt64 = 0

  private struct Key: Hashable {
    let pageID: String
    let targetWidth: Int
    let minimumWidth: Int?
  }

  private struct Cached {
    let page: CBZDecodedPage
    let bytes: Int
    var lastUse: UInt64
  }

  private struct Waiter {
    let id: UUID
    let continuation: CheckedContinuation<Void, any Error>
  }

  public init(publication: CBZPublication) {
    self.publication = publication
    pageIDs = Set(publication.pages.map(\.id))
  }

  /// Pass viewport width multiplied by display scale. Width-based sampling keeps
  /// strips readable; an unsafe strip fails explicitly instead of shrinking its
  /// width merely to fit a longest-edge thumbnail limit. Call again at a larger
  /// width when zooming, subject to the same pixel and byte limits.
  public func image(for pageID: String, targetWidth: Int) async throws -> CBZDecodedPage {
    try await image(for: pageID, targetWidth: targetWidth, minimumWidth: nil)
  }

  /// The roll can spend surplus display-scale resolution on unusually tall
  /// pages, but must retain at least one pixel per zoomed layout point.
  func imageForRoll(pageID: String, targetWidth: Int, minimumWidth: Int) async throws
    -> CBZDecodedPage
  {
    try await image(for: pageID, targetWidth: targetWidth, minimumWidth: minimumWidth)
  }

  private func image(for pageID: String, targetWidth: Int, minimumWidth: Int?) async throws
    -> CBZDecodedPage
  {
    try Task.checkCancellation()
    guard pageIDs.contains(pageID), targetWidth > 0,
      minimumWidth.map({ $0 > 0 && $0 <= min(4096, targetWidth) }) ?? true
    else {
      throw ReaderError.invalid("The requested comic page or render width is invalid.")
    }
    let boundedWidth = min(4096, targetWidth)
    let bucket = ((boundedWidth + 127) / 128) * 128
    let key = Key(pageID: pageID, targetWidth: bucket, minimumWidth: minimumWidth)
    if let page = cached(key) { return page }
    try await acquire()
    defer { release() }
    try Task.checkCancellation()
    if let page = cached(key) { return page }
    let generation = cacheGeneration
    let data = try await publication.archive.data(at: pageID)
    try Task.checkCancellation()
    // Synchronous actor-isolated decoding cannot overlap another decode. Archive
    // reads also remain behind this permit, so waiting requests retain no bytes.
    let decoded = try decode(data, targetWidth: bucket, minimumWidth: minimumWidth)
    try Task.checkCancellation()
    let cost = decoded.image.bytesPerRow * decoded.image.height
    guard cost <= Self.maximumCacheBytes else {
      throw ReaderError.limit("This comic page exceeds the decoded image byte budget.")
    }
    // A memory warning can clear the cache while the archive read is suspended.
    // The requesting view may still use its image, but that older request must
    // not immediately refill the cache that the host just released.
    guard generation == cacheGeneration else { return decoded }
    // A successful upgrade replaces this page's old bitmap. Keep one retained
    // resolution per page rather than charging every requested width bucket.
    let superseded = cache.keys.filter { $0.pageID == pageID }
    for previousKey in superseded {
      if let previous = cache.removeValue(forKey: previousKey) {
        cacheBytes -= previous.bytes
      }
    }
    while cacheBytes > Self.maximumCacheBytes - cost {
      guard let oldest = cache.min(by: { $0.value.lastUse < $1.value.lastUse }) else { break }
      cacheBytes -= oldest.value.bytes
      cache.removeValue(forKey: oldest.key)
    }
    clock &+= 1
    cache[key] = Cached(page: decoded, bytes: cost, lastUse: clock)
    cacheBytes += cost
    return decoded
  }

  public func removeAll() {
    cacheGeneration &+= 1
    cache.removeAll()
    cacheBytes = 0
  }

  /// Inspect one page without decoding a bitmap. Automatic mode samples the
  /// first content page after the cover; it is a preference heuristic, not format metadata.
  public func isLongStrip(pageID: String) async throws -> Bool {
    guard pageIDs.contains(pageID) else {
      throw ReaderError.invalid("The requested comic page is invalid.")
    }
    try await acquire()
    defer { release() }
    let data = try await publication.archive.data(at: pageID)
    try Task.checkCancellation()
    guard
      let source = CGImageSourceCreateWithData(
        data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
      let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
      let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
      let height = properties[kCGImagePropertyPixelHeight] as? NSNumber,
      width.doubleValue > 0, height.doubleValue > 0
    else { return false }
    let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
    let ratio = height.doubleValue / width.doubleValue
    return (5...8).contains(orientation) ? 1 / ratio >= 3 : ratio >= 3
  }

  private func cached(_ key: Key) -> CBZDecodedPage? {
    let existingKey: Key? =
      if cache[key] != nil {
        key
      } else {
        cache.first(where: { candidate in
          let page = candidate.value.page
          return candidate.key.pageID == key.pageID
            && candidate.key.minimumWidth == key.minimumWidth
            && page.image.width >= min(page.sourceWidth, key.targetWidth)
            && page.image.width <= key.targetWidth
        })?.key
      }
    guard let existingKey, var item = cache[existingKey] else { return nil }
    clock &+= 1
    item.lastUse = clock
    cache[existingKey] = item
    return item.page
  }

  private func acquire() async throws {
    try Task.checkCancellation()
    if !decoding {
      decoding = true
      return
    }
    let id = UUID()
    try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation {
        (continuation: CheckedContinuation<Void, any Error>) in
        if Task.isCancelled {
          continuation.resume(throwing: CancellationError())
        } else {
          waiters.append(Waiter(id: id, continuation: continuation))
        }
      }
    } onCancel: {
      Task { await self.cancelWaiter(id) }
    }
  }

  private func cancelWaiter(_ id: UUID) {
    guard let index = waiters.firstIndex(where: { $0.id == id }) else { return }
    waiters.remove(at: index).continuation.resume(throwing: CancellationError())
  }

  private func release() {
    if waiters.isEmpty {
      decoding = false
    } else {
      waiters.removeFirst().continuation.resume()
    }
  }

  private func decode(_ data: Data, targetWidth: Int, minimumWidth: Int?) throws -> CBZDecodedPage {
    try Task.checkCancellation()
    guard
      let source = CGImageSourceCreateWithData(
        data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
      let type = CGImageSourceGetType(source) as String?,
      [UTType.jpeg.identifier, UTType.png.identifier].contains(type),
      CGImageSourceGetCount(source) == 1,
      let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
      let rawWidth = properties[kCGImagePropertyPixelWidth] as? NSNumber,
      let rawHeight = properties[kCGImagePropertyPixelHeight] as? NSNumber,
      rawWidth.doubleValue.isFinite, rawHeight.doubleValue.isFinite,
      rawWidth.doubleValue >= 1, rawHeight.doubleValue >= 1,
      rawWidth.doubleValue <= 32_768, rawHeight.doubleValue <= 32_768,
      rawWidth.doubleValue * rawHeight.doubleValue <= 80_000_000
    else {
      throw ReaderError.invalid(
        "A comic page is not a supported single-frame JPEG/PNG, or its dimensions exceed the limit."
      )
    }
    let orientation = (properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue ?? 1
    let rotated = (5...8).contains(orientation)
    let width = rotated ? rawHeight.intValue : rawWidth.intValue
    let height = rotated ? rawWidth.intValue : rawHeight.intValue
    var desiredWidth = min(width, targetWidth)
    if let minimumWidth {
      // Dimensions are inspected before the first bitmap allocation. Unknown
      // strip ratios must not turn a legal page into a permanently failed
      // placeholder merely because the roll's estimate requested too many
      // pixels. Preserve the explicit readability floor when lowering quality.
      let safeWidth = Int(sqrt(Double(Self.maximumDecodedPixels) * Double(width) / Double(height)))
      if desiredWidth > safeWidth {
        guard safeWidth >= min(width, minimumWidth) else {
          throw ReaderError.limit(
            "This strip is too tall to decode at a readable width within the current image budget.")
        }
        desiredWidth = min(desiredWidth, max(min(width, minimumWidth), (safeWidth / 128) * 128))
      }
    }
    let desiredHeight = Int(ceil(Double(height) * Double(desiredWidth) / Double(width)))
    guard desiredWidth * desiredHeight <= Self.maximumDecodedPixels else {
      throw ReaderError.limit(
        "This strip is too tall to decode at a readable width within the current image budget.")
    }
    let longestEdge = max(desiredWidth, desiredHeight)
    // Discard older cache entries before allocating the next image. The returned
    // image's actual row stride is checked below; this estimate assumes RGBA8.
    let estimate = desiredWidth * desiredHeight * 4
    while cacheBytes > Self.maximumCacheBytes - estimate {
      guard let oldest = cache.min(by: { $0.value.lastUse < $1.value.lastUse }) else { break }
      cacheBytes -= oldest.value.bytes
      cache.removeValue(forKey: oldest.key)
    }
    try Task.checkCancellation()
    guard
      let image = CGImageSourceCreateThumbnailAtIndex(
        source, 0,
        [
          kCGImageSourceCreateThumbnailFromImageAlways: true,
          kCGImageSourceCreateThumbnailWithTransform: true,
          kCGImageSourceThumbnailMaxPixelSize: longestEdge,
          kCGImageSourceShouldCacheImmediately: true,
          kCGImageSourceShouldAllowFloat: false,
        ] as CFDictionary),
      image.width * image.height <= Self.maximumDecodedPixels,
      image.bytesPerRow * image.height <= Self.maximumCacheBytes
    else { throw ReaderError.limit("The comic page could not be decoded within the image budget.") }
    try Task.checkCancellation()
    return CBZDecodedPage(image: image, sourceWidth: width, sourceHeight: height)
  }
}
