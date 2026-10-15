import Foundation

final class XMLNode {
  let name: String
  let namespace: String
  let attributes: [String: String]
  var children: [XMLNode] = []
  private enum Content {
    case text(String)
    case child(XMLNode)
  }
  private var content: [Content] = []

  init(name: String, namespace: String, attributes: [String: String]) {
    self.name = name
    self.namespace = namespace
    self.attributes = attributes
  }

  var value: String {
    var result = ""
    appendValue(to: &result)
    return result.trimmingCharacters(in: .whitespacesAndNewlines)
  }

  func appendText(_ text: String) { content.append(.text(text)) }

  func appendChild(_ child: XMLNode) {
    children.append(child)
    content.append(.child(child))
  }

  private func appendValue(to result: inout String) {
    for piece in content {
      switch piece {
      case .text(let text): result.append(text)
      case .child(let child): child.appendValue(to: &result)
      }
    }
  }

  func descendants(_ name: String, namespace: String? = nil) -> [XMLNode] {
    children.flatMap { child in
      let matches = child.name == name && (namespace == nil || child.namespace == namespace)
      return (matches ? [child] : []) + child.descendants(name, namespace: namespace)
    }
  }

  func attribute(_ name: String) -> String? {
    attributes[name] ?? attributes.first(where: { $0.key.hasSuffix(":" + name) })?.value
  }
}

final class PublicationXML: NSObject, XMLParserDelegate {
  private var stack: [XMLNode] = []
  private var root: XMLNode?
  private var count = 0
  private var textBytes = 0
  private var rejected = false

  static func parse(_ data: Data) throws -> XMLNode {
    guard data.count <= 4 * 1024 * 1024 else { throw ReaderError.limit("XML exceeds 4 MB.") }
    let delegate = PublicationXML()
    let parser = XMLParser(data: data)
    parser.shouldProcessNamespaces = true
    parser.shouldResolveExternalEntities = false
    parser.externalEntityResolvingPolicy = .never
    parser.delegate = delegate
    guard parser.parse(), !delegate.rejected, let root = delegate.root else {
      throw ReaderError.invalid("Publication XML is malformed, too complex, or declares entities.")
    }
    return root
  }

  func parser(
    _ parser: XMLParser, didStartElement elementName: String,
    namespaceURI: String?, qualifiedName qName: String?, attributes attributeDict: [String: String]
  ) {
    count += 1
    guard count <= 100_000, stack.count < 128 else {
      reject(parser)
      return
    }
    let node = XMLNode(name: elementName, namespace: namespaceURI ?? "", attributes: attributeDict)
    if let parent = stack.last { parent.appendChild(node) } else { root = node }
    stack.append(node)
  }

  func parser(
    _ parser: XMLParser, didEndElement elementName: String,
    namespaceURI: String?, qualifiedName qName: String?
  ) { _ = stack.popLast() }

  func parser(_ parser: XMLParser, foundCharacters string: String) {
    textBytes += string.utf8.count
    guard textBytes <= 4 * 1024 * 1024 else {
      reject(parser)
      return
    }
    stack.last?.appendText(string)
  }

  func parser(_ parser: XMLParser, foundCDATA cdata: Data) {
    guard let text = String(data: cdata, encoding: .utf8) else {
      reject(parser)
      return
    }
    self.parser(parser, foundCharacters: text)
  }

  func parser(
    _ parser: XMLParser, foundInternalEntityDeclarationWithName name: String, value: String?
  ) {
    reject(parser)
  }

  func parser(
    _ parser: XMLParser, foundExternalEntityDeclarationWithName name: String,
    publicID: String?, systemID: String?
  ) { reject(parser) }

  private func reject(_ parser: XMLParser) {
    rejected = true
    parser.abortParsing()
  }
}
