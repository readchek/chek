import Combine
import Observation
import SwiftUI
import UIKit

@MainActor @Observable
public final class CBZReader {
  public let publication: CBZPublication
  public private(set) var currentPageIndex: Int
  public private(set) var location: CBZLocation
  public private(set) var readingDirection: CBZReadingDirection
  public private(set) var isLoading = false
  public var message: String?
  private(set) var decodedPage: CBZDecodedPage?
  private(set) var failureDescription: String?
  private(set) var loadRequestID = UUID()
  private let pipeline: CBZImagePipeline
  private var renderWidth = 0
  private var viewportWidth = 0
  @ObservationIgnored fileprivate var positionProvider: (() -> (String, Double)?)?

  public init(
    publication: CBZPublication, location: CBZLocation? = nil,
    direction: CBZReadingDirection? = nil
  ) {
    self.publication = publication
    let index = location.flatMap { publication.pageIndex(for: $0) } ?? 0
    currentPageIndex = index
    self.location =
      location.flatMap { publication.pageIndex(for: $0) != nil ? $0 : nil }
      ?? CBZLocation(pageID: publication.pages[index].id)
    readingDirection = direction ?? publication.readingDirection
    pipeline = CBZImagePipeline(publication: publication)
  }

  public var currentPage: CBZPage { publication.pages[currentPageIndex] }

  public var hasMessage: Bool {
    get { message != nil }
    set { if !newValue { message = nil } }
  }

  public func openPage(at index: Int) {
    guard publication.pages.indices.contains(index), index != currentPageIndex else { return }
    currentPageIndex = index
    location = CBZLocation(pageID: publication.pages[index].id)
    decodedPage = nil
    failureDescription = nil
    renderWidth = viewportWidth
    loadRequestID = UUID()
  }

  /// Delta follows publication order. The app can place its buttons according to
  /// readingDirection without changing stable page indices or saved locations.
  public func movePage(by delta: Int) {
    guard (-publication.pages.count...publication.pages.count).contains(delta) else { return }
    openPage(at: currentPageIndex + delta)
  }

  public func setReadingDirection(_ direction: CBZReadingDirection) {
    readingDirection = direction
  }

  @discardableResult public func captureLocation() -> CBZLocation {
    if let (pageID, fraction) = positionProvider?() {
      recordPosition(pageID: pageID, progression: fraction, force: true)
    }
    return location
  }

  public func retry() {
    failureDescription = nil
    loadRequestID = UUID()
  }

  fileprivate func requestWidth(viewport: Int, zoom: CGFloat) {
    guard viewport > 0, zoom.isFinite else { return }
    viewportWidth = min(4096, viewport)
    let width = min(4096, Int(ceil(Double(viewportWidth) * Double(max(1, min(4, zoom))))))
    let bucket = ((width + 127) / 128) * 128
    guard bucket != renderWidth else { return }
    renderWidth = bucket
    loadRequestID = UUID()
  }

  fileprivate func recordPosition(pageID: String, progression: Double, force: Bool = false) {
    guard pageID == currentPage.id, progression.isFinite else { return }
    let fraction = min(1, max(0, progression))
    if force || abs(location.progression - fraction) >= 0.0005 {
      location = CBZLocation(pageID: pageID, progression: fraction)
    }
  }

  fileprivate func load() async {
    guard renderWidth > 0, !Task.isCancelled else { return }
    let generation = loadRequestID
    let pageID = currentPage.id
    isLoading = true
    defer { if generation == loadRequestID { isLoading = false } }
    do {
      let page = try await pipeline.image(for: pageID, targetWidth: renderWidth)
      try Task.checkCancellation()
      guard generation == loadRequestID, currentPage.id == pageID else { return }
      decodedPage = page
      failureDescription = nil
    } catch is CancellationError {
    } catch {
      guard !Task.isCancelled, generation == loadRequestID else { return }
      if decodedPage == nil {
        failureDescription = error.localizedDescription
      } else {
        // A rejected zoom decode keeps the usable image and reading position.
        message = error.localizedDescription
      }
    }
  }

