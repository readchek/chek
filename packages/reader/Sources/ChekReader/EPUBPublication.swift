import Foundation

public enum EPUBPageProgressionDirection: String, Sendable {
  case leftToRight = "ltr"
  case rightToLeft = "rtl"
  case `default` = "default"
}

public struct EPUBLink: Identifiable, Hashable, Sendable {
  public var id: String { href }
  public let title: String
  public let href: String
  public let depth: Int
}

struct EPUBResource: Sendable {
  let path: String
  let mediaType: String
}

public struct EPUBPublication: Sendable {
  public let title: String
  public let author: String
  public let readingOrder: [EPUBLink]
  public let tableOfContents: [EPUBLink]
  public let landmarks: [EPUBLink]
  public let pageList: [EPUBLink]
  /// The spine's declared progression, or default when unspecified.
  /// The reader uses left-to-right chapter gestures for default progression.
  public let pageProgressionDirection: EPUBPageProgressionDirection
  let archive: ArchiveStore
  let resources: [String: EPUBResource]
  let host: String
  private let fixedResources: Set<String>
  private let obfuscatedFonts: Set<String>
  private let fontKey: [UInt8]

  /// Opens and validates an EPUB without extracting it into a directory.
  @concurrent public static func open(_ url: URL) async throws -> EPUBPublication {
    let archive = try ArchiveStore(url: url)
    guard
      try await archive.data(at: "mimetype", maximumBytes: 100) == Data("application/epub+zip".utf8)
    else {
      throw ReaderError.invalid("The EPUB mimetype is missing or incorrect.")
    }
    let container = try PublicationXML.parse(
      await archive.data(at: "META-INF/container.xml", maximumBytes: 4 * 1024 * 1024))
    guard
      let packagePath = container.descendants(
        "rootfile", namespace: "urn:oasis:names:tc:opendocument:xmlns:container"
      )
      .first(where: { $0.attributes["media-type"] == "application/oebps-package+xml" })?.attributes[
        "full-path"]
    else { throw ReaderError.invalid("The EPUB container has no package document.") }
    let path = try ArchiveStore.canonicalPath(packagePath)
    let package = try PublicationXML.parse(
      await archive.data(at: path, maximumBytes: 4 * 1024 * 1024))
    let opf = "http://www.idpf.org/2007/opf"
    guard package.name == "package", package.namespace == opf else {
      throw ReaderError.invalid("The package document has an unsupported namespace.")
    }
    let metadata = package.children.first(where: { $0.name == "metadata" && $0.namespace == opf })
    let legacyFixed =
      metadata?.children.contains(where: {
        $0.name == "meta" && $0.attributes["name"] == "fixed-layout"
          && $0.attributes["content"] == "true"
      }) == true
    let globalLayout =
      metadata?.children.first(where: {
        $0.name == "meta" && $0.attributes["property"] == "rendition:layout"
          && $0.attributes["refines"] == nil
      })?.value ?? (legacyFixed ? "pre-paginated" : "reflowable")
    guard ["reflowable", "pre-paginated"].contains(globalLayout) else {
      throw ReaderError.unsupported(
        "This EPUB uses an unsupported rendition layout. Continuous roll layout is not supported yet."
      )
    }
    let host = UUID().uuidString.lowercased()
    let packageURL = try resourceURL(path: path, host: host)
    var byID: [String: EPUBResource] = [:]
    var resources: [String: EPUBResource] = [:]
    var navigation: EPUBResource?
    for item in package.descendants("item", namespace: opf) {
      guard let id = item.attributes["id"], let href = item.attributes["href"],
        let mediaType = item.attributes["media-type"], byID[id] == nil
      else {
        throw ReaderError.invalid("A manifest item is missing data or repeats an identifier.")
      }
      let resolved = try resolve(href, relativeTo: packageURL, host: host)
      guard resources[resolved.path] == nil else {
        throw ReaderError.invalid("The manifest declares the same resource more than once.")
      }
      guard await archive.contains(resolved.path) else {
        throw ReaderError.invalid("A manifest resource is missing: \(resolved.path)")
      }
      let resource = EPUBResource(path: resolved.path, mediaType: mediaType)
      byID[id] = resource
      resources[resolved.path] = resource
      if item.attributes["properties"]?.split(whereSeparator: \.isWhitespace).contains("nav")
        == true
      {
        navigation = resource
      }
    }
    let spine = package.children.first(where: { $0.name == "spine" && $0.namespace == opf })
    guard
      let pageProgressionDirection = EPUBPageProgressionDirection(
        rawValue: spine?.attributes["page-progression-direction"] ?? "default")
    else {
      throw ReaderError.invalid("The EPUB spine declares an invalid page progression direction.")
    }
    var order: [EPUBLink] = []
    var fixedResources: Set<String> = []
    for item in package.descendants("itemref", namespace: opf) {
      guard let id = item.attributes["idref"], let resource = byID[id] else {
        throw ReaderError.invalid("The reading order refers to a missing manifest item.")
      }
      let overrides = (item.attributes["properties"] ?? "").split(whereSeparator: \.isWhitespace)
        .filter { $0.hasPrefix("rendition:layout-") }
      guard overrides.count <= 1,
        overrides.allSatisfy({
          ["rendition:layout-reflowable", "rendition:layout-pre-paginated"].contains($0)
        })
      else {
        throw ReaderError.unsupported(
          "This EPUB has conflicting or unsupported spine layout overrides.")
      }
      let fixed =
        overrides.first.map { $0 == "rendition:layout-pre-paginated" }
        ?? (globalLayout == "pre-paginated")
      if fixed || resource.mediaType == "image/svg+xml" { fixedResources.insert(resource.path) }
      guard ["application/xhtml+xml", "text/html", "image/svg+xml"].contains(resource.mediaType)
      else {
        throw ReaderError.unsupported(
          "This EPUB uses an unsupported spine document type: \(resource.mediaType)")
      }
      // Non-linear items remain loadable through navigation without forcing them into next/previous.
      if item.attributes["linear"] != "no" {
        let url = try resourceURL(path: resource.path, host: host)
        order.append(EPUBLink(title: "", href: linkHref(url), depth: 0))
      }
    }
    guard !order.isEmpty else {
      throw ReaderError.invalid("The EPUB has no readable spine documents.")
    }
    var toc: [EPUBLink] = []
    var landmarks: [EPUBLink] = []
    var pageList: [EPUBLink] = []
    if let navigation {
      let root = try PublicationXML.parse(
        await archive.data(at: navigation.path, maximumBytes: 4 * 1024 * 1024))
      let navURL = try resourceURL(path: navigation.path, host: host)
      for node in root.descendants("nav", namespace: "http://www.w3.org/1999/xhtml") {
        let types = (node.attribute("type") ?? "").split(whereSeparator: \.isWhitespace)
        if types.contains("toc"), toc.isEmpty {
          toc = try navigationLinks(node, base: navURL, host: host)
        }
        if types.contains("landmarks"), landmarks.isEmpty {
          landmarks = try navigationLinks(node, base: navURL, host: host)
        }
        if types.contains("page-list"), pageList.isEmpty {
          pageList = try navigationLinks(node, base: navURL, host: host)
        }
      }
    } else if let ncxID = package.descendants("spine", namespace: opf).first?.attributes["toc"],
      let ncx = byID[ncxID]
    {
      let root = try PublicationXML.parse(
        await archive.data(at: ncx.path, maximumBytes: 4 * 1024 * 1024))
      let base = try resourceURL(path: ncx.path, host: host)
      toc = try ncxLinks(root, base: base, host: host)
      pageList = try ncxLinks(root, base: base, host: host, element: "pageTarget")
    }
    if landmarks.isEmpty,
      let guide = package.children.first(where: { $0.name == "guide" && $0.namespace == opf })
    {
      for reference in guide.children where reference.name == "reference" {
        if let href = reference.attributes["href"] {
          let target = try resolve(href, relativeTo: packageURL, host: host)
          landmarks.append(
            EPUBLink(
              title: reference.attributes["title"] ?? reference.attributes["type"] ?? "",
              href: linkHref(target.url), depth: 0))
        }
      }
    }
    func declaredLinks(_ links: [EPUBLink]) throws -> [EPUBLink] {
      for link in links {
        let target = try resolve("/" + link.href, relativeTo: packageURL, host: host)
        guard resources[target.path] != nil else {
          throw ReaderError.invalid("A navigation link refers to a resource outside the manifest.")
        }
      }
      return links
    }
    toc = try declaredLinks(toc)
    landmarks = try declaredLinks(landmarks)
    pageList = try declaredLinks(pageList)
    var obfuscatedFonts: Set<String> = []
    var fontKey: [UInt8] = []
    if await archive.contains("META-INF/encryption.xml") {
      let obfuscation = try FontObfuscation.parse(
        await archive.data(at: "META-INF/encryption.xml", maximumBytes: 4 * 1024 * 1024),
        package: package, resources: resources, host: host)
      obfuscatedFonts = obfuscation.paths
      fontKey = obfuscation.key
    }
    var labels: [String: String] = [:]
    for link in toc {
      let path = link.href.split(separator: "#", maxSplits: 1).first.map(String.init) ?? ""
      if labels[path] == nil { labels[path] = link.title }
    }
    order = order.map { link in
      EPUBLink(
        title: labels[link.href] ?? "",
        href: link.href, depth: 0)
    }
    let dc = "http://purl.org/dc/elements/1.1/"
    let title = metadata?.descendants("title", namespace: dc).first?.value ?? ""
    let author =
      metadata?.descendants("creator", namespace: dc).map(\.value).joined(separator: ", ") ?? ""
    return EPUBPublication(
      title: title.isEmpty ? url.deletingPathExtension().lastPathComponent : title,
      author: author, readingOrder: order, tableOfContents: toc, landmarks: landmarks,
      pageList: pageList, pageProgressionDirection: pageProgressionDirection,
      archive: archive, resources: resources, host: host,
      fixedResources: fixedResources, obfuscatedFonts: obfuscatedFonts, fontKey: fontKey)
  }

