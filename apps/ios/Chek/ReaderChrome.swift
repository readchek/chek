import Observation
import SwiftUI

@Observable final class ReaderChrome {
  private(set) var visible = true
  @ObservationIgnored private var timer: Task<Void, Never>?
  @ObservationIgnored private var held = false

  func setHeld(_ held: Bool) {
    self.held = held
    if held { visible = true }
    interacted()
  }

  func toggle() {
    guard !held else { return }
    visible.toggle()
    interacted()
  }

  func interacted() {
    timer?.cancel()
    guard visible, !held else { return }
    timer = Task { [weak self] in
      do {
        try await Task.sleep(for: .seconds(3))
        try Task.checkCancellation()
        self?.visible = false
      } catch {}
    }
  }

  func stop() {
    timer?.cancel()
    timer = nil
  }
}

/// Chrome stays above a stable canvas; hiding it never changes the reader viewport.
struct ReaderChromeOverlay<Top: View, Bottom: View>: View {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  let visible: Bool
  @ViewBuilder let top: Top
  @ViewBuilder let bottom: Bottom

  var body: some View {
    GlassEffectContainer(spacing: 12) {
      if visible {
        VStack(spacing: 12) {
          top
          Spacer(minLength: 0)
          bottom
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .transition(.identity)
      }
    }
    .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: visible)
  }
}

struct ReaderGlassSurface: ViewModifier {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Namespace private var glassNamespace

  func body(content: Content) -> some View {
    content.glassEffect(.regular, in: .rect(cornerRadius: 24))
      .glassEffectID("surface", in: glassNamespace)
      .glassEffectTransition(reduceMotion ? .identity : .materialize)
  }
}

struct ReaderCloseButton: View {
  let disabled: Bool
  let action: () -> Void

  var body: some View {
    ReaderIconButton("Close", systemImage: "xmark", action: action)
      .disabled(disabled)
  }
}

struct ReaderIconButton: View {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Namespace private var glassNamespace
  let title: LocalizedStringKey
  let systemImage: String
  let action: () -> Void

  init(_ title: LocalizedStringKey, systemImage: String, action: @escaping () -> Void) {
    self.title = title
    self.systemImage = systemImage
    self.action = action
  }

  var body: some View {
    Button(action: action) {
      Label(title, systemImage: systemImage)
        .labelStyle(.iconOnly)
        .frame(minWidth: 44, minHeight: 44)
        .contentShape(.rect)
    }
    .buttonStyle(.glass)
    .glassEffectID(systemImage, in: glassNamespace)
    .glassEffectTransition(reduceMotion ? .identity : .materialize)
  }
}

struct ReaderRecoveryNotice: View {
  let message: String
  let dismiss: () -> Void

  var body: some View {
    HStack(alignment: .top) {
      Text(message).font(.callout)
      ReaderIconButton("Dismiss notice", systemImage: "xmark", action: dismiss)
    }
    .padding(12)
    .modifier(ReaderGlassSurface())
  }
}
