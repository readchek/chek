import ChekReader
import SwiftUI
import UniformTypeIdentifiers

@main
struct ReaderEmbeddingApp: App {
  var body: some Scene {
    WindowGroup {
      EmbeddingView()
        #if os(macOS)
          .frame(minWidth: 640, minHeight: 480)
        #endif
    }
  }
}

private struct EmbeddingView: View {
  @Environment(\.scenePhase) private var scenePhase
  @State private var session = EmbeddingSession()
  @State private var showFiles = false
  @State private var showImporter = false

  var body: some View {
    NavigationStack {
      VStack(spacing: 0) {
        SessionStatus(session: session)
        SessionCanvas(session: session)
      }
      .navigationTitle("Reader Embedding")
      #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
      #endif
      .toolbar {
        ToolbarItem(placement: .navigation) {
          Button {
            session.refreshFiles()
            showFiles = true
          } label: {
            ToolbarIcon("Files", systemImage: "folder")
          }
          .buttonStyle(.plain)
        }
        .sharedBackgroundVisibility(.hidden)
        ToolbarItem(placement: .primaryAction) {
          Menu {
            Button("Import file", systemImage: "square.and.arrow.down") { showImporter = true }
            Button("Replace session", systemImage: "arrow.triangle.2.circlepath") {
              session.replace()
            }
            .disabled(session.reader == nil)
            Button("Close session", systemImage: "xmark") { session.close() }
              .disabled(session.reader == nil)
            Toggle("Reader swipe navigation", isOn: $session.allowsSwipes)
            Button("Change tap callback") { session.callbackVersion += 1 }
          } label: {
            ToolbarIcon("Session", systemImage: "ellipsis")
          }
          .menuIndicator(.hidden)
          .buttonStyle(.plain)
          #if os(macOS)
            .menuStyle(.borderlessButton)
          #endif
        }
        .sharedBackgroundVisibility(.hidden)
      }
      .sheet(isPresented: $showFiles) {
        NavigationStack {
          List(session.files, id: \.self) { url in
            Button(url.lastPathComponent) {
              session.open(url)
              showFiles = false
            }
          }
          .overlay {
            if session.files.isEmpty {
              ScrollView {
                ContentUnavailableView(
                  "No publications", systemImage: "folder",
                  description: Text(
                    "Import a local EPUB or CBZ to start reading."))
              }
            }
          }
          .navigationTitle("Local files")
          .toolbar {
            Button("Import") {
              showFiles = false
              showImporter = true
            }
          }
        }
        #if os(macOS)
          .frame(minWidth: 480, minHeight: 360)
        #endif
      }
      .fileImporter(
        isPresented: $showImporter,
        allowedContentTypes: [
          UTType(filenameExtension: "epub") ?? .data,
          UTType(filenameExtension: "cbz") ?? .zip,
        ]
      ) { result in
        switch result {
        case .success(let url): session.importFile(url)
        case .failure(let error): session.error = error.localizedDescription
        }
      }
      .task { session.refreshFiles() }
      .onChange(of: scenePhase) { _, phase in
        if phase != .active { Task { await session.save() } }
      }
    }
  }
}

private struct ToolbarIcon: View {
  let title: LocalizedStringKey
  let systemImage: String

  init(_ title: LocalizedStringKey, systemImage: String) {
    self.title = title
    self.systemImage = systemImage
  }

  private var size: CGFloat {
    #if os(macOS)
      32
    #else
      44
    #endif
  }

  var body: some View {
    Label(title, systemImage: systemImage)
      .labelStyle(.iconOnly)
      .frame(width: size, height: size)
      .contentShape(.circle)
      .glassEffect(.regular.interactive(), in: .circle)
  }
}

private struct SessionCanvas: View {
  let session: EmbeddingSession

  var body: some View {
    let callbackVersion = session.callbackVersion
    // No .id(), placeholder detour, or cleared session on replacement.
    // Same-format replacement updates the existing reader view's input.
    switch session.reader {
    case .epub(let reader):
      EPUBReaderView(
        reader: reader,
        onTap: {
          session.tapCount += 1
          session.lastCallbackVersion = callbackVersion
        }
      )
      .modifier(ProgressAutosave(session: session, reader: .epub(reader)))
    case .cbz(let reader):
      CBZReaderView(
        reader: reader,
        onTap: {
          session.tapCount += 1
          session.lastCallbackVersion = callbackVersion
        }
      )
      .modifier(ProgressAutosave(session: session, reader: .cbz(reader)))
    case nil:
      GeometryReader { geometry in
        ScrollView {
          ContentUnavailableView(
            "Open a publication", systemImage: "book",
            description: Text("Choose Files or import a local EPUB or CBZ from the Session menu.")
          )
          .frame(minHeight: geometry.size.height)
        }
      }
    }
  }
}

private struct ProgressAutosave: ViewModifier {
  let session: EmbeddingSession
  let reader: EmbeddingSession.Reader

  func body(content: Content) -> some View {
    switch reader {
    case .epub(let reader):
      content.onChange(of: reader.location) { session.scheduleAutosave() }
    case .cbz(let reader):
      content.onChange(of: reader.location) { session.scheduleAutosave() }
    }
  }
}

