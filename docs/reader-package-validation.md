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

| Observation                           | Baseline | Hardened |
| ------------------------------------- | -------: | -------: |
| CPU sample rows (1 ms nominal weight) |      183 |      131 |
| Main-thread sample weight             |   153 ms |   107 ms |
| App physical footprint after flow     |  30.6 MB |  30.3 MB |
| App peak footprint reported by vmmap  |  32.3 MB |  32.0 MB |

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

## Physical-device follow-up

Device Hub interaction on iPhone 17 / iOS 27.0.1 confirmed Moby-Dick chapter
navigation and scrolling. Replacing its reader preserved the visible passage.
Birds-eye chose continuous mode and reached its final artwork after fast scrolls.
An initial 90.8% → 66.5% observation included elastic overscroll and does not by
itself prove replacement drift. Review found that estimated page heights could
clamp a requested anchor before real dimensions arrived, and that bounce offsets
could enter saved progress. Restoration now retains the requested anchor until
target dimensions arrive; the patched device build preserved settled page 11 at
66.5% with identical artwork after replacement. The final signed build also passed
the physical bounce check: an upward drag beyond the bottom kept reported progress
at 66.5% during overscroll and after settling. Replacement (session 1 → 2) and
close/reopen (session 2 → 3) preserved page 11, 66.5%, and the same final artwork.

A 31.10-second Animation Hitches recording targeted the physical device and
recorded no entries in its hitches or potential-hangs tables. It contained 486
ReaderEmbedding update events (median 0.78 ms, p95 1.30 ms, maximum 1.81 ms).
Matching swap IDs yielded 487 render-stage records (median 4.93 ms, p95 6.24 ms,
maximum 17.55 ms). These are instrument stage timings, not FPS or input latency.
The device was in Fair thermal state throughout, Device Hub streaming was active,
and gesture intervals were not precisely marked. This short run does not settle
the original lag report. Direct process attachment failed; device-wide recording
succeeded. WebKit CPU attribution remains inferred from process timing/context.

Evidence: `/tmp/chek-device-epub-hitches.trace` and its XML exports, including
`/tmp/chek-device-hitches-updates.xml` and `/tmp/chek-device-hitches.xml`.

Mixed-layout navigation initially reported a generic WebPage navigation error.
Retry loaded the fixed second page; forward to the third reflowable section and
back to the fixed page then succeeded. Its original cause is not yet established.
The reader now reports specific WebKit failure cases and resubscribes to navigation
events after explicit loading becomes ready; a failed event sequence previously
could leave later process failures unobserved. Page identity and weak captures
remain intact. The signed Release build passes (`/tmp/chek-device-anchor-final-build.log`),
as do Swift formatting and diff checks. These changes do not claim to resolve the
unidentified initial device error. The final physical repeat passed: the saved
fixed page loaded, reverse navigation reached the first reflowable section, and
separate forward swipes reached the fixed second page and third reflowable section.
Reverse navigation from the third section also reached the fixed page. Every
observed destination reported Ready, with no retry required. This does not prove
the earlier intermittent error cannot recur.

Device Hub initially stopped delivering screen taps despite a live stream and
working Home command. Direct phone input worked. Quitting Device Hub with
“Keep Simulators Running”, reopening it, and starting View Screen restored taps;
no phone restart or simulator shutdown was needed. The user subsequently checked
fast scrolling directly on the phone and corrected the apparent lag report:
scrolling on device was fine, while the stream was lagging. Treat this as a
qualitative direct-device observation, not a measured frame-rate guarantee.
A read-only EPUB audit found idle-debounced semantic capture, throttled interaction
messages, and no per-scroll persistence in the demo; no speculative performance
change was made in response to the streaming symptom.

The next physical session returned to Moby-Dick with a reported WebKit content
process termination. Try again restored chapter 1 and Ready without restarting
the host. The prior position at the time of termination was not observed, so this
does not establish termination-time position recovery. After a session replacement,
scrolling reached a distinct passage near the chapter end. Background/foreground
and two close/reopen cycles preserved that passage. Switching to birds-eye, scrolling
to page 11 at 66.5%, closing, and reopening also preserved the final artwork.
These are short lifecycle checks, not a memory soak or forced cancellation proof.

A subsequent cold-process relaunch (terminate-existing, after backgrounding) reset
the demo session counter to zero. Reopening each file restored Moby-Dick's same
near-end chapter 1 passage, birds-eye's page 11 at 66.5% with the same final artwork,
and the mixed-layout EPUB's third section. All reported Ready. This verifies
persisted locations across host process replacement after a normal background save;
it does not establish recovery from an abrupt termination before a save.

