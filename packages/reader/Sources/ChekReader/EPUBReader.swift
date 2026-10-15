import Foundation
import Observation
import SwiftUI
import UIKit
import WebKit

#if DEBUG
  import OSLog
#endif

@MainActor @Observable
public final class EPUBReader {
  public let publication: EPUBPublication
  public private(set) var currentHref: String
  public private(set) var requestedHref: String
  public private(set) var location: ReadingLocation?
  public private(set) var textScale: Double
  public var message: String?
  private(set) var page: WebPage?
  public private(set) var loadFailed = false
  public private(set) var failureDescription: String?
  /// Disable when the host supplies its own chapter navigation gestures.
  public var allowsChapterSwipe: Bool
  private(set) var fixedViewport: FixedPageViewport?
  private(set) var loadRequestID = UUID()
  private var resumeLocation: ReadingLocation?
  private var documentToken = ""
  private var isRestoring = true
  private var explicitLoad = false
  private var appliedTextScale: Double = 1
  private var scaleOperationID: UUID?
  private var resourceFailure: String?
  private var topInset: CGFloat = 0
  private var bottomInset: CGFloat = 0
  @ObservationIgnored fileprivate var onTap: @MainActor () -> Void = {}
  @ObservationIgnored fileprivate var onInteraction: @MainActor () -> Void = {}
  @ObservationIgnored fileprivate var callbackOwner: UUID?

  public var hasMessage: Bool {
    get { message != nil }
    set { if !newValue { message = nil } }
  }

  public init(
    publication: EPUBPublication, location: ReadingLocation? = nil, textScale: Double = 1,
    allowsChapterSwipe: Bool = true
  ) {
    self.publication = publication
    self.allowsChapterSwipe = allowsChapterSwipe
    let validLocation = location.flatMap { anchor in
      anchor.isValid && (try? publication.url(for: anchor.resourceHref)) != nil ? anchor : nil
    }
    resumeLocation = validLocation
    self.location = validLocation
    self.textScale = textScale.isFinite ? min(2, max(0.75, textScale)) : 1
    let startingHref = validLocation?.resourceHref ?? publication.readingOrder[0].href
    currentHref = startingHref
    requestedHref = startingHref
  }

  public var chapterIndex: Int? {
    let path = currentHref.split(separator: "#", maxSplits: 1).first.map(String.init)
    return publication.readingOrder.firstIndex(where: { $0.href == path })
  }

  public var isFixedLayout: Bool { publication.isFixedLayout(currentHref) }

  /// False while loading/restoring, or when an error replaces the document.
  /// Consumers should keep an exit control visible until DOM gestures are ready.
  public var isReadyForInteraction: Bool { !isRestoring && !loadFailed && !explicitLoad }

  public func open(_ link: EPUBLink) {
    requestedHref = link.href
    loadRequestID = UUID()
  }

  /// Retry the requested document, retaining the latest captured position when applicable.
  public func retry() {
    resumeLocation = location ?? resumeLocation
    loadRequestID = UUID()
  }

  /// Delta follows logical spine order, independently of physical swipe direction.
  public func moveChapter(by delta: Int) {
    guard let index = chapterIndex else { return }
    let (destination, overflow) = index.addingReportingOverflow(delta)
    guard !overflow, publication.readingOrder.indices.contains(destination) else {
      return
    }
    open(publication.readingOrder[destination])
  }

