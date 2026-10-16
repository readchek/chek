import SwiftUI

@main
struct ChekApp: App {
  var body: some Scene {
    WindowGroup {
      LibraryView()
        #if os(macOS)
          .frame(minWidth: 640, minHeight: 480)
        #endif
    }
  }
}
