import ChekReader
import SwiftUI

struct BookReaderScreen: View {
  @Environment(\.dismiss) private var dismiss
  @Environment(\.scenePhase) private var scenePhase
  @Environment(\.accessibilityVoiceOverEnabled) private var voiceOver
  @State private var reader: EPUBReader
  @State private var progress: ReadingProgressStore
  @State private var showingContents = false
  @State private var showingSettings = false
  @State private var closing = false
  @State private var chrome = ReaderChrome()
  @State private var showingRecovery: Bool
  private let progressWarning: String?

  init(book: OpenBook, publication: EPUBPublication) {
    progressWarning = book.progressWarning
    _showingRecovery = State(initialValue: book.progressWarning != nil)
    _reader = State(
      initialValue: EPUBReader(
        publication: publication,
        location: book.progress.location, textScale: book.progress.textScale))
    _progress = State(initialValue: ReadingProgressStore(id: book.id, progress: book.progress))
  }

  var body: some View {
    @Bindable var reader = reader
    EPUBReaderView(reader: reader, onTap: chrome.toggle, onInteraction: chrome.interacted)
      // Keep the hardware cutout clear independently of chrome visibility.
      .ignoresSafeArea(edges: .bottom)
      .overlay {
        ReaderChromeOverlay(visible: chrome.visible) {
          VStack(spacing: 8) {
            BookReaderHeader(
              reader: reader, closing: closing, close: close,
              contents: { showingContents = true }, settings: { showingSettings = true })
            if let progressWarning, showingRecovery {
              ReaderRecoveryNotice(message: progressWarning) { showingRecovery = false }
            }
          }
        } bottom: {
          BookChapterControls(reader: reader, interacted: chrome.interacted)
        }
      }
      #if os(iOS)
        .toolbarVisibility(.hidden, for: .statusBar)
      #endif
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
      .onAppear { refreshChrome() }
      .onChange(of: showingContents) { refreshChrome() }
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
      !reader.isReadyForInteraction || voiceOver || showingContents || showingSettings
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

private struct BookReaderHeader: View {
  let reader: EPUBReader
  let closing: Bool
  let close: () -> Void
  let contents: () -> Void
  let settings: () -> Void

  var body: some View {
    HStack(spacing: 8) {
      ReaderCloseButton(disabled: closing, action: close)
      Text(reader.publication.title)
        .font(.headline).lineLimit(1)
        .padding(.horizontal, 12).frame(height: 44)
        .modifier(ReaderGlassSurface())
      Spacer(minLength: 0)
      HStack(spacing: 8) {
        ReaderIconButton("Contents", systemImage: "list.bullet", action: contents)
        ReaderIconButton("Text size", systemImage: "textformat.size", action: settings)
          .disabled(reader.isFixedLayout)
      }
      .labelStyle(.iconOnly)
    }
  }
}

private struct BookChapterControls: View {
  let reader: EPUBReader
  let interacted: () -> Void

  var body: some View {
    HStack(spacing: 8) {
      ReaderIconButton("Previous chapter", systemImage: "chevron.backward", glass: false) {
        interacted()
        reader.moveChapter(by: -1)
      }
      .disabled(reader.chapterIndex == nil || reader.chapterIndex == 0)
      if let index = reader.chapterIndex {
        if reader.isFixedLayout {
          Text("Page \(index + 1) of \(reader.publication.readingOrder.count)")
            .font(.caption)
            .frame(height: 44)
        } else {
          Text("Section \(index + 1) of \(reader.publication.readingOrder.count)")
            .font(.caption)
            .frame(height: 44)
        }
      }
      ReaderIconButton("Next chapter", systemImage: "chevron.forward", glass: false) {
        interacted()
        reader.moveChapter(by: 1)
      }
      .disabled(
        reader.chapterIndex == nil
          || reader.chapterIndex == reader.publication.readingOrder.count - 1)
    }
    .labelStyle(.iconOnly)
    .modifier(ReaderGlassSurface(interactive: true))
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
      #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
      #endif
      .toolbar {
        ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
      }
    }
    #if os(iOS)
      .presentationDetents([.medium])
    #else
      .frame(minWidth: 360, minHeight: 240)
    #endif
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
  @State private var category = ContentsCategory.chapters
  let reader: EPUBReader

  var body: some View {
    let links =
      switch category {
      case .chapters:
        reader.publication.tableOfContents.isEmpty
          ? reader.publication.readingOrder : reader.publication.tableOfContents
      case .pages: reader.publication.pageList
      case .landmarks: reader.publication.landmarks
      }
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
      .safeAreaInset(edge: .top) {
        if !reader.publication.pageList.isEmpty || !reader.publication.landmarks.isEmpty {
          Picker("Navigation", selection: $category) {
            Text("Chapters").tag(ContentsCategory.chapters)
            if !reader.publication.pageList.isEmpty { Text("Pages").tag(ContentsCategory.pages) }
            if !reader.publication.landmarks.isEmpty {
              Text("Landmarks").tag(ContentsCategory.landmarks)
            }
          }
          .pickerStyle(.segmented)
          .padding()
        }
      }
      .navigationTitle("Contents")
      #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
      #endif
      .toolbar {
        ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
      }
    }
    #if os(macOS)
      .frame(minWidth: 480, minHeight: 360)
    #endif
  }
}

private enum ContentsCategory { case chapters, pages, landmarks }
