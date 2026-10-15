import Foundation
import Observation
import SwiftUI
import WebKit

@MainActor @Observable
public final class EPUBReader {
  public let publication: EPUBPublication
  public private(set) var currentHref: String
  public private(set) var requestedHref: String
  public private(set) var location: ReadingLocation?
  public private(set) var textScale: Double
  public var message: String?
  private(set) var page: WebPage?
  private(set) var loadFailed = false
  private(set) var failureDescription: String?
  private(set) var fixedViewport: FixedPageViewport?
  private(set) var loadRequestID = UUID()
  private var resumeLocation: ReadingLocation?
  private var documentToken = ""
  private var isRestoring = true
  private var explicitLoad = false
  private var resourceFailure: String?
  private var topInset: CGFloat = 0
  private var bottomInset: CGFloat = 0

  public var hasMessage: Bool {
    get { message != nil }
    set { if !newValue { message = nil } }
  }

  public init(publication: EPUBPublication, location: ReadingLocation? = nil, textScale: Double = 1)
  {
    self.publication = publication
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

  public func open(_ link: EPUBLink) {
    requestedHref = link.href
    loadRequestID = UUID()
  }

  public func moveChapter(by delta: Int) {
    guard let index = chapterIndex, publication.readingOrder.indices.contains(index + delta) else {
      return
    }
    open(publication.readingOrder[index + delta])
  }

  func load() async {
    let generation = loadRequestID
    loadFailed = false
    failureDescription = nil
    resourceFailure = nil
    do {
      if page != nil { _ = await captureLocation() }
      let viewport = try await publication.fixedViewport(for: requestedHref)
      guard generation == loadRequestID else { return }
      fixedViewport = viewport
      if page == nil { page = try await makePage() }
      try Task.checkCancellation()
      guard generation == loadRequestID, let page else { return }
      let url = try publication.url(for: requestedHref)
      let anchor = resumeLocation
      resumeLocation = nil
      documentToken = UUID().uuidString
      isRestoring = true
      explicitLoad = true
      for try await _ in page.load(url) { try Task.checkCancellation() }
      guard generation == loadRequestID else { return }
      let value = try await page.callJavaScript(
        "return await window.chekReader.activate(token, anchor, scale, top, bottom, fixed);",
        arguments: [
          "token": documentToken, "anchor": try jsonValue(anchor), "scale": textScale,
          "top": Double(topInset), "bottom": Double(bottomInset), "fixed": viewport != nil,
        ], contentWorld: .world(name: "ChekReader"))
      guard generation == loadRequestID else { return }
      isRestoring = false
      explicitLoad = false
      acceptLocation(value)
      if let resourceFailure, message == nil { message = resourceFailure }
    } catch is CancellationError {
    } catch {
      if !Task.isCancelled {
        explicitLoad = false
        loadFailed = true
        failureDescription = resourceFailure ?? error.localizedDescription
      }
    }
  }

  func observeNavigations() async {
    guard let page else { return }
    do {
      for try await _ in page.navigations { try Task.checkCancellation() }
    } catch is CancellationError {
    } catch {
      if !Task.isCancelled && !explicitLoad {
        loadFailed = true
        failureDescription = error.localizedDescription
      }
    }
  }

  /// Capture synchronously from the current DOM before close/background saves.
  @discardableResult public func captureLocation() async -> ReadingLocation? {
    guard !isRestoring, let page else { return location }
    let token = documentToken
    do {
      let value = try await page.callJavaScript(
        "return window.chekReader.capture();",
        contentWorld: .world(name: "ChekReader"))
      if token == documentToken { acceptLocation(value) }
    } catch { if !Task.isCancelled { message = error.localizedDescription } }
    return location
  }

  public func setTextScale(_ scale: Double) async {
    guard scale.isFinite, !isRestoring, !isFixedLayout, let page else { return }
    _ = await captureLocation()
    let token = documentToken
    textScale = min(2, max(0.75, scale))
    isRestoring = true
    do {
      let value = try await page.callJavaScript(
        "return await window.chekReader.setScale(scale);",
        arguments: ["scale": textScale], contentWorld: .world(name: "ChekReader"))
      guard token == documentToken else { return }
      isRestoring = false
      acceptLocation(value)
    } catch {
      if token == documentToken { isRestoring = false }
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
    guard !isRestoring, message.frameInfo.isMainFrame,
      message.frameInfo.request.url?.host == publication.host,
      let body = message.body as? [String: Any], body["token"] as? String == documentToken
    else { return }
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
    controller.addUserScript(
      WKUserScript(
        source: viewport, injectionTime: .atDocumentStart,
        forMainFrameOnly: true, in: .world(name: "ChekReader")))
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

  public init(reader: EPUBReader) { self.reader = reader }

  public var body: some View {
    Group {
      if reader.loadFailed {
        ContentUnavailableView {
          Label(
            String(localized: "Page could not load", bundle: .module),
            systemImage: "exclamationmark.triangle")
        } description: {
          Text(
            reader.failureDescription
              ?? String(localized: "Close this book and reopen it to try again.", bundle: .module))
        }
      } else if let page = reader.page {
        if let viewport = reader.fixedViewport {
          FixedPageView(reader: reader, page: page, viewport: viewport)
        } else {
          ReaderWebContent(reader: reader, page: page)
        }
      } else {
        ProgressView(String(localized: "Opening book…", bundle: .module))
      }
    }
    .task(id: reader.loadRequestID) { await reader.load() }
  }
}

private struct ReaderWebContent: View {
  let reader: EPUBReader
  let page: WebPage

  var body: some View {
    WebView(page)
      .webViewBackForwardNavigationGestures(.disabled)
      .webViewLinkPreviews(.disabled)
      .webViewOnScrollGeometryChange(
        for: ReaderInsets.self,
        of: {
          ReaderInsets(top: $0.contentInsets.top, bottom: $0.contentInsets.bottom)
        },
        action: { _, insets in
          Task { await reader.setInsets(top: insets.top, bottom: insets.bottom) }
        }
      )
      .task { await reader.observeNavigations() }
  }
}

private struct FixedPageView: View {
  let reader: EPUBReader
  let page: WebPage
  let viewport: FixedPageViewport

  var body: some View {
    GeometryReader { geometry in
      // NavigationStack already proposes its available reader area. Subtracting
      // the reported safe areas again shrinks landscape pages twice.
      let width = max(1, geometry.size.width)
      let height = max(1, geometry.size.height)
      let factor = min(width / viewport.width, height / viewport.height)
      ReaderWebContent(reader: reader, page: page)
        .frame(width: viewport.width, height: viewport.height)
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
          if !(error is CancellationError) { await onFailure(error.localizedDescription) }
          continuation.finish(throwing: error)
        }
      }
      continuation.onTermination = { _ in task.cancel() }
    }
  }
}