  fileprivate func clearCache() async { await pipeline.removeAll() }
}

public struct CBZReaderView: View {
  private let reader: CBZReader

  public init(reader: CBZReader) { self.reader = reader }

  public var body: some View {
    ZStack {
      ComicCanvas(reader: reader)
      if let failure = reader.failureDescription {
        ContentUnavailableView {
          Label(
            String(localized: "Page could not load", bundle: .module),
            systemImage: "exclamationmark.triangle")
        } description: {
          Text(failure)
        } actions: {
          Button(String(localized: "Try again", bundle: .module)) { reader.retry() }
        }
        .background(.background)
      } else if reader.decodedPage == nil {
        ProgressView(String(localized: "Opening page…", bundle: .module))
      }
    }
    .task(id: reader.loadRequestID) { await reader.load() }
    .onReceive(
      NotificationCenter.default.publisher(for: UIApplication.didReceiveMemoryWarningNotification)
    ) {
      _ in Task { await reader.clearCache() }
    }
  }
}

private struct ComicCanvas: UIViewRepresentable {
  let reader: CBZReader

  func makeUIView(context: Context) -> ComicScrollView { ComicScrollView() }

  func updateUIView(_ view: ComicScrollView, context: Context) {
    let pageID = reader.currentPage.id
    view.onPosition = { [weak reader] pageID, fraction in
      reader?.recordPosition(pageID: pageID, progression: fraction)
    }
    view.onWidth = { [weak reader] viewport, zoom in
      // UIKit may lay out during a SwiftUI update. Publish the new request on
      // the next actor turn, rather than mutating observed state in that update.
      Task { @MainActor [weak reader] in
        guard reader?.currentPage.id == pageID else { return }
        reader?.requestWidth(viewport: viewport, zoom: zoom)
      }
    }
    reader.positionProvider = { [weak view] in
      guard let view, let pageID = view.pageID, view.hasImage else { return nil }
      return (pageID, view.progression)
    }
    view.show(
      reader.decodedPage, pageID: reader.currentPage.id, progression: reader.location.progression,
      pageNumber: reader.currentPageIndex + 1, pageCount: reader.publication.pages.count)
  }

  static func dismantleUIView(_ view: ComicScrollView, coordinator: ()) {
    view.delegate = nil
    view.onPosition = nil
    view.onWidth = nil
    view.releaseImage()
  }
}

/// UIKit supplies native pinch zoom, pan, deceleration and accessible scrolling.
/// Only the current image is retained here; pages are never stacked or predecoded.
@MainActor private final class ComicScrollView: UIScrollView, UIScrollViewDelegate {
  private let pageView = UIImageView()
  private var decoded: CBZDecodedPage?
  private var previousSize = CGSize.zero
  private var updating = false
  private var pendingProgression = 0.0
  private(set) var pageID: String?
  var onPosition: ((String, Double) -> Void)?
  var onWidth: ((Int, CGFloat) -> Void)?

  var hasImage: Bool { decoded != nil }
  var progression: Double {
    guard contentSize.height > 0 else { return pendingProgression }
    return min(1, max(0, Double((contentOffset.y + contentInset.top) / contentSize.height)))
  }

  init() {
    super.init(frame: .zero)
    delegate = self
    minimumZoomScale = 1
    maximumZoomScale = 4
    contentInsetAdjustmentBehavior = .never
    showsHorizontalScrollIndicator = false
    showsVerticalScrollIndicator = true
    backgroundColor = .systemBackground
    pageView.contentMode = .scaleAspectFit
    pageView.isAccessibilityElement = true
    addSubview(pageView)
  }

  required init?(coder: NSCoder) { nil }