  func load() async {
    let generation = loadRequestID
    scaleOperationID = nil
    explicitLoad = true
    loadFailed = false
    failureDescription = nil
    resourceFailure = nil
    do {
      if page != nil { _ = await captureLocation() }
      let viewport = try await publication.fixedViewport(for: requestedHref)
      guard generation == loadRequestID else { return }
      fixedViewport = viewport
      if page == nil {
        let preparedPage = try await makePage()
        try Task.checkCancellation()
        guard generation == loadRequestID else { return }
        page = preparedPage
      }
      try Task.checkCancellation()
      guard generation == loadRequestID, let page else { return }
      let url = try publication.url(for: requestedHref)
      let anchor = resumeLocation
      documentToken = UUID().uuidString
      isRestoring = true
      for try await _ in page.load(url) { try Task.checkCancellation() }
      guard generation == loadRequestID else { return }
      let activatedScale = textScale
      let value = try await page.callJavaScript(
        "return await window.chekReader.activate(token, anchor, scale, top, bottom, fixed);",
        arguments: [
          "token": documentToken, "anchor": try jsonValue(anchor), "scale": activatedScale,
          "top": Double(topInset), "bottom": Double(bottomInset), "fixed": viewport != nil,
        ], contentWorld: .world(name: "ChekReader"))
      guard generation == loadRequestID else { return }
      try Task.checkCancellation()
      if let failure = (value as? [String: Any])?["documentError"] as? String {
        throw ReaderError.invalid(
          "The publication document is malformed XML. \(failure.prefix(2048))")
      }
      resumeLocation = nil
      isRestoring = false
      explicitLoad = false
      appliedTextScale = activatedScale
      acceptLocation(value)
      if let resourceFailure, message == nil { message = resourceFailure }
      await applyRequestedTextScale()
    } catch is CancellationError {
    } catch {
      if !Task.isCancelled, generation == loadRequestID {
        explicitLoad = false
        loadFailed = true
        failureDescription = resourceFailure ?? navigationFailureDescription(error)
      }
    }
  }

  /// Capture synchronously from the current DOM before close/background saves.
  @discardableResult public func captureLocation() async -> ReadingLocation? {
    guard !isRestoring, let page else { return location }
    let token = documentToken
    let generation = loadRequestID
    do {
      let value = try await page.callJavaScript(
        "return window.chekReader.capture();",
        contentWorld: .world(name: "ChekReader"))
      if token == documentToken, generation == loadRequestID { acceptLocation(value) }
    } catch {
      if !Task.isCancelled, token == documentToken, generation == loadRequestID {
        message = error.localizedDescription
      }
    }
    return location
  }

  fileprivate func navigationFailed(_ error: any Error) {
    guard !explicitLoad else { return }
    loadFailed = true
    failureDescription = navigationFailureDescription(error)
  }

  private func navigationFailureDescription(_ error: any Error) -> String {
    guard let navigationError = error as? WebPage.NavigationError else {
      return error.localizedDescription
    }
    switch navigationError {
    case .failedProvisionalNavigation(let underlying):
      let details = underlying as NSError
      return "\(underlying.localizedDescription) (\(details.domain), \(details.code))"
    case .pageClosed:
      return String(
        localized: "The book's web page was closed. Try again to reopen it.", bundle: .module)
    case .webContentProcessTerminated:
      return String(
        localized: "The book's web content process ended. Try again to reload this page.",
        bundle: .module)
    case .invalidURL:
      return String(localized: "The book's page address is invalid.", bundle: .module)
    @unknown default:
      return error.localizedDescription
    }
  }

  public func setTextScale(_ scale: Double) async {
    guard scale.isFinite else { return }
    let boundedScale = min(2, max(0.75, scale))
    textScale = boundedScale
    await applyRequestedTextScale()
  }

  private func applyRequestedTextScale() async {
    guard scaleOperationID == nil, !isRestoring, !explicitLoad, !loadFailed,
      !isFixedLayout, appliedTextScale != textScale, let page
    else { return }
    let operationID = UUID()
    scaleOperationID = operationID
    defer {
      if scaleOperationID == operationID { scaleOperationID = nil }
    }
    let generation = loadRequestID
    let token = documentToken
    _ = await captureLocation()
    guard generation == loadRequestID, token == documentToken, !Task.isCancelled else { return }
    isRestoring = true
    do {
      // A slider can update while WebKit settles fonts. Keep one operation in
      // flight and apply the newest request before releasing restoration.
      while appliedTextScale != textScale {
        let requestedScale = textScale
        let value = try await page.callJavaScript(
          "return await window.chekReader.setScale(scale);",
          arguments: ["scale": requestedScale], contentWorld: .world(name: "ChekReader"))
        guard generation == loadRequestID, token == documentToken else { return }
        appliedTextScale = requestedScale
        acceptLocation(value)
      }
      isRestoring = false
    } catch {
      guard generation == loadRequestID, token == documentToken else { return }
      isRestoring = false
      if !Task.isCancelled { message = error.localizedDescription }
    }
  }

