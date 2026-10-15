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
  public private(set) var readingMode: CBZReadingMode
  public private(set) var effectiveReadingMode: CBZReadingMode = .paged
  public private(set) var isLoading = false
  public var message: String?
  public var allowsPageSwipe: Bool
  private(set) var decodedPage: CBZDecodedPage?
  public private(set) var failureDescription: String?
  private(set) var loadRequestID = UUID()
  private let pipeline: CBZImagePipeline
  private var automaticMode: CBZReadingMode?
  private var continuousReady = false
  private(set) var navigationRequestID = UUID()
  private var renderWidth = 0
  private var viewportWidth = 0
  private(set) var cacheReleaseID = UUID()
  @ObservationIgnored fileprivate var positionProvider: (() -> (String, Double)?)?
  @ObservationIgnored private var positionProviderOwner: ObjectIdentifier?

  public init(
    publication: CBZPublication, location: CBZLocation? = nil,
    direction: CBZReadingDirection? = nil, readingMode: CBZReadingMode = .automatic,
    allowsPageSwipe: Bool = true
  ) {
    self.publication = publication
    self.allowsPageSwipe = allowsPageSwipe
    let index = location.flatMap { publication.pageIndex(for: $0) } ?? 0
    currentPageIndex = index
    self.location =
      location.flatMap { publication.pageIndex(for: $0) != nil ? $0 : nil }
      ?? CBZLocation(pageID: publication.pages[index].id)
    readingDirection = direction ?? publication.readingDirection
    self.readingMode = readingMode
    effectiveReadingMode = readingMode == .continuous ? .continuous : .paged
    pipeline = CBZImagePipeline(publication: publication)
  }

  public var currentPage: CBZPage { publication.pages[currentPageIndex] }
  public var isReadyForInteraction: Bool {
    effectiveReadingMode == .continuous ? continuousReady : decodedPage != nil
  }

  public var hasMessage: Bool {
    get { message != nil }
    set { if !newValue { message = nil } }
  }

  public func openPage(at index: Int) {
    guard publication.pages.indices.contains(index), index != currentPageIndex else { return }
    currentPageIndex = index
    continuousReady = false
    location = CBZLocation(pageID: publication.pages[index].id)
    decodedPage = nil
    failureDescription = nil
    renderWidth = viewportWidth
    loadRequestID = UUID()
    navigationRequestID = UUID()
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

  public func setReadingMode(_ mode: CBZReadingMode) {
    guard mode != readingMode else { return }
    captureLocation()
    readingMode = mode
    effectiveReadingMode = mode == .automatic ? automaticMode ?? .paged : mode
    decodedPage = nil
    continuousReady = false
    failureDescription = nil
    navigationRequestID = UUID()
    loadRequestID = UUID()
  }

  @discardableResult public func captureLocation() -> CBZLocation {
    if let (pageID, fraction) = positionProvider?() {
      recordPosition(pageID: pageID, progression: fraction, force: true)
    }
    return location
  }

  fileprivate func setPositionProvider(
    owner: AnyObject, provider: @escaping () -> (String, Double)?
  ) {
    positionProviderOwner = ObjectIdentifier(owner)
    positionProvider = provider
  }

  fileprivate func clearPositionProvider(owner: AnyObject) {
    guard positionProviderOwner == ObjectIdentifier(owner) else { return }
    positionProviderOwner = nil
    positionProvider = nil
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
    guard progression.isFinite else { return }
    if pageID != currentPage.id {
      guard effectiveReadingMode == .continuous,
        let index = publication.pageIndex(for: CBZLocation(pageID: pageID))
      else { return }
      currentPageIndex = index
    }
    let fraction = min(1, max(0, progression))
    if force || location.pageID != pageID || abs(location.progression - fraction) >= 0.0005 {
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
      if readingMode == .automatic, automaticMode == nil {
        let sample = publication.pages[min(1, publication.pages.count - 1)].id
        let longStrip = try await pipeline.isLongStrip(pageID: sample)
        try Task.checkCancellation()
        guard generation == loadRequestID else { return }
        automaticMode = longStrip ? .continuous : .paged
        effectiveReadingMode = automaticMode ?? .paged
      }
      guard effectiveReadingMode == .paged else { return }
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

  fileprivate func clearCache() async {
    cacheReleaseID = UUID()
    await pipeline.removeAll()
  }

  fileprivate func image(pageID: String, width: Int) async throws -> CBZDecodedPage {
    try await pipeline.image(for: pageID, targetWidth: width)
  }

  fileprivate func setContinuousReady(_ ready: Bool) {
    guard continuousReady != ready else { return }
    continuousReady = ready
  }
}

public struct CBZReaderView: View {
  private let reader: CBZReader
  private let onTap: () -> Void
  private let onInteraction: () -> Void

  public init(
    reader: CBZReader, onTap: @escaping () -> Void = {},
    onInteraction: @escaping () -> Void = {}
  ) {
    self.reader = reader
    self.onTap = onTap
    self.onInteraction = onInteraction
  }

  public var body: some View {
    ZStack {
      if reader.effectiveReadingMode == .continuous {
        ComicRollCanvas(reader: reader, onTap: onTap, onInteraction: onInteraction)
      } else {
        ComicCanvas(reader: reader, onTap: onTap, onInteraction: onInteraction)
      }
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
      } else if reader.effectiveReadingMode == .paged && reader.decodedPage == nil {
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
  let onTap: () -> Void
  let onInteraction: () -> Void

  func makeUIView(context: Context) -> ComicScrollView { ComicScrollView() }

  func updateUIView(_ view: ComicScrollView, context: Context) {
    view.bind(reader)
    let pageID = reader.currentPage.id
    view.onTap = onTap
    view.onInteraction = onInteraction
    view.allowsPageSwipe = reader.allowsPageSwipe
    view.onSwipe = { [weak reader] left in
      guard let reader, reader.allowsPageSwipe else { return }
      let forward = left == (reader.readingDirection == .leftToRight)
      reader.movePage(by: forward ? 1 : -1)
    }
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
    reader.setPositionProvider(owner: view) { [weak view] in
      guard let view, let pageID = view.pageID, view.hasImage else { return nil }
      return (pageID, view.progression)
    }
    view.show(
      reader.decodedPage, pageID: reader.currentPage.id, progression: reader.location.progression,
      pageNumber: reader.currentPageIndex + 1, pageCount: reader.publication.pages.count)
  }

  static func dismantleUIView(_ view: ComicScrollView, coordinator: ()) {
    view.bind(nil)
    view.delegate = nil
    view.onPosition = nil
    view.onWidth = nil
    view.onTap = nil
    view.onInteraction = nil
    view.onSwipe = nil
    view.releaseImage()
  }
}

/// UIKit supplies native pinch zoom, pan, deceleration and accessible scrolling.
/// Only the current image is retained here; pages are never stacked or predecoded.
@MainActor
private final class ComicScrollView: UIScrollView, UIScrollViewDelegate, UIGestureRecognizerDelegate
{
  private let pageView = UIImageView()
  private var decoded: CBZDecodedPage?
  private weak var reader: CBZReader?
  private var previousSize = CGSize.zero
  private var updating = false
  private var pendingProgression = 0.0
  private(set) var pageID: String?
  var onPosition: ((String, Double) -> Void)?
  var onWidth: ((Int, CGFloat) -> Void)?
  var onTap: (() -> Void)?
  var onInteraction: (() -> Void)?
  var onSwipe: ((Bool) -> Void)?
  var allowsPageSwipe = true

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
    let tap = UITapGestureRecognizer(target: self, action: #selector(tapped))
    tap.cancelsTouchesInView = false
    addGestureRecognizer(tap)
    for direction: UISwipeGestureRecognizer.Direction in [.left, .right] {
      let swipe = UISwipeGestureRecognizer(target: self, action: #selector(swiped(_:)))
      swipe.direction = direction
      swipe.delegate = self
      addGestureRecognizer(swipe)
    }
  }

  @objc private func tapped() { onTap?() }

  @objc private func swiped(_ gesture: UISwipeGestureRecognizer) {
    guard zoomScale <= 1.01 else { return }
    onInteraction?()
    onSwipe?(gesture.direction == .left)
  }

  override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
    if gestureRecognizer is UISwipeGestureRecognizer {
      return allowsPageSwipe && zoomScale <= 1.01
    }
    return super.gestureRecognizerShouldBegin(gestureRecognizer)
  }

  func gestureRecognizer(
    _ gestureRecognizer: UIGestureRecognizer,
    shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
  ) -> Bool { gestureRecognizer is UISwipeGestureRecognizer }

  func scrollViewWillBeginDragging(_ scrollView: UIScrollView) { onInteraction?() }
  func scrollViewWillBeginZooming(_ scrollView: UIScrollView, with view: UIView?) {
    onInteraction?()
  }

  required init?(coder: NSCoder) { nil }

  func bind(_ reader: CBZReader?) {
    guard self.reader !== reader else { return }
    self.reader?.clearPositionProvider(owner: self)
    self.reader = reader
    pageID = nil
    pendingProgression = 0
    releaseImage()
    // A reused representable already has a window and unchanged bounds, so
    // neither lifecycle hook would otherwise send this reader its first width.
    previousSize = .zero
    setNeedsLayout()
  }

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

private struct ComicRollCanvas: UIViewRepresentable {
  let reader: CBZReader
  let onTap: () -> Void
  let onInteraction: () -> Void

  func makeUIView(context: Context) -> ComicRollView { ComicRollView() }

  func updateUIView(_ view: ComicRollView, context: Context) {
    view.onTap = onTap
    view.onInteraction = onInteraction
    view.configure(reader)
    reader.setPositionProvider(owner: view) { [weak view] in view?.readingPosition }
  }

  static func dismantleUIView(_ view: ComicRollView, coordinator: ()) { view.stop() }
}

/// One native scroll surface spans the publication. Unknown page heights are
/// estimates until that page comes near the viewport; replacing an estimate
/// preserves the currently visible page and its within-page position. Only five
/// nearby page views/requests can exist, independent of the archive's page count.
@MainActor private final class ComicRollView: UIScrollView, UIScrollViewDelegate {
  private struct Anchor {
    let index: Int
    let fraction: CGFloat
  }

  private let canvas = UIView()
  private weak var reader: CBZReader?
  private var pages: [CBZPage] = []
  private var ratios: [Int: CGFloat] = [:]
  private var starts: [CGFloat] = []
  private var cells: [Int: ComicRollPage] = [:]
  private var requests: [Int: (id: UUID, task: Task<Void, Never>)] = [:]
  private var navigationID: UUID?
  private var cacheReleaseID: UUID?
  private var keepsAdjacentPages = true
  private var previousSize = CGSize.zero
  private var updating = false
  private var pendingAnchor: Anchor?
  private var lastAnchor: Anchor?
  private var lastPublished = 0.0
  var onTap: (() -> Void)?
  var onInteraction: (() -> Void)?

  var readingPosition: (String, Double)? {
    guard let anchor = anchor(), pages.indices.contains(anchor.index) else { return nil }
    return (pages[anchor.index].id, Double(anchor.fraction))
  }

  init() {
    super.init(frame: .zero)
    delegate = self
    minimumZoomScale = 1
    maximumZoomScale = 4
    contentInsetAdjustmentBehavior = .never
    showsHorizontalScrollIndicator = false
    backgroundColor = .systemBackground
    addSubview(canvas)
    let tap = UITapGestureRecognizer(target: self, action: #selector(tapped))
    tap.cancelsTouchesInView = false
    addGestureRecognizer(tap)
  }

  required init?(coder: NSCoder) { nil }

  func configure(_ reader: CBZReader) {
    if self.reader !== reader {
      resetPublication()
      self.reader = reader
      pages = reader.publication.pages
      cacheReleaseID = reader.cacheReleaseID
    }
    if cacheReleaseID != reader.cacheReleaseID {
      cacheReleaseID = reader.cacheReleaseID
      // Cache eviction cannot release images still held by UIImageViews. After
      // pressure, keep visible pages and cancel offscreen decode work.
      keepsAdjacentPages = false
      setNeedsLayout()
    }
    guard navigationID != reader.navigationRequestID else { return }
    navigationID = reader.navigationRequestID
    pendingAnchor = Anchor(
      index: reader.currentPageIndex, fraction: CGFloat(reader.location.progression))
    setNeedsLayout()
  }

  func stop() {
    resetPublication()
    delegate = nil
    onTap = nil
    onInteraction = nil
  }

  private func resetPublication() {
    for request in requests.values { request.task.cancel() }
    requests.removeAll()
    cells.removeAll()
    for view in canvas.subviews { view.removeFromSuperview() }
    reader?.clearPositionProvider(owner: self)
    reader = nil
    pages.removeAll()
    ratios.removeAll()
    starts.removeAll()
    navigationID = nil
    cacheReleaseID = nil
    keepsAdjacentPages = true
    pendingAnchor = nil
    lastAnchor = nil
    lastPublished = 0
    updating = true
    setZoomScale(1, animated: false)
    canvas.frame = .zero
    contentSize = .zero
    contentInset = .zero
    contentOffset = .zero
    updating = false
  }

  @objc private func tapped(_ gesture: UITapGestureRecognizer) {
    // A failed page's native retry button owns its touch.
    if let hit = canvas.hitTest(gesture.location(in: canvas), with: nil), hit is UIControl {
      return
    }
    onTap?()
  }

  override func layoutSubviews() {
    guard !updating else {
      super.layoutSubviews()
      return
    }
    let sizeChanged = bounds.size != previousSize
    let preserved = pendingAnchor ?? (sizeChanged ? lastAnchor : nil) ?? anchor()
    updating = true
    super.layoutSubviews()
    updating = false
    guard bounds.width > 0, bounds.height > 0, !pages.isEmpty else { return }
    if sizeChanged || pendingAnchor != nil || starts.isEmpty {
      previousSize = bounds.size
      pendingAnchor = nil
      rebuild(preserving: preserved)
    }
    updateVisiblePages()
  }

  private func anchor() -> Anchor? {
    guard !starts.isEmpty, !pages.isEmpty else { return pendingAnchor }
    let y = max(0, (contentOffset.y + contentInset.top) / max(1, zoomScale))
    var lower = 0
    var upper = pages.count
    while lower < upper {
      let middle = (lower + upper) / 2
      if starts[middle + 1] <= y { lower = middle + 1 } else { upper = middle }
    }
    let index = min(pages.count - 1, lower)
    let height = starts[index + 1] - starts[index]
    return Anchor(index: index, fraction: min(1, max(0, (y - starts[index]) / max(1, height))))
  }

  private func rebuild(preserving anchor: Anchor?, restorePosition: Bool = true) {
    guard !updating, bounds.width > 0 else { return }
    updating = true
    defer { updating = false }
    let zoom = zoomScale
    let xFraction = contentSize.width > 0 ? contentOffset.x / contentSize.width : 0
    starts = [0]
    starts.reserveCapacity(pages.count + 1)
    for index in pages.indices {
      starts.append(starts[index] + bounds.width * (ratios[index] ?? 3.5))
    }
    // Keep UIKit's zoom transform and active pan/deceleration intact while
    // neighboring dimensions are learned. Only layout the base-sized canvas.
    canvas.bounds = CGRect(x: 0, y: 0, width: bounds.width, height: starts.last ?? 0)
    canvas.center = CGPoint(x: canvas.bounds.width * zoom / 2, y: canvas.bounds.height * zoom / 2)
    for (index, cell) in cells { cell.frame = pageFrame(index) }
    contentSize = CGSize(width: canvas.bounds.width * zoom, height: canvas.bounds.height * zoom)
    centerCanvas()
    if restorePosition, let anchor, pages.indices.contains(anchor.index) {
      let y =
        (starts[anchor.index] + (starts[anchor.index + 1] - starts[anchor.index]) * anchor.fraction)
        * zoom
      contentOffset = CGPoint(
        x: min(max(0, xFraction * contentSize.width), max(0, contentSize.width - bounds.width)),
        y: min(
          max(-contentInset.top, y - contentInset.top),
          max(-contentInset.top, contentSize.height - bounds.height + contentInset.bottom)))
      lastAnchor = anchor
    }
  }

  private func pageFrame(_ index: Int) -> CGRect {
    CGRect(x: 0, y: starts[index], width: bounds.width, height: starts[index + 1] - starts[index])
  }

  private func centerCanvas() {
    contentInset = UIEdgeInsets(
      top: max(0, (bounds.height - contentSize.height) / 2),
      left: max(0, (bounds.width - contentSize.width) / 2),
      bottom: max(0, (bounds.height - contentSize.height) / 2),
      right: max(0, (bounds.width - contentSize.width) / 2))
  }

  private func updateVisiblePages() {
    guard !updating, !isZooming, let anchor = anchor(), let reader else { return }
    // Include every intersecting page before spending slots on prefetch. Keep
    // the five-view cap even for unusually short images; more than five images
    // in one viewport can remain placeholders until they approach its top.
    let bottom = max(0, (contentOffset.y + bounds.height) / max(1, zoomScale))
    var lower = anchor.index
    var upper = anchor.index
    while upper + 1 < pages.count, upper - lower < 4, starts[upper + 1] < bottom {
      upper += 1
    }
    if keepsAdjacentPages {
      if lower > 0, upper - lower < 4 { lower -= 1 }
      upper = min(pages.count - 1, lower + 4)
    }
    let desired = Set(lower...upper)
    for index in Array(cells.keys) where !desired.contains(index) {
      cells.removeValue(forKey: index)?.removeFromSuperview()
    }
    for index in Array(requests.keys) where !desired.contains(index) {
      requests.removeValue(forKey: index)?.task.cancel()
    }
    let width = min(4096, Int(ceil(bounds.width * traitCollection.displayScale * zoomScale)))
    guard width > 0 else { return }
    let priority =
      [anchor.index]
      + desired.filter { $0 != anchor.index }.sorted {
        abs($0 - anchor.index) < abs($1 - anchor.index)
      }
    for index in priority {
      let cell: ComicRollPage
      if let existing = cells[index] {
        cell = existing
      } else {
        cell = ComicRollPage()
        cell.frame = pageFrame(index)
        cell.imageView.accessibilityLabel = String(
          localized: "Comic page \(index + 1) of \(pages.count)", bundle: .module,
          comment: "Accessible image label: current comic page number followed by total page count."
        )
        cells[index] = cell
        canvas.addSubview(cell)
      }
      guard requests[index] == nil, !cell.failed,
        !cell.budgetDeferred || index == anchor.index,
        cell.renderWidth < width && !cell.fullResolution
      else { continue }
      let id = UUID()
      let pageID = pages[index].id
      let request = Task { [weak self, weak reader, weak cell] in
        guard let reader else { return }
        do {
          let decoded = try await reader.image(pageID: pageID, width: width)
          try Task.checkCancellation()
          guard let self, self.requests[index]?.id == id, let cell else { return }
          let preserved = self.anchor()
          self.requests[index] = nil
          // Consumer-held images have their own byte bound. Cache eviction alone
          // cannot release a UIImage still displayed by a page view.
          let cost = decoded.image.bytesPerRow * decoded.image.height
          if index != preserved?.index,
            let visible = preserved.flatMap({ self.cells[$0.index] }),
            visible.imageBytes > CBZImagePipeline.maximumCacheBytes - cost
          {
            cell.budgetDeferred = true
            return
          }
          let others = self.cells.keys.filter { $0 != index }.sorted {
            abs($0 - (preserved?.index ?? index)) > abs($1 - (preserved?.index ?? index))
          }
          var held = self.cells.values.reduce(0) { $0 + $1.imageBytes } - cell.imageBytes
          for other in others where held > CBZImagePipeline.maximumCacheBytes - cost {
            guard let obsolete = self.cells[other] else { continue }
            held -= obsolete.imageBytes
            obsolete.releaseImage()
          }
          cell.show(decoded, requestedWidth: width)
          if index == preserved?.index { reader.setContinuousReady(true) }
          let ratio = CGFloat(decoded.sourceHeight) / CGFloat(decoded.sourceWidth)
          if self.ratios[index] != ratio {
            self.ratios[index] = ratio
            self.rebuild(
              preserving: preserved, restorePosition: index <= (preserved?.index ?? index))
          }
          self.setNeedsLayout()
        } catch is CancellationError {
        } catch {
          guard let self, self.requests[index]?.id == id, let cell else { return }
          self.requests[index] = nil
          if cell.imageView.image == nil {
            cell.showFailure(error.localizedDescription) { [weak self, weak cell] in
              cell?.failed = false
              self?.setNeedsLayout()
            }
          } else {
            // A zoom request can exceed the budget while its base image remains usable.
            cell.fullResolution = true
            reader.message = error.localizedDescription
          }
        }
      }
      requests[index] = (id, request)
    }
    let ready = cells[anchor.index]?.imageView.image != nil
    guard ready != reader.isReadyForInteraction else { return }
    let generation = reader.navigationRequestID
    Task { @MainActor [weak reader] in
      guard reader?.navigationRequestID == generation else { return }
      reader?.setContinuousReady(ready)
    }
  }

  func viewForZooming(in scrollView: UIScrollView) -> UIView? { canvas }

  func scrollViewWillBeginDragging(_ scrollView: UIScrollView) { onInteraction?() }

  func scrollViewWillBeginZooming(_ scrollView: UIScrollView, with view: UIView?) {
    onInteraction?()
  }

  func scrollViewDidZoom(_ scrollView: UIScrollView) {
    guard !updating else { return }
    centerCanvas()
    publishPosition()
  }

  func scrollViewDidEndZooming(
    _ scrollView: UIScrollView, with view: UIView?, atScale scale: CGFloat
  ) {
    updateVisiblePages()
    publishPosition(force: true)
  }

  func scrollViewDidScroll(_ scrollView: UIScrollView) {
    guard !updating else { return }
    updateVisiblePages()
    publishPosition()
  }

  func scrollViewDidEndDragging(_ scrollView: UIScrollView, willDecelerate decelerate: Bool) {
    if !decelerate { publishPosition(force: true) }
  }

  func scrollViewDidEndDecelerating(_ scrollView: UIScrollView) { publishPosition(force: true) }

  private func publishPosition(force: Bool = false) {
    guard !updating, let (pageID, fraction) = readingPosition else { return }
    lastAnchor = anchor()
    let now = ProcessInfo.processInfo.systemUptime
    guard force || now - lastPublished >= 0.2 else { return }
    lastPublished = now
    let generation = reader?.navigationRequestID
    Task { @MainActor [weak reader] in
      guard reader?.navigationRequestID == generation else { return }
      reader?.recordPosition(pageID: pageID, progression: fraction, force: force)
    }
  }
}

@MainActor private final class ComicRollPage: UIView {
  let imageView = UIImageView()
  private let spinner = UIActivityIndicatorView(style: .medium)
  private let retryButton = UIButton(type: .system)
  var renderWidth = 0
  var fullResolution = false
  var failed = false
  var budgetDeferred = false
  var imageBytes: Int {
    guard let image = imageView.image?.cgImage else { return 0 }
    return image.bytesPerRow * image.height
  }

  init() {
    super.init(frame: .zero)
    imageView.contentMode = .scaleToFill
    imageView.isAccessibilityElement = true
    addSubview(imageView)
    spinner.startAnimating()
    addSubview(spinner)
    retryButton.titleLabel?.numberOfLines = 0
    retryButton.isHidden = true
    addSubview(retryButton)
  }

  required init?(coder: NSCoder) { nil }

  override func layoutSubviews() {
    super.layoutSubviews()
    imageView.frame = bounds
    spinner.center = CGPoint(x: bounds.midX, y: bounds.midY)
    retryButton.frame = CGRect(
      x: 16, y: max(16, bounds.midY - 80), width: max(0, bounds.width - 32), height: 160)
  }

  func show(_ page: CBZDecodedPage, requestedWidth: Int) {
    imageView.image = UIImage(cgImage: page.image)
    renderWidth = requestedWidth
    fullResolution = page.image.width >= page.sourceWidth
    failed = false
    budgetDeferred = false
    spinner.stopAnimating()
    retryButton.isHidden = true
  }

  func releaseImage() {
    imageView.image = nil
    renderWidth = 0
    fullResolution = false
    budgetDeferred = true
    spinner.startAnimating()
  }

  func showFailure(_ message: String, retry: @escaping () -> Void) {
    failed = true
    spinner.stopAnimating()
    retryButton.setTitle(
      message + "\n" + String(localized: "Try again", bundle: .module), for: .normal)
    let actionID = UIAction.Identifier("retry-page")
    retryButton.removeAction(identifiedBy: actionID, for: .touchUpInside)
    retryButton.addAction(UIAction(identifier: actionID) { _ in retry() }, for: .touchUpInside)
    retryButton.isHidden = false
  }
}
