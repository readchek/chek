import ChekReader
import SwiftUI
import UniformTypeIdentifiers

struct LibraryView: View {
  @State private var library = LibraryStore()
  @State private var importing = false

  var body: some View {
    @Bindable var library = library
    NavigationStack {
      Group {
        if library.books.isEmpty {
          ContentUnavailableView {
            Label("No books yet", systemImage: "books.vertical")
          } description: {
            Text("Import an EPUB or CBZ from Files to start reading.")
          } actions: {
            Button("Import book", systemImage: "plus") { importing = true }
              .buttonStyle(.glassProminent)
              .disabled(library.isBusy)
          }
        } else {
          List(library.books) { book in
            Button {
              Task { await library.open(book) }
            } label: {
              LibraryBookRow(title: book.title, author: book.author, format: book.format)
            }
            .disabled(library.isBusy)
          }
        }
      }
      .navigationTitle("Library")
      .toolbar {
        ToolbarItem(placement: .primaryAction) {
          Button("Import book", systemImage: "plus") { importing = true }
            .disabled(library.isBusy)
        }
      }
      .overlay {
        if library.isBusy {
          ProgressView("Opening book…")
            .padding()
            .background(.regularMaterial, in: .rect(cornerRadius: 16))
        }
      }
      .fileImporter(
        isPresented: $importing,
        allowedContentTypes: [
          .epub, .chekComic,
          UTType(filenameExtension: "cbz") ?? .chekComic,
        ]
      ) { result in
        switch result {
        case .success(let url): Task { await library.importBook(url) }
        case .failure(let error): library.message = error.localizedDescription
        }
      }
      .fullScreenCover(item: $library.openBook) { book in
        switch book.publication {
        case .epub(let publication): BookReaderScreen(book: book, publication: publication)
        case .cbz(let publication): ComicReaderScreen(book: book, publication: publication)
        }
      }
      .alert("Book could not open", isPresented: $library.hasMessage) {
        Button("OK", role: .cancel) {}
      } message: {
        Text(library.message ?? "")
      }
      .task { await library.load() }
      .onOpenURL { url in Task { await library.importBook(url) } }
    }
  }
}

private struct LibraryBookRow: View {
  let title: String
  let author: String
  let format: PublicationFormat

  var body: some View {
    HStack(spacing: 16) {
      Image(systemName: format == .cbz ? "photo.on.rectangle" : "book.closed")
        .font(.title2)
        .foregroundStyle(.secondary)
        .accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 4) {
        Text(title).font(.headline).foregroundStyle(.primary)
        if !author.isEmpty { Text(author).font(.subheadline).foregroundStyle(.secondary) }
      }
    }
    .padding(.vertical, 8)
    .accessibilityElement(children: .combine)
  }
}

extension UTType {
  static let chekComic = UTType(importedAs: "dev.chek.cbz", conformingTo: .zip)
}