  func setInsets(top: CGFloat, bottom: CGFloat) async {
    guard topInset != top || bottomInset != bottom else { return }
    topInset = top
    bottomInset = bottom
    guard let page else { return }
    _ = try? await page.callJavaScript(
      "window.chekReader?.setInsets(top, bottom);",
      arguments: ["top": Double(top), "bottom": Double(bottom)],
      contentWorld: .world(name: "ChekReader"))
  }

  private func jsonValue(_ anchor: ReadingLocation?) throws -> Any {
    guard let anchor else { return NSNull() }
    return try JSONSerialization.jsonObject(with: JSONEncoder().encode(anchor))
  }

  private func acceptLocation(_ value: Any?) {
    guard let value, JSONSerialization.isValidJSONObject(value),
      let data = try? JSONSerialization.data(withJSONObject: value), data.count <= 16 * 1024,
      let anchor = try? JSONDecoder().decode(ReadingLocation.self, from: data), anchor.isValid,
      let url = try? publication.url(for: anchor.resourceHref)
    else { return }
    let canonical = EPUBPublication.linkHref(url).split(separator: "#", maxSplits: 1).first.map(
      String.init)
    guard canonical == anchor.resourceHref,
      canonical == currentHref.split(separator: "#", maxSplits: 1).first.map(String.init)
    else { return }
    location = anchor
  }

  fileprivate func receive(_ message: WKScriptMessage) {
    #if DEBUG
      if message.name == "contentPolicy" {
        guard message.frameInfo.isMainFrame,
          message.frameInfo.request.url?.host == publication.host,
          let body = message.body as? [String: String],
          let directive = body["directive"], directive.count <= 80,
          let disposition = body["disposition"],
          disposition == "enforce" || disposition == "report",
          let blocked = body["blocked"], blocked.count <= 4096
        else { return }
        let url = URL(string: blocked)
        let scheme = url?.scheme ?? "inline-or-unknown"
        let host = url?.host ?? "none"
        Logger(subsystem: "dev.chek.reader", category: "ContentPolicy").notice(
          "CSP directive=\(directive, privacy: .public) disposition=\(disposition, privacy: .public) scheme=\(scheme, privacy: .public) host=\(host, privacy: .public)"
        )
        return
      }
    #endif
    guard !isRestoring, message.frameInfo.isMainFrame,
      message.frameInfo.request.url?.host == publication.host,
      let body = message.body as? [String: Any], body["token"] as? String == documentToken
    else { return }
    if message.name == "readerInteraction", let kind = body["kind"] as? String {
      switch kind {
      case "tap": onTap()
      case "active": onInteraction()
      case "swipeLeft", "swipeRight":
        onInteraction()
        if allowsChapterSwipe {
          let leftToRightDelta = kind == "swipeLeft" ? 1 : -1
          moveChapter(
            by: publication.pageProgressionDirection == .rightToLeft
              ? -leftToRightDelta : leftToRightDelta)
        }
      default: break
      }
      return
    }
    #if DEBUG
      if let duration = body["captureMilliseconds"] as? Double, duration.isFinite, duration > 8 {
        Logger(subsystem: "dev.chek.reader", category: "ReadingPerformance").notice(
          "Semantic capture took \(duration, privacy: .public) ms")
      }
    #endif
    acceptLocation(body["location"])
  }

