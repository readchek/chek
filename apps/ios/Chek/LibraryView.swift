import ChekReader
import SwiftUI

struct LibraryView: View {
  var body: some View {
    NavigationStack {
      ContentUnavailableView {
        Label("No books yet", systemImage: "books.vertical")
      } description: {
        Text("Your local books and comics will appear here.")
      }
      .navigationTitle("Library")
    }
  }
}
