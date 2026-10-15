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
    GlassEffectContainer(spacing: 0) {
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
  var interactive = false

  func body(content: Content) -> some View {
    content.glassEffect(.regular.interactive(interactive), in: .rect(cornerRadius: 22))
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
  @Environment(\.isEnabled) private var isEnabled
  @Namespace private var glassNamespace
  let title: LocalizedStringKey
  let systemImage: String
  let action: () -> Void
  let glass: Bool

  init(
    _ title: LocalizedStringKey, systemImage: String, glass: Bool = true,
    action: @escaping () -> Void
  ) {
    self.title = title
    self.systemImage = systemImage
    self.action = action
    self.glass = glass
  }

  var body: some View {
    Button(action: action) {
      Label(title, systemImage: systemImage)
        .labelStyle(.iconOnly)
        .opacity(isEnabled ? 1 : 0.4)
        .frame(width: 44, height: 44)
        .contentShape(.circle)
    }
    .buttonStyle(.plain)
    .modifier(ReaderIconSurface(enabled: glass))
    .glassEffectID(systemImage, in: glassNamespace)
    .glassEffectTransition(reduceMotion ? .identity : .materialize)
  }
}

private struct ReaderIconSurface: ViewModifier {
  let enabled: Bool

  func body(content: Content) -> some View {
    if enabled {
      content.glassEffect(.regular.interactive(), in: .circle)
    } else {
      content
    }
  }
}

struct ReaderRecoveryNotice: View {
  let message: String
  let dismiss: () -> Void

  var body: some View {
    HStack(alignment: .top) {
      Text(message).font(.callout)
      ReaderIconButton("Dismiss notice", systemImage: "xmark", glass: false, action: dismiss)
    }
    .padding(12)
    .modifier(ReaderGlassSurface())
  }
}