  private func makePage() async throws -> WebPage {
    let controller = WKUserContentController()
    guard let scriptURL = Bundle.module.url(forResource: "ReaderDocument", withExtension: "js")
    else {
      throw ReaderError.invalid("The reader document bridge is missing.")
    }
    let viewport = try String(contentsOf: scriptURL, encoding: .utf8)
    controller.add(
      ReaderBridge(reader: self), contentWorld: .world(name: "ChekReader"), name: "readingLocation")
    controller.add(
      ReaderBridge(reader: self), contentWorld: .world(name: "ChekReader"),
      name: "readerInteraction")
    controller.addUserScript(
      WKUserScript(
        source: viewport, injectionTime: .atDocumentStart,
        forMainFrameOnly: true, in: .world(name: "ChekReader")))
    #if DEBUG
      controller.add(
        ReaderBridge(reader: self), contentWorld: .world(name: "ChekReader"), name: "contentPolicy")
      controller.addUserScript(
        WKUserScript(
          source: """
            document.addEventListener('securitypolicyviolation', event => {
              window.webkit.messageHandlers.contentPolicy.postMessage({
                directive: event.effectiveDirective,
                disposition: event.disposition,
                blocked: event.blockedURI
              });
            });
            """,
          injectionTime: .atDocumentStart, forMainFrameOnly: true,
          in: .world(name: "ChekReader")))
    #endif
    // Navigation policy does not cover subresources. Block network/file loads in
    // the content process as well as restricting responses with CSP.
    // Content blockers use a restricted regex dialect: alternation is unsupported.
    let rules =
      #"[{"trigger":{"url-filter":"^https?://"},"action":{"type":"block"}},{"trigger":{"url-filter":"^wss?://"},"action":{"type":"block"}},{"trigger":{"url-filter":"^ftp://"},"action":{"type":"block"}},{"trigger":{"url-filter":"^file://"},"action":{"type":"block"}}]"#
    let list = try await WKContentRuleListStore.default().compileContentRuleList(
      forIdentifier: "chek-offline-publications-v1", encodedContentRuleList: rules)
    guard let list else {
      throw ReaderError.invalid("The offline content policy could not be prepared.")
    }
    controller.add(list)
    var configuration = WebPage.Configuration()
    configuration.websiteDataStore = .nonPersistent()
    configuration.userContentController = controller
    configuration.defaultNavigationPreferences.allowsContentJavaScript = false
    configuration.allowsAirPlayForMediaPlayback = false
    configuration.dataDetectorTypes = []
    guard let scheme = URLScheme("chek-publication") else {
      throw ReaderError.invalid("The publication resource scheme could not be prepared.")
    }
    configuration.urlSchemeHandlers[scheme] = PublicationSchemeHandler(
      publication: publication,
      onFailure: { [weak self] message in
        guard let self else { return }
        if self.explicitLoad {
          self.resourceFailure = message
        } else if self.message == nil {
          self.message = message
        }
      })
    let page = WebPage(
      configuration: configuration,
      navigationDecider: PublicationNavigation(
        host: publication.host,
        onNavigation: { [weak self] url in
          guard let self,
            let canonical = try? self.publication.url(for: EPUBPublication.linkHref(url))
          else { return }
          self.currentHref = EPUBPublication.linkHref(canonical)
        },
        onLink: { [weak self] url in
          guard let self,
            let canonical = try? self.publication.url(for: EPUBPublication.linkHref(url))
          else { return }
          self.requestedHref = EPUBPublication.linkHref(canonical)
          self.loadRequestID = UUID()
        },
        onBlocked: { [weak self] in
          self?.message = String(
            localized: "External links are disabled while reading local books.", bundle: .module)
        }))
    #if DEBUG
      page.isInspectable = true
    #endif
    return page
  }
}

public struct EPUBReaderView: View {
  private let reader: EPUBReader
  private let onTap: @MainActor () -> Void
  private let onInteraction: @MainActor () -> Void

  public init(
    reader: EPUBReader,
    onTap: @escaping @MainActor () -> Void = {},
    onInteraction: @escaping @MainActor () -> Void = {}
  ) {
    self.reader = reader
    self.onTap = onTap
    self.onInteraction = onInteraction
  }

  public var body: some View {
    Group {
      if reader.loadFailed {
        ReaderFailureView(message: reader.failureDescription, retry: reader.retry)
      } else if let page = reader.page {
        ReaderPageView(reader: reader, page: page, viewport: reader.fixedViewport)
      } else {
        ProgressView(String(localized: "Opening book…", bundle: .module))
      }
    }
    .background {
      ReaderCallbacks(reader: reader, onTap: onTap, onInteraction: onInteraction)
        .frame(width: 0, height: 0)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
    .task(id: reader.loadRequestID) { [weak reader] in await reader?.load() }
    .id(ObjectIdentifier(reader))
  }
}

struct ReaderFailureView: View {
  let message: String?
  let retry: @MainActor () -> Void

  var body: some View {
    GeometryReader { geometry in
      ScrollView {
        ContentUnavailableView {
          Label(
            String(localized: "Page could not load", bundle: .module),
            systemImage: "exclamationmark.triangle")
        } description: {
          Text(
            message
              ?? String(localized: "Close this book and reopen it to try again.", bundle: .module))
        } actions: {
          Button(String(localized: "Try again", bundle: .module), action: retry)
            .buttonStyle(.glass)
        }
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, minHeight: max(0, geometry.size.height))
      }
      .scrollBounceBehavior(.basedOnSize)
    }
    .background(.background)
  }
}

