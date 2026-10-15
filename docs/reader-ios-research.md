# iOS reader research and decisions

Research date: 15 October 2026.

Chek can build its own EPUB and CBZ engine using current Apple primitives without
adopting another reading engine. The most useful recent developments are native
SwiftUI WebKit APIs and EPUB 3.4's proposed continuous roll layout for webtoons.
The harder work remains publication fidelity, durable reading locations, and
bounded image memory.

This research-only pass used a GPT-6.1 Sol subagent, relevant installed skills,
read-only toolchain inspection, and primary web sources. No app implementation,
dependency installation, build, or simulator session was performed in this pass.
The [handoff](reader-ios-handoff.md) remains the source of agreed scope.

## Decisions and proposals

| Topic                | Status                             | Direction                                                                                                                                           |
| -------------------- | ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Engine ownership     | Agreed in handoff                  | Build our own engine; no Readium, FolioReaderKit, epub.js, or Foliate-js wrapper.                                                                   |
| Product scope        | Agreed in handoff                  | Local EPUB and CBZ in v1; no server dependency.                                                                                                     |
| Package boundary     | Agreed in handoff                  | Package owns reading; app owns imports, library, and progress persistence. Keep public APIs driven by Chek usage.                                   |
| Platform primitives  | Agreed in handoff                  | WebKit and UIKit behind a SwiftUI API are allowed.                                                                                                  |
| Interface            | Agreed in handoff                  | Native Liquid Glass controls and system interface typography; refine interactions in the running app.                                               |
| Stable EPUB baseline | Research proposal                  | Target EPUB 3.3 compatibility, retain legacy EPUB 2 navigation support, and plan for EPUB 3.4 roll layout. This is not a claim of full conformance. |
| EPUB presentation    | Research proposal                  | Evaluate native WebView/WebPage first for iOS 26+; use a narrow WKWebView wrapper if measured control needs require it.                             |
| Parsing dependencies | Research proposal                  | Evaluate ZIPFoundation and start with Foundation XMLParser.                                                                                         |
| Reading locations    | Research proposal                  | Versioned semantic anchors with progression as fallback; stable entry identity plus within-image position for comics.                               |
| Storage              | Research proposal                  | App-owned original files, disposable bounded caches, and atomic progress writes.                                                                    |
| Content policy       | Requirement plus proposed defaults | Treat books as untrusted; disable book scripts and network access by default.                                                                       |
| Implementation       | Paused                             | This document does not authorize starting development.                                                                                              |

Minimum iOS version, final renderer, package name, persistence format, gestures,
paging defaults, and the exact supported EPUB feature matrix remain open.

## Current tools and API availability

