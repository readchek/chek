import SwiftUI

struct ProgressRecoveryNotice: View {
  let message: String
  @State private var visible = true

  var body: some View {
    if visible {
      HStack(alignment: .top) {
        Text(message).font(.callout)
        Button("Dismiss notice", systemImage: "xmark") { visible = false }
          .labelStyle(.iconOnly)
          .frame(minWidth: 44, minHeight: 44)
          .contentShape(.rect)
      }
      .padding(.horizontal)
      .padding(.vertical, 8)
      .background(.bar)
    }
  }
}
