import ChekReader
import SwiftUI

struct ComicReaderScreen: View {
  @Environment(\.dismiss) private var dismiss
  @Environment(\.scenePhase) private var scenePhase
  @State private var reader: CBZReader
  @State private var progress: ReadingProgressStore
  @State private var showingPages = false
  @State private var closing = false

  init(book: OpenBook, publication: CBZPublication) {
    _reader = State(
      initialValue: CBZReader(
        publication: publication,
        location: book.progress.comicLocation, direction: book.progress.comicDirection))
    _progress = State(initialValue: ReadingProgressStore(id: book.id, progress: book.progress))
  }

  var body: some View {
    @Bindable var reader = reader
    NavigationStack {
      CBZReaderView(reader: reader)
        .navigationTitle(reader.publication.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
          ToolbarItem(placement: .topBarLeading) {
            Button("Close", systemImage: "xmark") {
              guard !closing else { return }
              closing = true
              Task { if await saveLatest() { dismiss() } else { closing = false } }
            }
            .disabled(closing)
          }
          ToolbarItem(placement: .topBarTrailing) {
            Button("Pages", systemImage: "square.grid.2x2") { showingPages = true }
          }
          ToolbarItem(placement: .topBarTrailing) { ComicDirectionMenu(reader: reader) }
          ToolbarItem(placement: .bottomBar) { ComicPageControls(reader: reader) }
        }
        .sheet(isPresented: $showingPages) { ComicPagesSheet(reader: reader) }
        .onChange(of: reader.location) { updateProgress() }
        .onChange(of: reader.readingDirection) { updateProgress() }
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

  private func updateProgress() {
    progress.update(comicLocation: reader.location, readingDirection: reader.readingDirection)
  }

  private func saveLatest() async -> Bool {
    progress.update(
      comicLocation: reader.captureLocation(), readingDirection: reader.readingDirection)
    do {
      try await progress.flush()
      return true
    } catch {
      reader.message = error.localizedDescription
      return false
    }
  }
}

private struct ComicPageControls: View {
  let reader: CBZReader

  var body: some View {
    HStack {
      ComicPageButton(reader: reader, delta: reader.readingDirection == .leftToRight ? -1 : 1)
      Spacer()
      Text("Page \(reader.currentPageIndex + 1) of \(reader.publication.pages.count)")
        .font(.caption).foregroundStyle(.secondary)
      Spacer()
      ComicPageButton(reader: reader, delta: reader.readingDirection == .leftToRight ? 1 : -1)
    }
    .frame(maxWidth: .infinity)
  }
}

private struct ComicPageButton: View {
  let reader: CBZReader
  let delta: Int

  var body: some View {
    let backwards = (delta < 0) == (reader.readingDirection == .leftToRight)
    Button {
      reader.movePage(by: delta)
    } label: {
      Label {
        if delta < 0 { Text("Previous page") } else { Text("Next page") }
      } icon: {
        Image(systemName: backwards ? "chevron.left" : "chevron.right")
      }
      .labelStyle(.iconOnly)
    }
    .disabled(!reader.publication.pages.indices.contains(reader.currentPageIndex + delta))
  }
}

private struct ComicDirectionMenu: View {
  let reader: CBZReader

  var body: some View {
    Menu {
      Button {
        reader.setReadingDirection(.leftToRight)
      } label: {
        if reader.readingDirection == .leftToRight {
          Label("Left to right", systemImage: "checkmark")
        } else {
          Text("Left to right")
        }
      }
      Button {
        reader.setReadingDirection(.rightToLeft)
      } label: {
        if reader.readingDirection == .rightToLeft {
          Label("Right to left", systemImage: "checkmark")
        } else {
          Text("Right to left")
        }
      }
    } label: {
      Label("Reading direction", systemImage: "arrow.left.arrow.right")
    }
    .accessibilityValue(
      reader.readingDirection == .leftToRight ? Text("Left to right") : Text("Right to left"))
  }
}

private struct ComicPagesSheet: View {
  @Environment(\.dismiss) private var dismiss
  let reader: CBZReader

  var body: some View {
    NavigationStack {
      ScrollViewReader { scroll in
        List(Array(reader.publication.pages.enumerated()), id: \.element.id) { index, page in
          Button {
            reader.openPage(at: index)
            dismiss()
          } label: {
            HStack {
              Text("Page \(index + 1)")
              Spacer()
              if index == reader.currentPageIndex { Image(systemName: "checkmark") }
            }
          }
          .foregroundStyle(.primary)
          .id(page.id)
        }
        .onAppear { scroll.scrollTo(reader.currentPage.id, anchor: .center) }
      }
      .navigationTitle("Pages")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
      }
    }
  }
}