The EPUB spine's declared page progression now maps physical chapter gestures:
right swipe advances RTL books, while LTR/default retain left-swipe advancement.
Logical spine order and `moveChapter(by:)` are unchanged. The unchanged W3C
`pkg-spine-progression_rtl` fixture (revision
`54092b4233253e9aac80e93ec4782b380b4b3403`, W3C Software and Document License)
was packaged locally for manual use. On iPhone 17, right swipe moved content 001
to 002, and left swipe returned to 001. Turning off Reader swipe navigation kept
001 unchanged after a right swipe. After restoring swipes, the default-direction
mixed-layout sample moved 003 → 002 with a right swipe and 002 → 003 with a left
swipe. All destinations reported Ready. This checks chapter navigation, not
synthetic page spreads or CSS writing-mode conformance.

The independent consumer compiled for both simulator architectures, and the
personal-team signed Release device build passed. Evidence logs:
`/tmp/chek-rtl-package-build.log` and `/tmp/chek-rtl-device-build.log`.

## Release checks still open

### Physical lifecycle and memory follow-up

An Activity Monitor capture on the signed Release demo lasted 272.68 seconds.
The session counter advanced from 3 to 19: three mixed-layout EPUB replacements,
two comic opens with three replacements each, and one Moby-Dick open with four
replacements. The pass included fast comic scrolling, mixed-layout navigation,
and four explicit closes. Observed settled readers reported Ready and retained
their saved locations. The final reader remained closed for the idle tail.

The host process's sampled physical footprint started at 25.72 MiB, peaked at
66.92 MiB during the comic phase, and ended at 27.69 MiB. Samples from seconds
210–268 ranged from 27.69–27.75 MiB. These results show release after this bounded
soak, not a leak-free guarantee or a worst-case limit for arbitrary publications.
The capture includes WebKit processes, but their responsible-process field reports
launchd, so host footprint must not be presented as total EPUB memory. WebContent
processes appearing during the EPUB phases peaked individually between 41.42 and
76.39 MiB; ownership is inferred from timing, not established by that field. No
WebContent or WebKit GPU process remained in the subsequent process inventory;
the WebKit networking process remained. Export emitted a dylib-overlap warning
but completed with the memory tables available.

Evidence: `/tmp/chek-reader-lifecycle-memory.trace`,
`/tmp/chek-reader-memory-live.xml`, and `/tmp/chek-reader-memory-summary.json`.

The earlier 26.06-second Time Profiler capture was also reviewed. It contained no
potential-hang rows (250 ms threshold). Sampled running-thread weights were 997 ms
for the host, 557 ms for WebContent, and 94 ms for WebKit GPU. Symbols were largely
raw addresses; no actionable package CPU hotspot was established. This older
capture had no precise gesture markers and does not measure frame rate or input
latency. Evidence: `/tmp/chek-fast-scroll-analysis.json` and its XML exports.

Accessibility review found missing package-level navigation equivalents. Paged
CBZ now handles accessibility page-scroll requests and announces the decoded
destination; EPUB exposes logical Previous chapter and Next chapter custom
actions while preserving WebKit child elements. Both honor navigation opt-out and
boundaries. The independent consumer and signed device build pass; physical
VoiceOver speech, focus and action discovery are being checked separately.

### Remaining qualification

Physical-device setup: the standalone Release demo was signed with the personal
individual team `KZX5HZ32P9`, installed on iPhone 17 / iOS 27.0.1, and visibly
rendered in Device Hub. Moby-Dick, birds-eye, and the mixed-layout sample were
copied into its own Documents container. The focused checks above are not a
complete device performance or reliability qualification. Chek uses the requested
`run.chibi.manga` identifier; the standalone
consumer remains `dev.chek.reader.embedding`.

- Repeatable device frame-time, loading latency and peak-memory measurements,
  including WebKit processes. The reported fast-scroll lag was traced by the user
  to streaming; direct-device scrolling was qualitatively satisfactory.
- Rapid cancellation/retry exercises beyond the verified reader replacements.
- Larger legal EPUB/CBZ corpus, malformed input and repeated open/close soak runs.
- VoiceOver speech/gestures and abrupt-termination lifecycle checks; maximum Dynamic Type was
  checked for the shared failure surface and consumer controls, not every UI path.
- The v1 format-support contract is now explicit in the embedding guide and package
  README. EPUB roll, spreads, DRM and new codecs remain outside current coverage.

The package is internally installable and hardened, not certified feature-complete
or ready for a public release.
