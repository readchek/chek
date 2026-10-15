# Reader package validation

15 October 2026. Scope: installable EPUB/CBZ package; Chek remains a manual
verification harness. No automated tests were added, as instructed.

## Installation and composition

- The root SwiftPM product resolves independently of Bun and the Chek app.
- `examples/reader-consumer` builds through the public product for arm64 and
  x86_64 iOS Simulator. It imports no app types and includes no sample books.
- The existing harness builds in Release against the root package, including
  JavaScript and localized resources loaded through `Bundle.module`.
- Hosts own source files, library identity, persistence and controls. Readers
  expose locations, navigation, retry, errors and gesture opt-outs.
- Local installation is ready. Remote installation requires pushing an approved
  revision. Public redistribution still needs a source-license decision; the
  ZIPFoundation notice does not license this package's own code.

## Reliability changes

EPUB work checks navigation generation and document identity before publishing
results or failures. Initial resume survives unsuccessful activation. Replacing
a reader resets callback/load ownership and page navigation observation. JavaScript
restoration checks its operation after frame waits. Font and image settling share
one 2.5-second timeout and release event listeners when it expires. ZIP preflight
and XML parsing honor task cancellation. XML descendants accumulate into one
result rather than copying intermediate subtree arrays.

Comic canvases detach old capture providers when replacing readers. Continuous
mode cancels old requests and resets publication geometry. Page identity lookup
uses an index. A cache generation prevents suspended reads from repopulating an
evicted cache. Memory pressure cancels offscreen prefetch. Continuous mode includes
all intersecting pages, with an explicit error above 256 visible pages, at most two
pending decode requests, and a separate 64 MiB displayed-bitmap bound. The decode
cache also has a 64 MiB bound. These are not total process memory limits.
Resolution follows aggregate page area while retaining at least one decoded pixel
per zoomed layout point. Actual dimensions constrain the first strip decode before
bitmap allocation. Offscreen neighbors cannot evict visible images. Pages that
still exceed the budget fail visibly, with retry controls inside the visible
intersection. Prefetch resumes on canvas remount. The pathological page-count and
budget-error branches were code-reviewed, not exercised with generated fixtures.

## Focused simulator evidence

Xcode 27, Swift 6.4, iPhone 18 Pro simulator, iOS 27.0 (24A434), Release builds.
Serve Sim was active throughout. Baseline source was archived from `e2b5412`,
not built from a changing working tree. The hardened build includes the package
installation and lifecycle changes described above.

Two 15-second Time Profiler captures each included four alternating vertical
drags in Moby-Dick chapter 8. These are single exploratory runs, not repeatable
benchmark distributions: gesture timing and starting offsets were not identical.

| Observation | Baseline | Hardened |
| --- | ---: | ---: |
| CPU sample rows (1 ms nominal weight) | 183 | 131 |
| Main-thread sample weight | 153 ms | 107 ms |
| App physical footprint after flow | 30.6 MB | 30.3 MB |
| App peak footprint reported by vmmap | 32.3 MB | 32.0 MB |

Do not interpret these differences as a measured speedup. The trace attaches
only to the host app, excluding WebKit content/render processes and mirror costs.
Instruments warned that one table lacked a known input source; the exported
time-profile table contains samples. Animation Hitches explicitly failed with
“Hitches is not supported on this platform.” No FPS or hitch-rate result exists.

Local evidence (temporary artifacts, not package resources):

- `/tmp/chek-baseline-epub-cpu.trace`
- `/tmp/chek-hardened-epub-cpu.trace`
- `/tmp/chek-baseline-time-profile.xml`
- `/tmp/chek-hardened-time-profile.xml`
- `/tmp/chek-baseline-memory.txt`
- `/tmp/chek-hardened-memory.txt`
- `/tmp/chek-package-profile-build.log`
- `/tmp/chek-package-consumer-final.log`

Manual checks on the hardened build confirmed EPUB rendering, scrolling and
close; comic scrolling after a simulated memory warning; identical artwork
position after close/reopen; continuous-to-paged switching and a page 4-to-5
swipe; and return to automatic mode. Compiler and review checks cover the
reader-replacement and stale-result branches; those branches were not exercised
through a dedicated persistent-host UI in that initial pass.

The standalone `examples/reader-consumer/ExampleApp` now exercises replacement
without a host `.id()` reset. On the iOS 27 simulator, replacing a vertical-writing
EPUB preserved its location and delivered the next tap to the host. Changing the
host callback then delivered the new callback version. Four rapid text-size
increases settled at 140% with the document ready. This exposed and corrected an
appearance-based callback ownership bug; ownership now follows the concrete
attached view. Text-size requests are serialized and coalesced while WebKit
settles layout.

