# Reader package and iOS app handoff

## Goal and agreed scope

Build Chek's first iOS app around local-file reading, with both EPUB and CBZ in
the first version. The reader is the core product and comes before provider
integration. Develop it as a reusable Swift package with a SwiftUI-facing API,
with Chek as its first consumer.

The latest decision is to build our own modern, lean reading engine. This
supersedes the earlier suggestion to start around Readium. Do not adopt Readium,
FolioReaderKit, epub.js, or Foliate-js as the engine without a new user decision.
Owning the engine does not mean recreating platform primitives: WebKit can lay
out EPUB HTML/CSS, and UIKit can support image zooming and gestures behind a
SwiftUI API. Small archive/XML dependencies can be evaluated on their merits.
Do not mistake SwiftUI-facing for a requirement to render everything with
SwiftUI Text or to strip EPUB formatting down to plain text.

Chek is a ground-up successor to Chibi Manga, a personal e-reader for e-books,
manga, manhwa, and comics. Users will eventually choose local files, Komga,
Kavita, or an optional Chek library server. The new server is intended to be a
standalone open-source library server, not the predecessor's Suwayomi translator.
Backend work may proceed alongside reader work in another task; it is not a
dependency or part of this task's implementation scope.

## Package and app boundary

Suggested layout, not an already-created scaffold:

```text
packages/reader/   Swift package: parsing, rendering, reading state and actions
apps/ios/         SwiftUI app consuming the local package
```

- Package: publication opening, EPUB/CBZ parsing, rendering, navigation,
  preferences, and serializable reading locations.
- App: file import UI, owned local copies, library, storing/restoring progress,
  and eventually providers, accounts, and sync.
- Expose enough state and actions for custom controls. Keep public APIs small,
  driven by working Chek usage rather than speculative format abstractions.
- Package branding, final package name, minimum OS, and shipping format coverage
  beyond EPUB/CBZ are still open. Use provisional names and verify the installed
  toolchain before choosing a target. Avoid unnecessarily requiring new APIs.

## Design direction

Native SwiftUI app, Apple system interface typography, and Liquid Glass controls.
Reading content stays unobstructed; glass is for controls and navigation, not a
translucent treatment over every page. Respect Dynamic Type, VoiceOver, reduced
motion, and native accessibility settings.

Choose paging gestures, tap zones, toolbar arrangement, scrolling defaults,
settings, and similar interaction details while using the app. Earlier assistant
recommendations were examples, not approved specifications. Do not block
implementation on a questionnaire about every interaction.

Start from neutral background/foreground separation, with restrained accents.
The website's cobalt palette and Bricolage font are not the app's design system.
Earlier charcoal/blue values in docs/design.md are historical, not a locked app
palette. Keep light-mode possibilities open.

## First working slice

Build the Swift package and minimal iOS consumer together. Start with EPUB since
its layout and reading-location behavior carry greater technical uncertainty.

1. Inspect repo instructions, installed Xcode/SDKs, simulator runtimes, and Serve
   Sim usage. Establish a buildable Swift package and iOS app.
2. Import a local EPUB using native file selection, safely access the selected
   file, and retain an app-owned copy. Parse container/package metadata, spine,
   navigation, and referenced resources. Render chapters with our own engine.
3. Make open → read → close → resume work. Reading locations must survive text
   size, viewport, and orientation changes; page indices alone are insufficient
   for reflowable EPUB. Decide location representation through actual examples.
4. Add native CBZ reading with deterministic page ordering, bounded image
   decoding/cache use, zoom, and long-strip behavior. Do not eagerly unpack and
   decode every page of a large archive.
5. Grow controls and the local library around that working reader. Refine
   interactions in the running app with the user.

This sequence is an incremental approach, not authorization to call a basic
chapter demo a complete EPUB implementation. Track supported EPUB behavior
honestly, including fixed-layout content. Do not silently discard book styles,
embedded fonts, illustrations, or navigation metadata.

Handle imported books as untrusted content: bound archive/resource processing,
prevent path traversal, and control book scripts, external navigation, and
network requests. These are implementation requirements for local files, not
user approval gates.

## Verification and tooling

- Always prefer Serve Sim for running and interactively checking the iOS app.
  Discover its installed CLI/API rather than guessing commands. Open its live
  preview in the Codex in-app browser and confirm the app renders before
  reporting success. Builds alone do not verify the reading experience.
- Use representative legal/public-domain or user-provided EPUB and CBZ files.
  Check chapter navigation, illustrations, fonts, text resizing, rotation, close
  and resume, large comic pages, and malformed files as the features land.
- Do not write automated tests unless the user explicitly asks. Existing build,
  compiler, formatter checks and manual simulator verification are authorized.
- Use apply_patch for focused edits. For structural transformations, inspect
  available language-aware tooling first; use reviewable patches when no
  suitable codemod exists. Do not rewrite source with ad hoc replacement scripts.
- Stay inside this repo. Preserve the website, API, and unrelated changes. Do
  not inspect or modify the old Chibi checkout without authorization.
- No commit, push, deployment, or backend implementation is requested by this
  handoff. Keep the root README project-facing, not a session journal.

## References and current checkout

Read these before implementing:

- [Product direction](product.md)
- [Design reference](design.md), subject to the clarifications above
- [Development](development.md) for the existing Bun workspaces
- [Repo instructions](../AGENTS.md), plus the working preferences in the task prompt

Apple's bundled Xcode guidance was exported unchanged into `.agents/skills/`:
eight SKILL.md files and 73 references. Consult the relevant skill before using
it, especially swiftui-specialist, building-document-based-swiftui-applications,
and uikit-app-modernization. The document-based skill is a reference, not a
decision to make Chek a DocumentGroup app. Skills describing OS 27 features must
be checked against the chosen deployment target. Also use the available
write-swift skill for modern Swift implementation.

At handoff, apps/api and apps/website exist; no iOS app or Swift reader package
has been created. The exported `.agents/` directory is untracked and must be
preserved. No reading engine has been implemented or verified.