Local inspection found Xcode 27.0 build 27A266a, Apple Swift 6.4
(`swiftlang-6.4.0.34.1`), and iOS/iOS Simulator SDK 27.0. Apple's release list
identifies that exact Xcode build as the 14 September 2026 release. Installed SDKs
do not determine Chek's minimum deployment target. Simulator runtime discovery
was blocked by sandbox access during this pass; it did not establish that
runtimes are absent. [Apple releases](https://developer.apple.com/news/releases/)

Swift 6.4 shipped on 15 September 2026; 6.4.1 was announced on 13 October. The
installed write-swift skill still calls 6.4 unreleased. Some exported OS 27 skill
headers also say “2027 releases.” Use current SDK availability declarations and
primary release sources when those labels conflict.
[Swift 6.4](https://www.swift.org/blog/swift-6.4-released/),
[Swift 6.4.1](https://forums.swift.org/t/announcing-swift-6-4-1/90029)

iOS 26 is a plausible candidate minimum for native SwiftUI WebKit and Liquid
Glass. iOS 27 adds useful APIs, but the research found no reader requirement that
alone warrants making it the minimum. Decide reach versus implementation cost
before scaffolding the app.

## EPUB standards and recent advances

EPUB 3.3 is the published Recommendation baseline, revised on 13 January 2026.
It covers the container, package, spine, navigation, XHTML/SVG resources, styles,
fonts, fixed layout, and more. A chapter viewer is only part of that behavior.
[EPUB 3.3](https://www.w3.org/TR/epub-33/)

EPUB 3.4 is a Candidate Recommendation Draft, dated 2 October 2026. Its
`rendition:layout="roll"` makes fixed-layout spine items fit the viewport width
and appear sequentially without gaps. This is directly relevant to manhwa and
webtoons. Keep roll distinct from ordinary reflowable scrolling; also consider
older fixed-layout books using `rendition:flow="scrolled-continuous"`.
[EPUB 3.4](https://www.w3.org/TR/epub-34/),
[reading-system rules](https://www.w3.org/TR/epub-rs-34/)

The draft adds media types including AVIF and JPEG XL and updates accessibility
metadata. W3C's October announcement stresses that outdated font obfuscation
must still work for existing books. A newer format recommendation does not remove
legacy compatibility needs, and a listed codec does not prove native decoding on
our chosen OS. [W3C announcement](https://www.w3.org/blog/2026/were-on-a-roll-epub-3-4-is-coming/)

EPUBCheck 5.4.0 includes EPUB 3.4 checks. Use it later to understand sample-file
validity; its result does not verify Chek's rendering, accessibility, or resume
behavior. [EPUBCheck changelog](https://github.com/w3c/epubcheck/blob/main/CHANGELOG.md)

## Native WebKit presentation

The iOS 26 SwiftUI WebKit surface includes WebView, observable WebPage, custom
URL scheme handling, navigation policy, JavaScript calls, scroll observation,
selection, magnification, and find UI. It deserves evaluation before assuming a
UIKit wrapper is necessary. [WebKit 26 overview](https://webkit.org/blog/17333/webkit-features-in-safari-26-0/)

The installed iOS 27 interface exposes WebPage configuration for a
userContentController, websiteDataStore, navigation preferences, and scheme
handlers. Script injection and local resource loading are not inherently missing
from the native API. JavaScript calls accept named arguments and a content world;
pass data as arguments instead of interpolating book text into code. Content
worlds isolate JavaScript namespaces, not the shared DOM.
[JavaScript API](<https://developer.apple.com/documentation/webkit/webpage/calljavascript(_:arguments:in:contentworld:)>)

WebPage also exposes `NavigationError.webContentProcessTerminated`. It would be
incorrect to choose WKWebView on the premise that only its delegate can signal
termination. Verify how persistent navigation observation receives termination
after a completed load, then recover from the last saved location.
[Termination error](https://developer.apple.com/documentation/webkit/webpage/navigationerror/webcontentprocessterminated)

| Concern               | Native WebView and WebPage                                 | When a WKWebView wrapper may help                                               |
| --------------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Publication resources | Custom URLSchemeHandler with async response/data sequences | If a required loading behavior fails in the native surface                      |
| DOM bridge            | Argument-based JavaScript and content-world support        | A demonstrated integration gap, not an assumed lack of scripts                  |
| Scroll restoration    | Concrete positions and observed geometry                   | Direct UIScrollView coordination                                                |
| Paging and gestures   | Must be evaluated against the chosen interaction           | Fine control over delegates, deceleration, insets, zoom, or gesture arbitration |
| Process recovery      | Explicit termination error exists                          | Only if runtime evidence shows recovery needs unavailable natively              |

Native scroll positions are offsets or edges, not semantic document identities.
Resolve reading anchors against the current DOM before scrolling. WKWebView's
public scrollView is the clear UIKit control advantage.
[Scroll position](<https://developer.apple.com/documentation/swiftui/view/webviewscrollposition(_:)>),
[scroll geometry](<https://developer.apple.com/documentation/swiftui/view/webviewonscrollgeometrychange(for:of:action:)>),
[WKWebView scrollView](https://developer.apple.com/documentation/webkit/wkwebview/scrollview)

iOS 27 introduces WKJSHandle, DOM serialization, and further content-world and
navigation APIs. These may improve later tooling or bridging; they do not replace
EPUB parsing or solve location persistence.
[WebKit 27 announcement](https://webkit.org/blog/17967/news-from-wwdc26-webkit-in-safari-27-beta/)

## Publication fidelity and locations

The proposed resource layer reads archive entries on demand, parses container and
package metadata, and resolves archive-relative URLs with fragments, percent
encoding, and document base paths. Use namespace-aware XML parsing rather than
hardcoded namespace prefixes. The spine defines reading order; the TOC does not.
Retain landmarks, page lists, non-linear items, and legacy NCX/guide data.

Preserve original XHTML, SVG, CSS, illustrations, and embedded fonts. Apply reader
preferences selectively: blanket font, margin, or position overrides can break
poetry, tables, vertical writing, and fixed-layout books. Fixed layout needs
viewport and scaling behavior of its own, including mixed-layout publications.
These are design implications of the [EPUB format](https://www.w3.org/TR/epub-33/),
not implemented capabilities.

The proposed versioned location contains a publication identity, resource href,
optional fragment, semantic DOM/text anchor, and progression fallback. Quote
context can aid recovery when a structural path fails, but repeated text and
changed editions remain ambiguous. Comics need entry identity and normalized
position within a page, especially for tall strips.

EPUB CFI is a useful interoperability reference. A smaller internal anchor may
be sufficient initially, provided it is not advertised as full CFI support.
Web Annotation text selectors supply useful fallback concepts.
[EPUB CFI](https://w3c.github.io/epub-specs/epub33/epubcfi/),
[annotation selectors](https://www.w3.org/TR/annotation-model/#selectors)

Pixels, page indices, and percentages alone are insufficient for reflow. Capture
an anchor before a font or viewport change, wait for relevant font/image layout,
then resolve it again. A navigation-finished event does not guarantee final
layout. Save during reading, not only during dismissal or app termination.

## Offline resources and untrusted content

Proposed defaults implement the handoff's security requirement:

- Disable book scripts; keep Chek's own bridge in a named content world.
- Give each publication an isolated resource namespace and validate decoded paths.
- Reject archive traversal, symlinks, ambiguous duplicate paths, and unsupported encryption.
- Bound entry count, actual expanded bytes, XML input/depth, and decoded image pixels.
- Block remote resources, forms, popups, and unexpected schemes; handle intentional external links natively.
- Prefer nonpersistent web storage unless a concrete feature needs persistence.

`allowsContentJavaScript = false` blocks publication scripts while allowing
app-injected scripts and app JavaScript calls. A navigation delegate alone does
not block all subresource requests. Combine resource boundaries, restrictive CSP,
and content rules; verify actual network behavior. Permit legitimate book styles
and intended local resource schemes without permitting remote execution.
[WebKit preference source](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKWebpagePreferences.h),
[CSP](https://webkit.org/blog/6830/a-refined-content-security-policy/),
[content rules](https://webkit.org/blog/3476/content-blockers-first-look/)

Keep XML external entity resolution disabled and apply independent resource
bounds. A custom resource scheme avoids a local HTTP server, but fonts, SVG,
MIME types, cross-document links, cancellation, and any media range requests
still need validation.
[XMLParser](https://developer.apple.com/documentation/foundation/xmlparser/shouldresolveexternalentities),
[URLSchemeHandler](https://developer.apple.com/documentation/webkit/urlschemehandler)

## CBZ and image memory

Define deterministic numeric-aware ordering with stable tie-breaking and archive
paths as page identities. Exclude resource-fork noise. ComicInfo supplies useful
cover, manga direction, page type, and spread hints, but does not remove ordering
decisions or replace inspection of image dimensions.
[ComicInfo](https://github.com/anansi-project/comicinfo),
[metadata fields](https://anansi-project.github.io/docs/comicinfo/documentation)

The proposed pipeline indexes once, decompresses visible and nearby pages, and
uses a bounded file cache for larger extracted resources. Inspect dimensions with
Image I/O, decode to a useful viewport size, and request more resolution for zoom.
Bound the decoded cache by byte cost, cancel obsolete work, and keep reading
direction separate from the interface's language direction.

Image I/O thumbnail creation supports a maximum pixel dimension; omitting it can
produce a source-sized image. Long strips need special attention: limiting the
longest edge can make the width unreadable. Cropping a fully decoded giant JPEG
does not avoid its decode cost. CATiledLayer controls tiled drawing, not guaranteed
region decoding. Measure before selecting a tiling strategy.
[Image I/O](https://developer.apple.com/documentation/imageio/cgimagesource),
[thumbnail bound](https://developer.apple.com/documentation/imageio/kcgimagesourcethumbnailmaxpixelsize),
[CATiledLayer](https://developer.apple.com/documentation/quartzcore/catiledlayer)

Start codec validation with JPEG/PNG. Enumerate runtime Image I/O types and decode
representative WebP, AVIF, and JPEG XL files before promising support. WebKit and
Image I/O are separate decoding paths.

## Dependencies and Swift architecture

| Candidate                                                  | Evidence                                                       | Proposed use                                                        |
| ---------------------------------------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------- |
| [ZIPFoundation](https://github.com/weichsel/ZIPFoundation) | Swift, MIT, per-entry/chunk APIs; latest listed release 0.9.20 | First archive candidate; keep CRC checks and enforce our own limits |
| [SSZipArchive](https://github.com/ZipArchive/ZipArchive)   | Objective-C, MIT; bundled minizip-ng has Zlib licensing        | Alternative if a concrete limitation warrants it                    |
| [Zip](https://github.com/marmelroy/Zip)                    | MIT convenience layer over minizip                             | Less suited to the desired controlled resource layer                |
| Foundation XMLParser                                       | System streaming parser with namespace support                 | Initial metadata parser                                             |
| [SWXMLHash](https://github.com/drmohundro/SWXMLHash)       | MIT dictionary-like XML wrapper                                | Optional convenience; no initial need established                   |

ZIPFoundation's release notes include resource and memory fixes. Pin and inspect
the selected version when implementation begins; licensing and popularity do not
prove malformed-input safety. No dependency was added by this pass.
[ZIPFoundation releases](https://github.com/weichsel/ZIPFoundation/releases)

Use value types for metadata, preferences, and locations; concrete types before
speculative protocols. Keep UI state on the main actor and expensive resource
work out of rendering callbacks. Under modern concurrency settings, marking work
async alone does not guarantee off-main execution. Prefer structured cancellation
and bounded tasks. New Swift 6.4 ownership and memory features are options for
measured needs, not prerequisites for the first reader.
[Apple concurrency guidance](https://developer.apple.com/videos/play/wwdc2025/266/)

Use native file import, balance security-scoped access, coordinate file-provider
reads where needed, and stage an app-owned copy before committing a library
record. Keep originals separate from disposable caches. Atomic Codable storage
is a candidate starting point; neither SwiftData nor a database is yet required.
[File importer](<https://developer.apple.com/documentation/swiftui/view/fileimporter(ispresented:allowedcontenttypes:oncompletion:)>),
[file coordination](https://developer.apple.com/documentation/uikit/providing-access-to-directories)

## Skills and manual verification

The research reviewed write-swift; the exported SwiftUI specialist, OS 27,
document-based app, and UIKit modernization guidance; and the build-ios-apps
Liquid Glass, simulator browser, debugger, performance, ETTrace, and memgraph
skills. Use the exported Apple references for view identity, observation,
localization, and window context. DocumentGroup remains an option, not a decision
implied by reading the document-app skill. ETTrace and memgraph tools are later
diagnostic options, not required additions to the app.

Follow the existing Serve Sim preference when implementation resumes: select an
explicit device, inspect current CLI usage, open the printed mirror URL in Codex's
browser, and verify an actual rendered frame. A successful build is not reading
verification. Use Bun for package CLI invocation and avoid stopping unrelated
mirrors.

Use existing [W3C EPUB fixtures](https://github.com/w3c/epub-tests) with their
[license](https://github.com/w3c/epub-tests/blob/main/LICENSE.md) and expected
behaviors for manual checks. Check individual licenses in
[IDPF samples](https://github.com/idpf/epub3-samples).
[Standard Ebooks](https://standardebooks.org/) offers realistic books but describes
public-domain status in the United States; confirm suitability for the intended
use. Select explicitly licensed comic images or user-provided CBZ files; this
pass did not obtain or validate a representative CBZ corpus.

| Later manual check     | Evidence needed                                                                  |
| ---------------------- | -------------------------------------------------------------------------------- |
| Import and persistence | File-provider import, app-owned copy, failed-import cleanup, relaunch            |
| EPUB fidelity          | Chapters, TOC, links, images, embedded fonts, tables, RTL/vertical text          |
| Resume                 | Same passage after close, restart, font changes, rotation, and viewport changes  |
| Fixed layout           | Correct viewport, scale, spreads, mixed layout; separate roll evaluation         |
| Comics                 | Numeric ordering, zoom, direction, long strips, rapid navigation, bounded memory |
| Accessibility          | VoiceOver content and controls, Dynamic Type, Reduce Motion/Transparency         |
| Failure recovery       | Malformed archives, oversized resources, denied network loads, web process loss  |
| Resource lifetime      | Repeated open/close and backgrounding without retained readers or runaway caches |

No new automated tests are authorized. Existing validators and manual fixture
use do not establish app correctness until the actual reader is exercised.

## Recommended sequence when implementation resumes

1. Choose the deployment baseline and evaluate native WebPage resource loading,
   DOM anchoring, and the intended scroll behavior with a few representative books.
2. Complete import, read, close, and semantic resume with app-owned local files.
3. Establish the EPUB feature matrix, including fixed-layout and unsupported-content behavior.
4. Build bounded CBZ decoding, zoom, direction, and long-strip reading.
5. Refine native controls and accessibility in Serve Sim; profile concrete issues.

The first prototype should settle these risks before fixing a large package API.
The renderer, dependencies, and location schema above are research recommendations
to validate, not newly approved product decisions.