// A persistent host can replace callback captures without replacing its reader.
// Representable updates receive those closures without mutating state in body.
private struct ReaderCallbacks: UIViewRepresentable {
  let reader: EPUBReader
  let onTap: @MainActor () -> Void
  let onInteraction: @MainActor () -> Void

  func makeCoordinator() -> Coordinator { Coordinator() }

  func makeUIView(context: Context) -> CallbackView {
    let view = CallbackView(coordinator: context.coordinator)
    update(view)
    return view
  }

  func updateUIView(_ view: CallbackView, context: Context) { update(view) }

  static func dismantleUIView(_ view: CallbackView, coordinator: Coordinator) {
    coordinator.detach()
  }

  private func update(_ view: CallbackView) {
    let coordinator = view.coordinator
    if coordinator.reader !== reader {
      coordinator.detach()
      coordinator.reader = reader
    }
    coordinator.onTap = onTap
    coordinator.onInteraction = onInteraction
    if view.window != nil { coordinator.attach() }
  }

  @MainActor final class Coordinator {
    let id = UUID()
    weak var reader: EPUBReader?
    var onTap: @MainActor () -> Void = {}
    var onInteraction: @MainActor () -> Void = {}

    func attach() {
      guard let reader else { return }
      reader.callbackOwner = id
      reader.onTap = onTap
      reader.onInteraction = onInteraction
    }

    func detach() {
      guard let reader, reader.callbackOwner == id else { return }
      reader.callbackOwner = nil
      reader.onTap = {}
      reader.onInteraction = {}
    }
  }

  @MainActor final class CallbackView: UIView {
    let coordinator: Coordinator

    init(coordinator: Coordinator) {
      self.coordinator = coordinator
      super.init(frame: .zero)
    }

    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
      super.didMoveToWindow()
      if window == nil {
        coordinator.detach()
      } else {
        coordinator.attach()
      }
    }
  }
}

private struct ReaderWebContent: View {
  let reader: EPUBReader
  let page: WebPage

  var body: some View {
    WebView(page)
      .webViewBackForwardNavigationGestures(.disabled)
      .webViewLinkPreviews(.disabled)
      .accessibilityElement(children: .contain)
      .accessibilityActions {
        if reader.allowsChapterSwipe, reader.isReadyForInteraction, let index = reader.chapterIndex
        {
          if index > 0 {
            Button {
              [weak reader] in
              guard let reader, reader.allowsChapterSwipe, reader.isReadyForInteraction else {
                return
              }
              reader.onInteraction()
              reader.moveChapter(by: -1)
            } label: {
              Text("Previous chapter", bundle: .module)
            }
          }
          if index + 1 < reader.publication.readingOrder.count {
            Button {
              [weak reader] in
              guard let reader, reader.allowsChapterSwipe, reader.isReadyForInteraction else {
                return
              }
              reader.onInteraction()
              reader.moveChapter(by: 1)
            } label: {
              Text("Next chapter", bundle: .module)
            }
          }
        }
      }
      .webViewOnScrollGeometryChange(
        for: ReaderInsets.self,
        of: {
          ReaderInsets(top: $0.contentInsets.top, bottom: $0.contentInsets.bottom)
        },
        action: { [weak reader] _, insets in
          Task { [weak reader] in
            await reader?.setInsets(top: insets.top, bottom: insets.bottom)
          }
        }
      )
      // A navigation error ends this sequence, including errors handled by the
      // explicit load task. Subscribe again when that load becomes ready so
      // later content-process failures remain observable.
      .task(
        id: ReaderNavigationObservationID(
          page: ObjectIdentifier(page), isReady: reader.isReadyForInteraction)
      ) { [weak page, weak reader] in
        guard let page else { return }
        do {
          for try await _ in page.navigations { try Task.checkCancellation() }
        } catch is CancellationError {
        } catch {
          if !Task.isCancelled, reader?.page === page { reader?.navigationFailed(error) }
        }
      }
  }
}

private struct ReaderPageView: View {
  let reader: EPUBReader
  let page: WebPage
  let viewport: FixedPageViewport?

