import Foundation

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
  let archive: ArchiveStore
  let resources: [String: EPUBResource]
  let host: String

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
    if metadata?.descendants("meta").contains(where: {
      ($0.attributes["property"] == "rendition:layout" && $0.value != "reflowable")
        || ($0.attributes["name"] == "fixed-layout" && $0.attributes["content"] == "true")
    }) == true {
      throw ReaderError.unsupported(
        "Fixed-layout EPUB reading is not supported yet. This book was not imported.")
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
      if item.attributes["properties"]?.split(separator: " ").contains("nav") == true {
        navigation = resource
      }
    }
    var order: [EPUBLink] = []
    for item in package.descendants("itemref", namespace: opf) {
      guard let id = item.attributes["idref"], let resource = byID[id] else {
        throw ReaderError.invalid("The reading order refers to a missing manifest item.")
      }
      if item.attributes["properties"]?.split(separator: " ").contains(where: {
        $0 == "rendition:layout-pre-paginated" || $0 == "rendition:layout-roll"
      }) == true {
        throw ReaderError.unsupported(
          "This EPUB includes fixed-layout pages, which are not supported yet.")
      }
      guard ["application/xhtml+xml", "text/html"].contains(resource.mediaType) else {
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
    if let navigation {
      let root = try PublicationXML.parse(
        await archive.data(at: navigation.path, maximumBytes: 4 * 1024 * 1024))
      let navURL = try resourceURL(path: navigation.path, host: host)
      let tocNode = root.descendants("nav", namespace: "http://www.w3.org/1999/xhtml").first(
        where: {
          $0.attribute("type")?.split(separator: " ").contains("toc") == true
        })
      if let tocNode { toc = try navigationLinks(tocNode, base: navURL, host: host) }
    } else if let ncxID = package.descendants("spine", namespace: opf).first?.attributes["toc"],
      let ncx = byID[ncxID]
    {
      let root = try PublicationXML.parse(
        await archive.data(at: ncx.path, maximumBytes: 4 * 1024 * 1024))
      let base = try resourceURL(path: ncx.path, host: host)
      toc = try ncxLinks(root, base: base, host: host)
    }
    toc = toc.filter {
      guard let target = try? resolve("/" + $0.href, relativeTo: packageURL, host: host) else {
        return false
      }
      return resources[target.path] != nil
    }
    order = order.map { link in
      EPUBLink(
        title: toc.first(where: {
          $0.href.split(separator: "#", maxSplits: 1).first.map(String.init) == link.href
        })?.title ?? "",
        href: link.href, depth: 0)
    }
    let dc = "http://purl.org/dc/elements/1.1/"
    let title = metadata?.descendants("title", namespace: dc).first?.value ?? ""
    let author =
      metadata?.descendants("creator", namespace: dc).map(\.value).joined(separator: ", ") ?? ""
    return EPUBPublication(
      title: title.isEmpty ? url.deletingPathExtension().lastPathComponent : title,
      author: author, readingOrder: order, tableOfContents: toc, archive: archive,
      resources: resources, host: host)
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
    return (path, url)
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

  private static func ncxLinks(_ node: XMLNode, base: URL, host: String, depth: Int = 0) throws
    -> [EPUBLink]
  {
    var links: [EPUBLink] = []
    for child in node.children {
      if child.name == "navPoint",
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
        child, base: base, host: host, depth: depth + (child.name == "navPoint" ? 1 : 0))
    }
    return links
  }
}
