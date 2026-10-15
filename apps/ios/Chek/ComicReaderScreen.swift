import ChekReader
import SwiftUI

struct ComicReaderScreen: View {
  @Environment(\.dismiss) private var dismiss
  @Environment(\.scenePhase) private var scenePhase
  @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
  @State private var reader: CBZReader
  @State private var progress: ReadingProgressStore
  @State private var showingPages = false
  @State private var closing = false
  @State private var showingSettings = false
  @State private var showingRecovery: Bool
  @State private var chrome = ReaderChrome()
  private let progressWarning: String?

  init(book: OpenBook, publication: CBZPublication) {
    progressWarning = book.progressWarning
    _showingRecovery = State(initialValue: book.progressWarning != nil)
    _reader = State(
      initialValue: CBZReader(
        publication: publication,
        location: book.progress.comicLocation, direction: book.progress.comicDirection,
        readingMode: book.progress.comicReadingMode ?? .automatic))
    _progress = State(initialValue: ReadingProgressStore(id: book.id, progress: book.progress))
  }

  var body: some View {
    @Bindable var reader = reader
    CBZReaderView(reader: reader, onTap: chrome.toggle, onInteraction: chrome.interacted)
      .ignoresSafeArea()
      .overlay {
        ReaderChromeOverlay(visible: chrome.visible) {
          VStack(spacing: 8) {
            ComicReaderHeader(
              title: reader.publication.title, closing: closing, close: close,
              pages: { showingPages = true }, settings: { showingSettings = true })
            if let progressWarning, showingRecovery {
              ReaderRecoveryNotice(message: progressWarning) { showingRecovery = false }
            }
          }
        } bottom: {
          ComicPageControls(reader: reader, interacted: chrome.interacted)
        }
      }
      .toolbarVisibility(.hidden, for: .statusBar)
      .sheet(isPresented: $showingPages) { ComicPagesSheet(reader: reader) }
      .sheet(isPresented: $showingSettings) { ComicSettingsSheet(reader: reader) }
      .onChange(of: reader.location) { updateProgress() }
      .onChange(of: reader.readingDirection) { updateProgress() }
      .onChange(of: reader.readingMode) { updateProgress() }
      .onChange(of: progress.errorMessage) {
        if let error = progress.errorMessage { reader.message = error }
      }
      .onChange(of: scenePhase) {
        if scenePhase != .active { Task { _ = await saveLatest() } }
      }
      .onAppear { refreshChrome() }
      .onChange(of: showingPages) { refreshChrome() }
      .onChange(of: showingSettings) { refreshChrome() }
      .onChange(of: showingRecovery) { refreshChrome() }
      .onChange(of: voiceOver) { refreshChrome() }
      .onChange(of: reader.hasMessage) { refreshChrome() }
      .onChange(of: reader.isReadyForInteraction) { refreshChrome() }
      .onDisappear {
        chrome.stop()
        Task { try? await progress.flush() }
      }
      .alert("Reader", isPresented: $reader.hasMessage) {
        Button("OK", role: .cancel) {}
      } message: {
        Text(reader.message ?? "")
      }
      .interactiveDismissDisabled()
  }

  private func refreshChrome() {
    chrome.setHeld(
      !reader.isReadyForInteraction || voiceOver || showingPages || showingSettings
        || showingRecovery || reader.hasMessage
        || closing)
  }

  private func close() {
    guard !closing else { return }
    closing = true
    refreshChrome()
    Task {
      if await saveLatest() {
        dismiss()
      } else {
        closing = false
        refreshChrome()
      }
    }
  }

  private func updateProgress() {
    progress.update(
      comicLocation: reader.location, readingDirection: reader.readingDirection,
      readingMode: reader.readingMode)
  }

  private func saveLatest() async -> Bool {
    progress.update(
      comicLocation: reader.captureLocation(), readingDirection: reader.readingDirection,
      readingMode: reader.readingMode)
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
  let interacted: () -> Void

  var body: some View {
    HStack {
      ComicPageButton(
        reader: reader, delta: reader.readingDirection == .leftToRight ? -1 : 1,
        interacted: interacted)
      Text("Page \(reader.currentPageIndex + 1) of \(reader.publication.pages.count)")
        .font(.caption)
        .padding(.horizontal, 12).frame(minHeight: 44)
        .modifier(ReaderGlassSurface())
      ComicPageButton(
        reader: reader, delta: reader.readingDirection == .leftToRight ? 1 : -1,
        interacted: interacted)
    }
    .padding(.horizontal, 8)
  }
}

private struct ComicPageButton: View {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Namespace private var glassNamespace
  let reader: CBZReader
  let delta: Int
  let interacted: () -> Void

  var body: some View {
    let backwards = (delta < 0) == (reader.readingDirection == .leftToRight)
    Button {
      interacted()
      reader.movePage(by: delta)
    } label: {
      Label {
        if delta < 0 { Text("Previous page") } else { Text("Next page") }
      } icon: {
        Image(systemName: backwards ? "chevron.left" : "chevron.right")
      }
      .labelStyle(.iconOnly)
      .frame(minWidth: 44, minHeight: 44)
      .contentShape(.rect)
    }
    .buttonStyle(.glass)
    .glassEffectID(delta, in: glassNamespace)
    .glassEffectTransition(reduceMotion ? .identity : .materialize)
    .disabled(!reader.publication.pages.indices.contains(reader.currentPageIndex + delta))
  }
}

private struct ComicSettingsSheet: View {
  @Environment(\.dismiss) private var dismiss
  let reader: CBZReader

  var body: some View {
    NavigationStack {
      Form {
        Section("Reading mode") {
          ComicModeButton(reader: reader, mode: .automatic, title: "Automatic")
          ComicModeButton(reader: reader, mode: .continuous, title: "Continuous vertical")
          ComicModeButton(reader: reader, mode: .paged, title: "Pages")
          Text(
            "Automatic joins long strips into one vertical scroll. Pages supports horizontal swipes."
          )
          .font(.footnote).foregroundStyle(.secondary)
        }
        Section("Page direction") {
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
        }
      }
      .navigationTitle("Reading settings")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
      }
    }
    .presentationDetents([.medium, .large])
  }
}

private struct ComicModeButton: View {
  let reader: CBZReader
  let mode: CBZReadingMode
  let title: LocalizedStringKey

  var body: some View {
    Button {
      reader.setReadingMode(mode)
    } label: {
      HStack {
        Text(title)
        Spacer()
        if reader.readingMode == mode { Image(systemName: "checkmark") }
      }
    }
    .foregroundStyle(.primary)
  }
}

private struct ComicReaderHeader: View {
  let title: String
  let closing: Bool
  let close: () -> Void
  let pages: () -> Void
  let settings: () -> Void

  var body: some View {
    HStack(spacing: 8) {
      ReaderCloseButton(disabled: closing, action: close)
      Text(title).font(.headline).lineLimit(1)
        .padding(.horizontal, 12).frame(minHeight: 44)
        .modifier(ReaderGlassSurface())
      Spacer(minLength: 0)
      HStack(spacing: 0) {
        ReaderIconButton("Pages", systemImage: "square.grid.2x2", action: pages)
        ReaderIconButton("Reading settings", systemImage: "slider.horizontal.3", action: settings)
      }
      .labelStyle(.iconOnly)
    }
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