  var body: some View {
    GeometryReader { geometry in
      // NavigationStack already proposes its available reader area. Subtracting
      // the reported safe areas again shrinks landscape pages twice.
      let width = max(1, geometry.size.width)
      let height = max(1, geometry.size.height)
      let contentWidth = viewport?.width ?? width
      let contentHeight = viewport?.height ?? height
      let factor = min(width / contentWidth, height / contentHeight)
      // A WebPage may attach to only one WebView. Keep that view's identity
      // stable when a mixed-layout spine switches between reflow and fixed.
      ReaderWebContent(reader: reader, page: page)
        .frame(width: contentWidth, height: contentHeight)
        .scaleEffect(factor)
        .frame(width: width, height: height)
        .clipped()
    }
  }
}

private struct PublicationNavigation: WebPage.NavigationDeciding {
  let host: String
  let onNavigation: @MainActor (URL) -> Void
  let onLink: @MainActor (URL) -> Void
  let onBlocked: @MainActor () -> Void

  mutating func decidePolicy(
    for action: WebPage.NavigationAction,
    preferences: inout WebPage.NavigationPreferences
  ) async -> WKNavigationActionPolicy {
    preferences.allowsContentJavaScript = false
    guard let url = action.request.url, url.scheme == "chek-publication", url.host == host,
      action.target?.isMainFrame == true, !action.shouldPerformDownload,
      action.navigationType != .formSubmitted, action.navigationType != .formResubmitted
    else {
      onBlocked()
      return .cancel
    }
    if action.navigationType == .linkActivated {
      onLink(url)
      return .cancel
    }
    onNavigation(url)
    return .allow
  }

  func decidePolicy(for response: WebPage.NavigationResponse) async -> WKNavigationResponsePolicy {
    guard response.response.url?.scheme == "chek-publication", response.response.url?.host == host,
      response.canShowMimeType
    else { return .cancel }
    return .allow
  }
}

private struct ReaderInsets: Hashable {
  let top: CGFloat
  let bottom: CGFloat
}

private struct ReaderNavigationObservationID: Hashable {
  let page: ObjectIdentifier
  let isReady: Bool
}

@MainActor private final class ReaderBridge: NSObject, WKScriptMessageHandler {
  weak var reader: EPUBReader?

  init(reader: EPUBReader) { self.reader = reader }

  func userContentController(
    _ userContentController: WKUserContentController, didReceive message: WKScriptMessage
  ) {
    reader?.receive(message)
  }
}

private struct PublicationSchemeHandler: URLSchemeHandler {
  let publication: EPUBPublication
  let onFailure: @MainActor @Sendable (String) -> Void

  func reply(for request: URLRequest) -> AsyncThrowingStream<URLSchemeTaskResult, any Error> {
    AsyncThrowingStream { continuation in
      let task = Task {
        do {
          guard let url = request.url, url.scheme == "chek-publication",
            url.host == publication.host,
            request.httpMethod == nil || request.httpMethod == "GET"
          else { throw ReaderError.invalid("A publication request was rejected.") }
          let path = try ArchiveStore.canonicalPath(String(url.path.dropFirst()))
          guard let resource = publication.resources[path] else {
            throw ReaderError.invalid("This resource is not declared in the book's manifest.")
          }
          let data = try await publication.data(for: resource)
          try Task.checkCancellation()
          try ArchiveStore.checkImage(data, mediaType: resource.mediaType)
          let csp =
            "default-src 'none'; script-src 'none'; connect-src 'none'; style-src 'unsafe-inline' chek-publication:; img-src chek-publication:; font-src chek-publication:; media-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
          guard
            let response = HTTPURLResponse(
              url: url, statusCode: 200, httpVersion: nil,
              headerFields: [
                "Content-Type": resource.mediaType, "Content-Length": String(data.count),
                "Content-Security-Policy": csp, "X-Content-Type-Options": "nosniff",
                "Access-Control-Allow-Origin": "*",
              ])
          else { throw ReaderError.invalid("A publication response could not be formed.") }
          try Task.checkCancellation()
          continuation.yield(.response(response))
          continuation.yield(.data(data))
          continuation.finish()
        } catch {
          if !Task.isCancelled, !(error is CancellationError) {
            await onFailure(error.localizedDescription)
          }
          continuation.finish(throwing: error)
        }
      }
      continuation.onTermination = { _ in task.cancel() }
    }
  }
}
