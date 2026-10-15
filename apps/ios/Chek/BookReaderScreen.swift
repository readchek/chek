import ChekReader
import SwiftUI

struct BookReaderScreen: View {
  @Environment(\.dismiss) private var dismiss
  @State private var reader: EPUBReader
  @State private var showingContents = false

  init(publication: EPUBPublication) {
    _reader = State(initialValue: EPUBReader(publication: publication))
  }

  var body: some View {
    @Bindable var reader = reader
    NavigationStack {
      EPUBReaderView(reader: reader)
        .navigationTitle(reader.publication.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
          ToolbarItem(placement: .topBarLeading) {
            Button("Close", systemImage: "xmark") { dismiss() }
          }
          ToolbarItem(placement: .topBarTrailing) {
            Button("Contents", systemImage: "list.bullet") { showingContents = true }
          }
          ToolbarItemGroup(placement: .bottomBar) {
            Button("Previous chapter", systemImage: "chevron.backward") {
              reader.moveChapter(by: -1)
            }
            .disabled(reader.chapterIndex == nil || reader.chapterIndex == 0)
            Spacer()
            if let index = reader.chapterIndex {
              Text("Section \(index + 1) of \(reader.publication.readingOrder.count)")
                .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Next chapter", systemImage: "chevron.forward") { reader.moveChapter(by: 1) }
              .disabled(
                reader.chapterIndex == nil
                  || reader.chapterIndex == reader.publication.readingOrder.count - 1)
          }
        }
        .sheet(isPresented: $showingContents) {
          ContentsSheet(reader: reader)
        }
        .alert("Reader", isPresented: $reader.hasMessage) {
          Button("OK", role: .cancel) {}
        } message: {
          Text(reader.message ?? "")
        }
    }
  }
}

private struct ContentsSheet: View {
  @Environment(\.dismiss) private var dismiss
  let reader: EPUBReader

  var body: some View {
    let links =
      reader.publication.tableOfContents.isEmpty
      ? reader.publication.readingOrder : reader.publication.tableOfContents
    NavigationStack {
      // The immutable TOC may legally contain repeated hrefs; its positions are stable.
      List(Array(links.enumerated()), id: \.offset) { index, link in
        Button {
          reader.open(link)
          dismiss()
        } label: {
          if link.title.isEmpty {
            Text("Section \(index + 1)")
          } else {
            Text(link.title).padding(.leading, CGFloat(min(link.depth, 8)) * 12)
          }
        }
        .foregroundStyle(.primary)
      }
      .navigationTitle("Contents")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
      }
    }
  }
}
