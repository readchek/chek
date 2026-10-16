#if os(macOS)
  import AppKit
  import SwiftUI

  /// The model and image pipeline are shared with iOS. AppKit owns only scrolling,
  /// magnification, the visible image views, and their cancellable decode requests.
  struct MacComicCanvas: NSViewRepresentable {
    let reader: CBZReader
    let onTap: () -> Void
    let onInteraction: () -> Void

    func makeNSView(context: Context) -> MacComicScrollView { MacComicScrollView() }

    func updateNSView(_ view: MacComicScrollView, context: Context) {
      view.onTap = onTap
      view.onInteraction = onInteraction
      view.configure(reader)
      reader.setPositionProvider(owner: view) { [weak view] in view?.readingPosition }
    }

    static func dismantleNSView(_ view: MacComicScrollView, coordinator: ()) { view.stop() }
  }

  @MainActor final class MacComicScrollView: NSScrollView {
    private struct Anchor {
      let index: Int
      let fraction: CGFloat
    }

    private final class Canvas: NSView {
      override var isFlipped: Bool { true }
    }

    private let canvas = Canvas()
    private weak var reader: CBZReader?
    private var cells: [Int: MacComicPage] = [:]
    private var requests: [Int: (id: UUID, task: Task<Void, Never>)] = [:]
    private var ratios: [Int: CGFloat] = [:]
    private var starts: [CGFloat] = []
    private var navigationID: UUID?
    private var retryID: UUID?
    private var cacheReleaseID: UUID?
    private var mode: CBZReadingMode = .paged
    private var previousSize = CGSize.zero
    private var updating = false
    private var pendingAnchor: Anchor?
    private var unresolvedAnchor: Anchor?
    private var lastAnchor: Anchor?
    private var lastPublished = 0.0
    private var positionTask: Task<Void, Never>?
    private var requestedWidth = 0
    private var horizontalGesture = CGFloat.zero
    private var changedPageDuringGesture = false
    var onTap: (() -> Void)?
    var onInteraction: (() -> Void)?

    var readingPosition: (String, Double)? {
      guard let reader, let anchor = anchor(),
        reader.publication.pages.indices.contains(anchor.index)
      else { return nil }
      return (reader.publication.pages[anchor.index].id, Double(anchor.fraction))
    }

    override var acceptsFirstResponder: Bool { true }

    init() {
      super.init(frame: .zero)
      drawsBackground = true
      backgroundColor = .textBackgroundColor
      hasVerticalScroller = true
      hasHorizontalScroller = true
      autohidesScrollers = true
      allowsMagnification = true
      minMagnification = 1
      maxMagnification = 4
      documentView = canvas
      contentView.postsBoundsChangedNotifications = true
      NotificationCenter.default.addObserver(
        self, selector: #selector(clipBoundsChanged), name: NSView.boundsDidChangeNotification,
        object: contentView)
    }

    required init?(coder: NSCoder) { nil }

    func configure(_ reader: CBZReader) {
      if self.reader !== reader {
        reset()
        self.reader = reader
      }
      if mode != reader.effectiveReadingMode {
        releaseCells()
        starts.removeAll()
        requestedWidth = 0
        mode = reader.effectiveReadingMode
        navigationID = nil
      }
      if retryID != reader.loadRequestID {
        retryID = reader.loadRequestID
        for cell in cells.values { cell.prepareRetry() }
      }
      if cacheReleaseID != reader.cacheReleaseID {
        cacheReleaseID = reader.cacheReleaseID
        for index in Array(cells.keys) where !isVisible(index) {
          cells.removeValue(forKey: index)?.removeFromSuperview()
          requests.removeValue(forKey: index)?.task.cancel()
        }
      }
      if navigationID != reader.navigationRequestID {
        positionTask?.cancel()
        positionTask = nil
        lastPublished = 0
        navigationID = reader.navigationRequestID
        pendingAnchor = Anchor(
          index: reader.currentPageIndex, fraction: reader.location.progression)
        unresolvedAnchor = pendingAnchor
        if mode == .paged {
          releaseCells()
          updating = true
          magnification = 1
          updating = false
        }
      }
      needsLayout = true
    }

    func stop() {
      reset()
      onTap = nil
      onInteraction = nil
    }

    private func releaseCells() {
      for request in requests.values { request.task.cancel() }
      requests.removeAll()
      for cell in cells.values { cell.removeFromSuperview() }
      cells.removeAll()
    }

    private func reset() {
      positionTask?.cancel()
      positionTask = nil
      lastPublished = 0
      reader?.clearPositionProvider(owner: self)
      reader = nil
      releaseCells()
      ratios.removeAll()
      starts.removeAll()
      navigationID = nil
      retryID = nil
      cacheReleaseID = nil
      pendingAnchor = nil
      unresolvedAnchor = nil
      lastAnchor = nil
      requestedWidth = 0
      previousSize = .zero
      horizontalGesture = 0
      changedPageDuringGesture = false
      updating = true
      magnification = 1
      canvas.frame = .zero
      updating = false
    }

    override func layout() {
      guard !updating else {
        super.layout()
        return
      }
      let preserved = pendingAnchor ?? lastAnchor ?? anchor()
      updating = true
      super.layout()
      updating = false
      guard contentSize.width > 0, contentSize.height > 0, let reader else { return }
      let resized = previousSize != contentSize
      if resized || pendingAnchor != nil || starts.isEmpty {
        previousSize = contentSize
        pendingAnchor = nil
        rebuild(preserving: preserved)
      }
      if mode == .paged {
        showPaged(reader)
      } else {
        updateVisiblePages()
      }
    }

    private func anchor() -> Anchor? {
      if let unresolvedAnchor { return unresolvedAnchor }
      guard let reader else { return nil }
      if mode == .paged {
        let height = cells[reader.currentPageIndex]?.frame.height ?? 0
        guard height > 0 else { return pendingAnchor }
        let y = max(0, contentView.bounds.minY - (cells[reader.currentPageIndex]?.frame.minY ?? 0))
        return Anchor(index: reader.currentPageIndex, fraction: min(1, y / height))
      }
      guard starts.count == reader.publication.pages.count + 1 else { return pendingAnchor }
      let y = min(
        max(0, canvas.bounds.height - contentView.bounds.height), max(0, contentView.bounds.minY))
      var lower = 0
      var upper = reader.publication.pages.count
      while lower < upper {
        let middle = (lower + upper) / 2
        if starts[middle + 1] <= y { lower = middle + 1 } else { upper = middle }
      }
      let index = min(reader.publication.pages.count - 1, lower)
      return Anchor(
        index: index,
        fraction: min(1, max(0, (y - starts[index]) / max(1, starts[index + 1] - starts[index]))))
    }

    private func rebuild(preserving anchor: Anchor?) {
      guard !updating, let reader, contentSize.width > 0 else { return }
      updating = true
      defer { updating = false }
      let width = contentSize.width
      if mode == .continuous {
        starts = [0]
        starts.reserveCapacity(reader.publication.pages.count + 1)
        for index in reader.publication.pages.indices {
          starts.append(starts[index] + width * (ratios[index] ?? 3.5))
        }
        canvas.frame = CGRect(
          x: 0, y: 0, width: width, height: max(contentView.bounds.height, starts.last ?? 0))
        for (index, cell) in cells { cell.frame = pageFrame(index) }
      } else if let decoded = reader.decodedPage {
        let height = width * CGFloat(decoded.sourceHeight) / CGFloat(decoded.sourceWidth)
        canvas.frame = CGRect(
          x: 0, y: 0, width: width, height: max(contentView.bounds.height, height))
        cells[reader.currentPageIndex]?.frame = CGRect(
          x: 0, y: max(0, (contentView.bounds.height - height) / 2), width: width, height: height)
      } else {
        canvas.frame = CGRect(origin: .zero, size: contentSize)
      }
      if let anchor {
        let y: CGFloat
        if mode == .continuous, starts.indices.contains(anchor.index + 1) {
          y =
            starts[anchor.index] + (starts[anchor.index + 1] - starts[anchor.index])
            * anchor.fraction
          if ratios[anchor.index] != nil { unresolvedAnchor = nil }
        } else if let cell = cells[anchor.index], cell.image != nil {
          y = cell.frame.minY + cell.frame.height * anchor.fraction
          unresolvedAnchor = nil
        } else {
          return
        }
        contentView.scroll(
          to: CGPoint(
            x: min(contentView.bounds.minX, max(0, width - contentView.bounds.width)),
            y: min(max(0, y), max(0, canvas.bounds.height - contentView.bounds.height))))
        reflectScrolledClipView(contentView)
        lastAnchor = self.anchor()
      }
    }

    private func pageFrame(_ index: Int) -> CGRect {
      CGRect(
        x: 0, y: starts[index], width: contentSize.width, height: starts[index + 1] - starts[index])
    }

    private func showPaged(_ reader: CBZReader) {
      let width = pixelWidth
      if width != requestedWidth {
        requestedWidth = width
        Task { @MainActor [weak self, weak reader] in
          guard let self, self.reader === reader else { return }
          reader?.requestWidth(
            viewport: Int(ceil(self.contentSize.width * self.displayScale)),
            zoom: self.magnification)
        }
      }
      guard let decoded = reader.decodedPage else { return }
      let index = reader.currentPageIndex
      let cell = cells[index] ?? makeCell(index)
      guard cell.image !== decoded.image else { return }
      let preserved = unresolvedAnchor ?? lastAnchor ?? anchor()
      cell.show(decoded, width: width)
      rebuild(preserving: preserved)
    }

    private var displayScale: CGFloat { window?.backingScaleFactor ?? 2 }
    private var pixelWidth: Int {
      Int(min(4096, ceil(contentSize.width * displayScale * magnification)))
    }

    private func makeCell(_ index: Int) -> MacComicPage {
      let cell = MacComicPage()
      cell.imageView.setAccessibilityLabel(
        String(
          localized: "Comic page \(index + 1) of \(reader?.publication.pages.count ?? 0)",
          bundle: .module))
      cells[index] = cell
      canvas.addSubview(cell)
      return cell
    }

    private func isVisible(_ index: Int) -> Bool {
      guard starts.indices.contains(index + 1) else { return false }
      return starts[index + 1] > contentView.bounds.minY && starts[index] < contentView.bounds.maxY
    }

    private func updateVisiblePages() {
      guard !updating, let reader, let anchor = anchor(),
        starts.count == reader.publication.pages.count + 1
      else { return }
      var upper = anchor.index
      while upper + 1 < reader.publication.pages.count, starts[upper + 1] < contentView.bounds.maxY
      { upper += 1 }
      let visible = Set(anchor.index...upper)
      guard visible.count <= 256 else {
        for request in requests.values { request.task.cancel() }
        requests.removeAll()
        publishFailure(
          "Too many short pages fit in one view. Choose paged mode to read this comic.")
        return
      }
      var desired = visible
      if visible.count < 5 {
        if anchor.index > 0 { desired.insert(anchor.index - 1) }
        if upper + 1 < reader.publication.pages.count { desired.insert(upper + 1) }
      }
      let readableWidth = min(4096, contentSize.width * magnification)
      func budgetWidth(_ indices: Set<Int>) -> CGFloat {
        let area = indices.reduce(CGFloat.zero) { $0 + (ratios[$1] ?? 3.5) }
        return sqrt(CGFloat(CBZImagePipeline.maximumCacheBytes / 8) / max(1, area))
      }
      if budgetWidth(desired) < readableWidth { desired = visible }
      let width = max(
        Int(ceil(readableWidth / 128)) * 128,
        Int(min(CGFloat(pixelWidth), budgetWidth(desired)) / 128) * 128)
      guard width > 0 else { return }
      for index in Array(cells.keys) where !desired.contains(index) {
        cells.removeValue(forKey: index)?.removeFromSuperview()
      }
      for index in Array(requests.keys) where !desired.contains(index) {
        requests.removeValue(forKey: index)?.task.cancel()
      }
      if requestedWidth != width {
        requestedWidth = width
        for request in requests.values { request.task.cancel() }
        requests.removeAll()
      }
      for cell in cells.values where (cell.image?.width ?? 0) > width { cell.releaseImage() }
      let priority =
        [anchor.index] + visible.subtracting([anchor.index]).sorted()
        + desired.subtracting(visible).sorted { abs($0 - anchor.index) < abs($1 - anchor.index) }
      for index in priority {
        let cell = cells[index] ?? makeCell(index)
        cell.frame = pageFrame(index)
        cell.visibleRectInCanvas = contentView.bounds
        guard requests.count < 2, requests[index] == nil, !cell.failed,
          !cell.budgetDeferred || visible.contains(index), cell.renderWidth < width,
          !cell.fullResolution
        else { continue }
        let id = UUID()
        let pageID = reader.publication.pages[index].id
        let request = Task { [weak self, weak reader, weak cell] in
          guard let reader else { return }
          do {
            let decoded = try await reader.image(
              pageID: pageID, width: width, minimumWidth: Int(ceil(readableWidth)))
            try Task.checkCancellation()
            guard let self, self.reader === reader, self.requests[index]?.id == id, let cell else {
              return
            }
            self.requests[index] = nil
            let preserved = self.anchor()
            let ratio = CGFloat(decoded.sourceHeight) / CGFloat(decoded.sourceWidth)
            let learned = self.ratios[index] != ratio
            if learned {
              self.ratios[index] = ratio
              self.rebuild(preserving: preserved)
            }
            let cost = decoded.image.bytesPerRow * decoded.image.height
            var held = self.cells.values.reduce(0) { $0 + $1.imageBytes } - cell.imageBytes
            let obsolete = self.cells.keys.filter { $0 != index && !self.isVisible($0) }.sorted {
              abs($0 - index) > abs($1 - index)
            }
            for other in obsolete where held > CBZImagePipeline.maximumCacheBytes - cost {
              guard let otherCell = self.cells[other] else { continue }
              held -= otherCell.imageBytes
              otherCell.releaseImage()
            }
            guard held <= CBZImagePipeline.maximumCacheBytes - cost else {
              if !learned, self.isVisible(index) {
                cell.showFailure(
                  String(
                    localized: "These pages exceed the visible image memory budget.",
                    bundle: .module)
                ) { [weak self, weak cell] in
                  cell?.prepareRetry()
                  self?.needsLayout = true
                }
              } else if !learned {
                cell.budgetDeferred = true
              }
              self.needsLayout = true
              return
            }
            cell.show(decoded, width: width)
            if index == self.anchor()?.index { reader.setContinuousReady(true) }
            self.publishPosition(force: true)
            self.needsLayout = true
          } catch is CancellationError {
          } catch {
            guard let self, self.reader === reader, self.requests[index]?.id == id, let cell else {
              return
            }
            self.requests[index] = nil
            if cell.image == nil {
              cell.showFailure(error.localizedDescription) { [weak self, weak cell] in
                cell?.prepareRetry()
                self?.needsLayout = true
              }
            } else {
              cell.fullResolution = true
              reader.message = error.localizedDescription
            }
            self.needsLayout = true
          }
        }
        requests[index] = (id, request)
      }
      let generation = reader.navigationRequestID
      Task { @MainActor [weak self, weak reader] in
        guard let self, let reader, self.reader === reader,
          reader.navigationRequestID == generation, let anchor = self.anchor()
        else { return }
        reader.setContinuousReady(self.cells[anchor.index]?.image != nil)
      }
    }

    private func publishFailure(_ message: String) {
      let generation = reader?.navigationRequestID
      Task { @MainActor [weak self, weak reader] in
        guard let self, let reader, self.reader === reader, reader.navigationRequestID == generation
        else { return }
        reader.failContinuous(message)
      }
    }

    override func reflectScrolledClipView(_ clipView: NSClipView) {
      super.reflectScrolledClipView(clipView)
      clipBoundsChanged()
    }

    /// Clip bounds cover wheel, scrollbar, keyboard and accessibility scrolling.
    /// Wheel phases alone do not include every native scroll path.
    @objc private func clipBoundsChanged() {
      guard !updating else { return }
      if mode == .continuous { updateVisiblePages() }
      publishPosition()
    }

    private func publishPosition(force: Bool = false) {
      guard !updating, readingPosition != nil else { return }
      lastAnchor = anchor()
      let now = ProcessInfo.processInfo.systemUptime
      let delay = force ? 0 : max(0, 0.2 - (now - lastPublished))
      if positionTask != nil {
        guard force else { return }
        positionTask?.cancel()
      }
      let generation = reader?.navigationRequestID
      // One trailing publication reads the latest anchor when it fires. Dropping
      // events within the throttle window loses the final position for discrete
      // wheel events, which often have no ended phase.
      positionTask = Task { @MainActor [weak self, weak reader] in
        do {
          if delay > 0 { try await Task.sleep(for: .seconds(delay)) }
          try Task.checkCancellation()
        } catch { return }
        guard let self, let reader, self.reader === reader,
          reader.navigationRequestID == generation,
          let (pageID, fraction) = self.readingPosition
        else { return }
        self.positionTask = nil
        self.lastAnchor = self.anchor()
        self.lastPublished = ProcessInfo.processInfo.systemUptime
        reader.recordPosition(pageID: pageID, progression: fraction, force: force)
      }
    }

    override func mouseDown(with event: NSEvent) {
      window?.makeFirstResponder(self)
      onTap?()
      super.mouseDown(with: event)
    }

    override func magnify(with event: NSEvent) {
      unresolvedAnchor = nil
      onInteraction?()
      super.magnify(with: event)
      needsLayout = true
      publishPosition(force: event.phase == .ended)
    }

    override func scrollWheel(with event: NSEvent) {
      unresolvedAnchor = nil
      onInteraction?()
      if event.phase == .began {
        horizontalGesture = 0
        changedPageDuringGesture = false
      }
      if mode == .paged, magnification <= 1.01, event.hasPreciseScrollingDeltas,
        abs(event.scrollingDeltaX) > abs(event.scrollingDeltaY), event.momentumPhase.isEmpty
      {
        horizontalGesture += event.scrollingDeltaX
        if abs(horizontalGesture) > 60, !changedPageDuringGesture {
          changedPageDuringGesture = navigate(left: horizontalGesture < 0)
        }
        if changedPageDuringGesture { return }
      }
      super.scrollWheel(with: event)
      publishPosition(force: event.phase == .ended || event.momentumPhase == .ended)
    }

    override func swipe(with event: NSEvent) {
      if mode == .paged, magnification <= 1.01, event.deltaX != 0,
        navigate(left: event.deltaX > 0)
      {
        return
      }
      super.swipe(with: event)
    }

    private func navigate(left: Bool) -> Bool {
      guard let reader, reader.allowsPageSwipe else { return false }
      let delta = left == (reader.readingDirection == .leftToRight) ? 1 : -1
      guard reader.publication.pages.indices.contains(reader.currentPageIndex + delta) else {
        return false
      }
      onInteraction?()
      reader.movePage(by: delta)
      return true
    }

    override func keyDown(with event: NSEvent) {
      if mode == .paged, magnification <= 1.01,
        event.modifierFlags.intersection([.command, .control, .option]).isEmpty
      {
        if event.keyCode == 124, navigate(left: true) { return }
        if event.keyCode == 123, navigate(left: false) { return }
      }
      super.keyDown(with: event)
    }
  }

  @MainActor private final class MacComicPage: NSView {
    let imageView = NSImageView()
    private let spinner = NSProgressIndicator()
    private let retryButton = NSButton()
    private var retry: (() -> Void)?
    private(set) var image: CGImage?
    var renderWidth = 0
    var fullResolution = false
    var failed = false
    var budgetDeferred = false
    var visibleRectInCanvas = CGRect.zero { didSet { needsLayout = true } }
    var imageBytes: Int { image.map { $0.bytesPerRow * $0.height } ?? 0 }
    override var isFlipped: Bool { true }

    init() {
      super.init(frame: .zero)
      imageView.imageScaling = .scaleAxesIndependently
      imageView.setAccessibilityElement(true)
      imageView.setAccessibilityRole(.image)
      addSubview(imageView)
      spinner.style = .spinning
      spinner.controlSize = .small
      spinner.isDisplayedWhenStopped = false
      spinner.startAnimation(nil)
      addSubview(spinner)
      retryButton.target = self
      retryButton.action = #selector(retryPage)
      retryButton.bezelStyle = .rounded
      retryButton.cell?.wraps = true
      retryButton.cell?.isScrollable = false
      retryButton.isHidden = true
      addSubview(retryButton)
    }

    required init?(coder: NSCoder) { nil }

    override func layout() {
      super.layout()
      imageView.frame = bounds
      let visible = frame.intersection(visibleRectInCanvas)
      let y = visible.isNull ? bounds.midY : visible.midY - frame.minY
      spinner.frame = CGRect(x: bounds.midX - 10, y: y - 10, width: 20, height: 20)
      retryButton.frame = CGRect(
        x: 16, y: max(0, y - 40), width: max(0, bounds.width - 32), height: 80)
    }

    func show(_ decoded: CBZDecodedPage, width: Int) {
      image = decoded.image
      imageView.image = NSImage(
        cgImage: decoded.image,
        size: CGSize(width: decoded.sourceWidth, height: decoded.sourceHeight))
      renderWidth = width
      fullResolution = decoded.image.width >= decoded.sourceWidth
      failed = false
      budgetDeferred = false
      retry = nil
      spinner.stopAnimation(nil)
      retryButton.isHidden = true
    }

    func releaseImage() {
      image = nil
      imageView.image = nil
      renderWidth = 0
      fullResolution = false
      budgetDeferred = true
      spinner.startAnimation(nil)
    }

    func prepareRetry() {
      failed = false
      fullResolution = false
      budgetDeferred = false
      retry = nil
      retryButton.isHidden = true
      if image == nil { spinner.startAnimation(nil) }
    }

    func showFailure(_ message: String, retry: @escaping () -> Void) {
      failed = true
      self.retry = retry
      spinner.stopAnimation(nil)
      retryButton.title = message + "\n" + String(localized: "Try again", bundle: .module)
      retryButton.isHidden = false
    }

    @objc private func retryPage() { retry?() }
  }
#endif