The independent consumer also opened a nine-page PNG CBZ, switched from automatic
paged mode to continuous scrolling, and rendered adjoining pages after a fast
scroll. Replacing its reader preserved page 8 at 23.1% and the next tap reached
the host. These checks establish integration behavior, not frame-time results.

The W3C mixed-layout sample exposed a crash when the same WebPage moved between
conditional WebView branches. A single stable WebView now changes frame and scale
across layout modes. Manual forward and reverse navigation through reflowable,
fixed, and reflowable sections rendered correctly after the fix.
The four-document SVG spine sample reached its final page. Vertical-rl text
rendered vertically. The malformed-XHTML sample now reports the XML diagnostic
through `failureDescription` and displays the native load-failure view, rather
than marking WebKit's parser-error page ready. On the combined build, birds-eye
automatically chose continuous mode and remained rendered after fast jumps to
page 5, followed by a successful close.

## Ownership and broader profiling follow-up

The independent consumer's Debug memory graph after malformed EPUB → comic →
close still contained one `EPUBReader`. Its reference tree led through the WebView
trait environment to a strong scroll-geometry callback capture. Weak reader
captures removed that chain. A fresh graph after the same flow contained no
`EPUBReader`, `CBZReader`, `ComicRollView`, or `CBZImagePipeline` instances. A second
graph after opening a normal EPUB, replacing it, receiving a tap, and closing also
contained none of those instances. This verifies those flows, not an unlimited
soak or a leak-free process. Both diagnostic leak scans reported six unattributed
32-byte allocations; their origin remains unresolved.

The first closed graph measured 31.5 MB host physical footprint (70.3 MB peak);
the corrected flow measured 30.9 MB (70.2 MB peak). After the additional normal
EPUB flow it measured 33.7 MB. These Debug observations exclude WebKit memory and
are not a controlled memory benchmark.

A separate 20-second diagnostic Time Profiler recording included four alternating
scroll gestures in Moby-Dick chapter 1. It contained 214 nominal 1 ms host samples
and 113 samples for the simulator WebKit content process. Despite specifying the
simulator device, all-process recording also collected host processes; only the
identified reader and simulator WebKit samples were analyzed. This confirms
broader process coverage, not a speedup or frame-time claim. The same unknown-table
warning occurred. Physical-device frame-time validation remains open.

Temporary evidence:

- `/tmp/chek-embedding-closed-memory/epub-retention.txt`
- `/tmp/chek-embedding-closed-memory-fixed/heap.txt`
- `/tmp/chek-embedding-normal-closed-memory/heap.txt`
- `/tmp/chek-reader-webkit-inclusive.trace`
- `/tmp/chek-reader-webkit-inclusive-profile.xml`
- `/tmp/chek-package-final-release.log`
- `/tmp/chek-package-consumer-validation.log`

The Release harness and independent public-product consumer both built after
these changes. The only Release warning was skipped App Intents metadata
extraction because the app has no App Intents dependency.

At maximum simulator Dynamic Type, the original failure view clipped its
diagnostic. Both formats now share a scrollable failure surface, and EPUB exposes
the same native retry action as CBZ. Manual verification reached the complete
malformed-XHTML diagnostic and Try again button by scrolling; retry returned a
controlled failure without a crash. The consumer moves detailed controls into a
scrollable sheet at accessibility sizes. The simulator text-size setting was
restored afterward. VoiceOver speech and gestures remain unverified.

## Release checks still open

Physical-device setup: the standalone Release demo was signed with the personal
individual team `KZX5HZ32P9`, installed on iPhone 17 / iOS 27.0.1, and visibly
rendered in Device Hub. Moby-Dick, birds-eye, and the mixed-layout sample were
copied into its own Documents container. Interactive device measurements are
still pending; installation and the initial screen do not verify scrolling or
frame times. Chek uses the requested `run.chibi.manga` identifier; the standalone
consumer remains `dev.chek.reader.embedding`.

- Repeatable device frame-time, loading latency and peak-memory measurements,
  including WebKit processes. Investigate the reported scroll lag end to end.
- Rapid cancellation/retry exercises beyond the verified reader replacements.
- Larger legal EPUB/CBZ corpus, malformed input and repeated open/close soak runs.
- VoiceOver speech/gestures and device lifecycle checks; maximum Dynamic Type was
  checked for the shared failure surface and consumer controls, not every UI path.
- Define the v1 format-support contract against the unsupported features in the
  package README. This work does not add EPUB roll, spreads, DRM or new codecs.

The package is internally installable and hardened, not certified feature-complete
or ready for a public release.