private struct SessionStatus: View {
  @Environment(\.dynamicTypeSize) private var dynamicTypeSize
  @State private var showDetails = false
  let session: EmbeddingSession

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      if dynamicTypeSize.isAccessibilitySize {
        SessionReadiness(session: session)
        Button("Reader controls", systemImage: "slider.horizontal.3") { showDetails = true }
      } else {
        SessionDetails(session: session)
      }
    }
    .font(.caption)
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(.horizontal)
    .padding(.vertical, 8)
    .background(.background)
    .sheet(isPresented: $showDetails) {
      NavigationStack {
        ScrollView {
          SessionDetails(session: session)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .navigationTitle("Reader controls")
        #if os(iOS)
          .navigationBarTitleDisplayMode(.inline)
        #endif
        .toolbar {
          Button("Done") { showDetails = false }
        }
      }
      #if os(macOS)
        .frame(minWidth: 480, minHeight: 360)
      #endif
    }
  }
}

private struct SessionReadiness: View {
  let session: EmbeddingSession

  var body: some View {
    switch session.reader {
    case .epub(let reader):
      Text(
        reader.failureDescription != nil
          ? "Failed" : (reader.isReadyForInteraction ? "Ready" : "Loading"))
    case .cbz(let reader):
      Text(
        reader.failureDescription != nil
          ? "Failed" : (reader.isReadyForInteraction ? "Ready" : "Loading"))
    case nil: Text(session.isOpening ? "Opening" : "Closed")
    }
  }
}

private struct SessionDetails: View {
  let session: EmbeddingSession

  var body: some View {
    VStack(alignment: .leading, spacing: 4) {
      Text(session.filename).lineLimit(1)
      Text("Session \(session.generation) · Taps \(session.tapCount)")
      Text(
        "Callback \(session.callbackVersion) · Last tap \(session.lastCallbackVersion.map(String.init) ?? "none")"
      )
      switch session.reader {
      case .epub(let reader):
        Text(reader.failureDescription ?? (reader.isReadyForInteraction ? "Ready" : "Loading"))
        Text(reader.location?.resourceHref ?? reader.currentHref).lineLimit(1)
        ChapterControls(reader: reader)
        TextScaleControls(reader: reader)
      case .cbz(let reader):
        Text(reader.failureDescription ?? (reader.isReadyForInteraction ? "Ready" : "Loading"))
        Text(
          "Page \(reader.currentPageIndex + 1) · \(reader.location.progression.formatted(.percent.precision(.fractionLength(1))))"
        )
        ComicModeMenu(reader: reader)
      case nil: Text("Closed")
      }
      if session.isOpening { ProgressView("Opening replacement") }
      if let error = session.error { Text(error).foregroundStyle(.red) }
    }
  }
}

private struct ChapterControls: View {
  @Environment(\.dynamicTypeSize) private var dynamicTypeSize
  let reader: EPUBReader

  var body: some View {
    let href = reader.isReadyForInteraction ? reader.currentHref : reader.requestedHref
    let path = href.split(separator: "#", maxSplits: 1).first.map(String.init)
    let index = reader.publication.readingOrder.firstIndex(where: { $0.href == path })
    let layout =
      dynamicTypeSize.isAccessibilitySize
      ? AnyLayout(VStackLayout(alignment: .leading)) : AnyLayout(HStackLayout())
    layout {
      Button("Previous chapter", systemImage: "chevron.backward") { reader.moveChapter(by: -1) }
        .disabled(index == nil || index == 0)
      Button("Next chapter", systemImage: "chevron.forward") { reader.moveChapter(by: 1) }
        .disabled(index == nil || index == reader.publication.readingOrder.count - 1)
    }
    .buttonStyle(.bordered)
  }
}

private struct TextScaleControls: View {
  @Environment(\.dynamicTypeSize) private var dynamicTypeSize
  let reader: EPUBReader

  var body: some View {
    let layout =
      dynamicTypeSize.isAccessibilitySize
      ? AnyLayout(VStackLayout(alignment: .leading)) : AnyLayout(HStackLayout())
    layout {
      Button("Smaller", systemImage: "textformat.size.smaller") {
        Task { await reader.setTextScale(reader.textScale - 0.1) }
      }
      Text(reader.textScale.formatted(.percent.precision(.fractionLength(0))))
      Button("Larger", systemImage: "textformat.size.larger") {
        Task { await reader.setTextScale(reader.textScale + 0.1) }
      }
    }
    .buttonStyle(.bordered)
  }
}

private struct ComicModeMenu: View {
  let reader: CBZReader

  var body: some View {
    Menu {
      Button("Automatic", systemImage: reader.readingMode == .automatic ? "checkmark" : "circle") {
        reader.setReadingMode(.automatic)
      }
      Button("Paged", systemImage: reader.readingMode == .paged ? "checkmark" : "circle") {
        reader.setReadingMode(.paged)
      }
      Button("Continuous", systemImage: reader.readingMode == .continuous ? "checkmark" : "circle")
      {
        reader.setReadingMode(.continuous)
      }
    } label: {
      Label("Reading mode: \(reader.effectiveReadingMode.rawValue)", systemImage: "book.pages")
    }
    .buttonStyle(.bordered)
  }
}
