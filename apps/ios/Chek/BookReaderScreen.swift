import ChekReader
import SwiftUI

struct BookReaderScreen: View {
  @Environment(\.dismiss) private var dismiss
  @Environment(\.scenePhase) private var scenePhase
  @State private var reader: EPUBReader
  @State private var progress: ReadingProgressStore
  @State private var showingContents = false
  @State private var showingSettings = false
  @State private var closing = false

  init(book: OpenBook) {
    _reader = State(
      initialValue: EPUBReader(
        publication: book.publication,
        location: book.progress.location, textScale: book.progress.textScale))
    _progress = State(initialValue: ReadingProgressStore(id: book.id, progress: book.progress))
  }

  var body: some View {
    @Bindable var reader = reader
    NavigationStack {
      EPUBReaderView(reader: reader)
        .navigationTitle(reader.publication.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
          ToolbarItem(placement: .topBarLeading) {
            Button("Close", systemImage: "xmark") {
              guard !closing else { return }
              closing = true
              Task {
                if await saveLatest() { dismiss() } else { closing = false }
              }
            }
            .disabled(closing)
          }
          ToolbarItem(placement: .topBarTrailing) {
            Button("Contents", systemImage: "list.bullet") { showingContents = true }
          }
          ToolbarItem(placement: .topBarTrailing) {
            Button("Text size", systemImage: "textformat.size") { showingSettings = true }
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
        .sheet(isPresented: $showingSettings) { TextSettingsSheet(reader: reader) }
        .onChange(of: reader.location) {
          progress.update(location: reader.location, textScale: reader.textScale)
        }
        .onChange(of: reader.textScale) {
          progress.update(location: reader.location, textScale: reader.textScale)
        }
        .onChange(of: progress.errorMessage) {
          if let error = progress.errorMessage { reader.message = error }
        }
        .onChange(of: scenePhase) {
          if scenePhase != .active { Task { _ = await saveLatest() } }
        }
        .onDisappear { Task { try? await progress.flush() } }
        .alert("Reader", isPresented: $reader.hasMessage) {
          Button("OK", role: .cancel) {}
        } message: {
          Text(reader.message ?? "")
        }
    }
    .interactiveDismissDisabled()
  }

  private func saveLatest() async -> Bool {
    let location = await reader.captureLocation()
    progress.update(location: location, textScale: reader.textScale)
    do {
      try await progress.flush()
      return true
    } catch {
      reader.message = error.localizedDescription
      return false
    }
  }
}

private struct TextSettingsSheet: View {
  @Environment(\.dismiss) private var dismiss
  @State private var adjusting = false
  let reader: EPUBReader

  var body: some View {
    NavigationStack {
      Form {
        LabeledContent("Text size") {
          Text(reader.textScale, format: .percent.precision(.fractionLength(0)))
        }
        HStack {
          Button("Smaller", systemImage: "textformat.size.smaller") { adjust(by: -0.125) }
            .disabled(adjusting || reader.textScale <= 0.75)
          Spacer()
          Button("Larger", systemImage: "textformat.size.larger") { adjust(by: 0.125) }
            .disabled(adjusting || reader.textScale >= 2)
        }
        Button("Publisher size") {
          adjusting = true
          Task {
            await reader.setTextScale(1)
            adjusting = false
          }
        }
        .disabled(adjusting || reader.textScale == 1)
      }
      .navigationTitle("Text size")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
      }
    }
    .presentationDetents([.medium])
  }

  private func adjust(by delta: Double) {
    adjusting = true
    Task {
      await reader.setTextScale(reader.textScale + delta)
      adjusting = false
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
