/// The blocking lifecycle of a reader canvas. A ready comic can still refine
/// its displayed image in the background without blocking interaction.
public enum ReaderPhase: Equatable, Sendable {
  /// No rendering operation is active. Mounting the reader view starts loading.
  case idle
  /// A request is pending, preparing resources, or waiting for viewport dimensions.
  case loading
  /// The document is being activated or its reading position is being restored.
  case restoring
  case ready
  case failed(String)

  var failureDescription: String? {
    if case .failed(let description) = self { description } else { nil }
  }
}
