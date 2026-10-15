import CryptoKit
import Foundation

enum FontObfuscation {
  static func parse(_ data: Data, package: XMLNode, resources: [String: EPUBResource], host: String)
    throws -> (paths: Set<String>, key: [UInt8])
  {
    let root = try PublicationXML.parse(data)
    let xmlenc = "http://www.w3.org/2001/04/xmlenc#"
    guard root.name == "encryption",
      root.namespace == "urn:oasis:names:tc:opendocument:xmlns:container"
    else {
      throw ReaderError.invalid("The font obfuscation document has an invalid root element.")
    }
    guard let identifierID = package.attributes["unique-identifier"],
      let identifier = package.descendants(
        "identifier", namespace: "http://purl.org/dc/elements/1.1/"
      )
      .first(where: { $0.attributes["id"] == identifierID })?.rawValue
    else {
      throw ReaderError.invalid("Obfuscated fonts require the package's unique identifier.")
    }
    let normalized = identifier.unicodeScalars.filter { ![9, 10, 13, 32].contains($0.value) }.map(
      String.init
    ).joined()
    guard !normalized.isEmpty else {
      throw ReaderError.invalid("The font obfuscation identifier is empty.")
    }
    let key = Array(Insecure.SHA1.hash(data: Data(normalized.utf8)))
    var paths: Set<String> = []
    let base = try EPUBPublication.resourceURL(path: "", host: host)
    guard !root.children.isEmpty,
      root.children.allSatisfy({ $0.name == "EncryptedData" && $0.namespace == xmlenc })
    else {
      throw ReaderError.unsupported("This EPUB has unsupported encryption declarations.")
    }
    for item in root.descendants("EncryptedData", namespace: xmlenc) {
      let methods = item.children.filter { $0.name == "EncryptionMethod" && $0.namespace == xmlenc }
      guard methods.count == 1,
        methods[0].attributes["Algorithm"] == "http://www.idpf.org/2008/embedding"
      else {
        throw ReaderError.unsupported(
          "This EPUB uses unsupported resource encryption or font obfuscation. DRM is not supported."
        )
      }
      let references = item.descendants("CipherReference", namespace: xmlenc)
      guard references.count == 1, references[0].children.isEmpty,
        let href = references[0].attributes["URI"],
        let url = URL(string: href, relativeTo: base)?.absoluteURL,
        url.scheme == "chek-publication", url.host == host, url.fragment == nil, url.query == nil
      else {
        throw ReaderError.invalid("An obfuscated font has an invalid resource reference.")
      }
      let path = try ArchiveStore.canonicalPath(String(url.path.dropFirst()))
      let fonts: Set<String> = [
        "font/otf", "font/ttf", "font/woff", "font/woff2", "application/vnd.ms-opentype",
        "application/font-sfnt", "application/font-woff", "application/x-font-ttf",
        "application/x-font-opentype",
      ]
      guard let resource = resources[path], fonts.contains(resource.mediaType),
        paths.insert(path).inserted
      else {
        throw ReaderError.invalid(
          "Font obfuscation refers to a missing, repeated, or non-font resource.")
      }
    }
    return (paths, key)
  }

  static func decode(_ data: Data, key: [UInt8]) throws -> Data {
    var result = data
    for index in 0..<min(1040, result.count) { result[index] ^= key[index % key.count] }
    let signatures: [Data] = [
      Data([0, 1, 0, 0]), Data("OTTO".utf8), Data("true".utf8), Data("typ1".utf8),
      Data("ttcf".utf8), Data("wOFF".utf8), Data("wOF2".utf8),
    ]
    guard signatures.contains(Data(result.prefix(4))) else {
      throw ReaderError.invalid(
        "An obfuscated font could not be decoded using the book identifier. Publisher fonts may be unavailable."
      )
    }
    return result
  }
}