  func isFixedLayout(_ href: String) -> Bool {
    guard let url = try? url(for: href) else { return false }
    return fixedResources.contains(String(url.path.dropFirst()))
      || resources[String(url.path.dropFirst())]?.mediaType == "image/svg+xml"
  }

  @concurrent func fixedViewport(for href: String) async throws -> FixedPageViewport? {
    guard isFixedLayout(href) else { return nil }
    let url = try url(for: href)
    let path = String(url.path.dropFirst())
    guard let resource = resources[path] else {
      throw ReaderError.invalid("A fixed-layout resource is missing.")
    }
    return try FixedPageViewport.parse(
      await archive.data(at: path, maximumBytes: 4 * 1024 * 1024), mediaType: resource.mediaType)
  }

  func data(for resource: EPUBResource) async throws -> Data {
    let data = try await archive.data(at: resource.path)
    return obfuscatedFonts.contains(resource.path)
      ? try FontObfuscation.decode(data, key: fontKey) : data
  }

  func url(for href: String) throws -> URL {
    let base = try Self.resourceURL(path: "", host: host)
    let resolved = try Self.resolve(
      href.hasPrefix("/") ? href : "/" + href, relativeTo: base, host: host)
    guard resources[resolved.path] != nil else {
      throw ReaderError.invalid("The linked resource is not in the manifest.")
    }
    return resolved.url
  }

