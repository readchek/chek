import ChekReader
import SwiftUI

/// An embedding example using only the public reader product.
/// The host keeps its imported file alive and stores locations separately.
public struct ReaderContent: View {
  private enum Session {
    case epub(EPUBReader)
    case cbz(CBZReader)
  }

  private struct Request: Hashable {
    let url: URL
    let format: PublicationFormat
  }

  private let url: URL
  private let format: PublicationFormat
  @State private var session: Session?
  @State private var failure: String?

  public init(url: URL, format: PublicationFormat) {
    self.url = url
    self.format = format
  }

  public var body: some View {
    Group {
      switch session {
      case .epub(let reader):
        EPUBReaderView(reader: reader)
      case .cbz(let reader):
        CBZReaderView(reader: reader)
      case nil:
        if let failure {
          ContentUnavailableView(
            "Could not open publication", systemImage: "book.closed",
            description: Text(failure))
        } else {
          ProgressView("Opening publication")
        }
      }
    }
    .task(id: Request(url: url, format: format)) {
      session = nil
      failure = nil
      do {
        let opened: Session
        switch format {
        case .epub:
          opened = .epub(EPUBReader(publication: try await EPUBPublication.open(url)))
        case .cbz:
          opened = .cbz(CBZReader(publication: try await CBZPublication.open(url)))
        }
        try Task.checkCancellation()
        session = opened
      } catch is CancellationError {
      } catch {
        if !Task.isCancelled { failure = error.localizedDescription }
      }
    }
  }
}
