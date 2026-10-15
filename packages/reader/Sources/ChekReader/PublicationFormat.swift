/// The publication containers that the reader is being built to open.
///
/// A format identifies the container; it does not imply that a particular file
/// has been validated or that all of its publication features are supported.
public enum PublicationFormat: String, Codable, CaseIterable, Sendable {
  case epub
  case cbz
}