  static func resourceURL(path: String, host: String) throws -> URL {
    var parts = URLComponents()
    parts.scheme = "chek-publication"
    parts.host = host
    parts.path = "/" + path
    guard let url = parts.url else {
      throw ReaderError.invalid("A resource URL could not be formed.")
    }
    return url
  }

  static func linkHref(_ url: URL) -> String {
    guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: true) else { return "" }
    return String(parts.percentEncodedPath.dropFirst())
      + (parts.percentEncodedFragment.map { "#" + $0 } ?? "")
  }

  private static func resolve(_ href: String, relativeTo base: URL, host: String) throws -> (
    path: String, url: URL
  ) {
    guard let url = URL(string: href, relativeTo: base)?.absoluteURL,
      url.scheme == "chek-publication", url.host == host, url.user == nil, url.password == nil,
      url.port == nil
    else {
      throw ReaderError.unsupported("Remote or external publication resources are not supported.")
    }
    let path = try ArchiveStore.canonicalPath(String(url.path.dropFirst()))
    var canonical = URLComponents(
      url: try resourceURL(path: path, host: host), resolvingAgainstBaseURL: true)
    canonical?.percentEncodedFragment =
      URLComponents(url: url, resolvingAgainstBaseURL: true)?.percentEncodedFragment
    guard let canonicalURL = canonical?.url else {
      throw ReaderError.invalid("A resource URL could not be normalized.")
    }
    return (path, canonicalURL)
  }

  private static func navigationLinks(_ node: XMLNode, base: URL, host: String, depth: Int = 0)
    throws -> [EPUBLink]
  {
    var links: [EPUBLink] = []
    for child in node.children {
      if child.name == "a", let href = child.attributes["href"] {
        let target = try resolve(href, relativeTo: base, host: host)
        links.append(
          EPUBLink(
            title: child.value, href: linkHref(target.url),
            depth: max(depth - 1, 0)))
      }
      links += try navigationLinks(
        child, base: base, host: host, depth: depth + (child.name == "ol" ? 1 : 0))
    }
    return links
  }

  private static func ncxLinks(
    _ node: XMLNode, base: URL, host: String, depth: Int = 0, element: String = "navPoint"
  ) throws
    -> [EPUBLink]
  {
    var links: [EPUBLink] = []
    for child in node.children {
      if child.name == element,
        let src = child.children.first(where: { $0.name == "content" })?.attributes["src"]
      {
        let target = try resolve(src, relativeTo: base, host: host)
        let title = child.children.first(where: { $0.name == "navLabel" })?.value ?? ""
        links.append(
          EPUBLink(
            title: title, href: linkHref(target.url),
            depth: depth))
      }
      links += try ncxLinks(
        child, base: base, host: host, depth: depth + (child.name == element ? 1 : 0),
        element: element)
    }
    return links
  }
}
