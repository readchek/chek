import Foundation
import Observation
import SwiftUI
import WebKit

@MainActor @Observable
public final class EPUBReader {
  public let publication: EPUBPublication
  public private(set) var currentHref: String
  public private(set) var requestedHref: String
  public var message: String?
  private(set) var page: WebPage?
  private(set) var loadFailed = false

  public var hasMessage: Bool {
    get { message != nil }
    set { if !newValue { message = nil } }
  }

  public init(publication: EPUBPublication) {
    self.publication = publication
    currentHref = publication.readingOrder[0].href
    requestedHref = publication.readingOrder[0].href
  }

  public var chapterIndex: Int? {
    let path = currentHref.split(separator: "#", maxSplits: 1).first.map(String.init)
    return publication.readingOrder.firstIndex(where: { $0.href == path })
  }

  public func open(_ link: EPUBLink) {
    currentHref = link.href
    requestedHref = link.href
  }

  public func moveChapter(by delta: Int) {
    guard let index = chapterIndex, publication.readingOrder.indices.contains(index + delta) else {
      return
    }
    open(publication.readingOrder[index + delta])
  }

  func load() async {
    loadFailed = false
    do {
      if page == nil { page = try await makePage() }
      try Task.checkCancellation()
      guard let page else { return }
      let url = try publication.url(for: requestedHref)
      for try await _ in page.load(url) { try Task.checkCancellation() }
    } catch is CancellationError {
    } catch {
      if !Task.isCancelled {
        loadFailed = true
        message = error.localizedDescription
      }
    }
  }

  func observeNavigations() async {
    guard let page else { return }
    do {
      for try await _ in page.navigations { try Task.checkCancellation() }
    } catch is CancellationError {
    } catch {
      if !Task.isCancelled {
        loadFailed = true
        message = error.localizedDescription
      }
    }
  }

  private func makePage() async throws -> WebPage {
    let controller = WKUserContentController()
    // Supply a device-width viewport for reflowable books that omit one. Leave
    // authored styles and existing viewport declarations intact.
    let viewport = #"""
      (() => {
        const install = () => {
          if (!document.head) return false;
          if (!document.querySelector('meta[name="viewport"]')) {
            const meta = document.createElementNS('http://www.w3.org/1999/xhtml', 'meta');
            meta.setAttribute('name', 'viewport');
            meta.setAttribute('content', 'width=device-width, initial-scale=1');
            document.head.appendChild(meta);
          }
          if (!document.querySelector('style[data-chek-reader-insets]')) {
            const style = document.createElementNS('http://www.w3.org/1999/xhtml', 'style');
            style.setAttribute('data-chek-reader-insets', '');
            style.textContent = 'html { padding-inline: 1rem; box-sizing: border-box; }';
            document.head.appendChild(style);
          }
          return true;
        };
        if (!install()) {
          const observer = new MutationObserver(() => { if (install()) observer.disconnect(); });
          observer.observe(document, {childList: true, subtree: true});
          document.addEventListener('DOMContentLoaded', () => { observer.disconnect(); install(); }, {once: true});
        }
      })();
      """#
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
      onFailure: { [weak self] message in if self?.message == nil { self?.message = message } })
    return WebPage(
      configuration: configuration,
      navigationDecider: PublicationNavigation(
        host: publication.host,
        onNavigation: { [weak self] url in
          self?.currentHref = EPUBPublication.linkHref(url)
        },
        onBlocked: { [weak self] in
          self?.message = String(
            localized: "External links are disabled while reading local books.", bundle: .module)
        }))
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
          Text(String(localized: "Close this book and reopen it to try again.", bundle: .module))
        }
      } else if let page = reader.page {
        WebView(page)
          .webViewBackForwardNavigationGestures(.disabled)
          .webViewLinkPreviews(.disabled)
          .task { await reader.observeNavigations() }
      } else {
        ProgressView(String(localized: "Opening book…", bundle: .module))
      }
    }
    .task(id: reader.requestedHref) { await reader.load() }
  }
}

private struct PublicationNavigation: WebPage.NavigationDeciding {
  let host: String
  let onNavigation: @MainActor (URL) -> Void
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
          let data = try await publication.archive.data(at: path)
          if resource.mediaType.hasPrefix("image/"), resource.mediaType != "image/svg+xml" {
            try ArchiveStore.checkImage(data)
          }
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