  func show(
    _ page: CBZDecodedPage?, pageID: String, progression: Double, pageNumber: Int, pageCount: Int
  ) {
    pageView.accessibilityLabel = String(
      localized: "Comic page \(pageNumber) of \(pageCount)", bundle: .module,
      comment: "Accessible image label: current comic page number followed by total page count.")
    let changedPage = self.pageID != pageID
    let changedImage = decoded?.image !== page?.image
    guard changedPage || changedImage else { return }
    let fraction = changedPage ? progression : self.progression
    let zoom = changedPage ? 1 : zoomScale
    self.pageID = pageID
    decoded = page
    pendingProgression = fraction
    pageView.image = page.map { UIImage(cgImage: $0.image) }
    reflow(progression: fraction, zoom: zoom)
  }

  func releaseImage() {
    decoded = nil
    pageView.image = nil
  }

  override func layoutSubviews() {
    guard !updating, bounds.size != previousSize else {
      super.layoutSubviews()
      return
    }
    // UIKit can clamp offsets and call delegates while laying out a new size.
    // Keep the last deliberate reading position until reflow restores it.
    let fraction = pendingProgression
    let zoom = zoomScale
    updating = true
    super.layoutSubviews()
    previousSize = bounds.size
    updating = false
    reflow(progression: fraction, zoom: zoom)
    requestWidth()
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window != nil { requestWidth() }
  }

  private func reflow(progression: Double, zoom: CGFloat) {
    guard bounds.width > 0, !updating else { return }
    updating = true
    defer { updating = false }
    let horizontalFraction =
      contentSize.width > 0
      ? max(0, (contentOffset.x + contentInset.left) / contentSize.width) : 0
    setZoomScale(1, animated: false)
    guard let decoded else {
      pageView.frame = .zero
      contentSize = .zero
      contentInset = .zero
      contentOffset = .zero
      return
    }
    let width = bounds.width
    let height = width * CGFloat(decoded.sourceHeight) / CGFloat(decoded.sourceWidth)
    pageView.frame = CGRect(x: 0, y: 0, width: width, height: height)
    contentSize = pageView.bounds.size
    setZoomScale(max(1, min(4, zoom)), animated: false)
    contentSize = CGSize(width: width * zoomScale, height: height * zoomScale)
    centerImage()
    let y = CGFloat(progression) * contentSize.height - contentInset.top
    let x = horizontalFraction * contentSize.width - contentInset.left
    contentOffset = CGPoint(
      x: min(
        max(-contentInset.left, x),
        max(-contentInset.left, contentSize.width - bounds.width + contentInset.right)),
      y: min(
        max(-contentInset.top, y),
        max(-contentInset.top, contentSize.height - bounds.height + contentInset.bottom)))
  }

  private func centerImage() {
    let vertical = max(0, (bounds.height - contentSize.height) / 2)
    let horizontal = max(0, (bounds.width - contentSize.width) / 2)
    contentInset = UIEdgeInsets(
      top: vertical, left: horizontal, bottom: vertical, right: horizontal)
  }

  private func requestWidth() {
    let pixels = bounds.width * traitCollection.displayScale
    guard pixels.isFinite, pixels > 0 else { return }
    onWidth?(Int(min(4096, ceil(pixels))), zoomScale)
  }

  func viewForZooming(in scrollView: UIScrollView) -> UIView? { hasImage ? pageView : nil }

  func scrollViewDidZoom(_ scrollView: UIScrollView) {
    guard !updating else { return }
    centerImage()
    publishPosition()
  }

  func scrollViewDidEndZooming(
    _ scrollView: UIScrollView, with view: UIView?, atScale scale: CGFloat
  ) {
    publishPosition()
    requestWidth()
  }

  func scrollViewDidScroll(_ scrollView: UIScrollView) { publishPosition() }

  private func publishPosition() {
    guard !updating, hasImage, let pageID else { return }
    pendingProgression = progression
    onPosition?(pageID, progression)
  }
}
