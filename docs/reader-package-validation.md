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
evicted cache. Memory pressure cancels offscreen prefetch and retains intersecting
pages within the existing five-view and 64 MiB display limits. The decode cache
has a separate 64 MiB bound. These are not total process memory limits. More than
five short images visible together can still show placeholders; very large
images can also exceed the display budget. Prefetch resumes on canvas remount.

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
through a dedicated persistent-host UI in this pass.

## Release checks still open

- Repeatable device frame-time, loading latency and peak-memory measurements,
  including WebKit processes. Investigate the reported scroll lag end to end.
- Persistent-host reader replacement and rapid cancellation/retry exercises.
- Larger legal EPUB/CBZ corpus, malformed input and repeated open/close soak runs.
- VoiceOver speech/gestures, maximum Dynamic Type and device lifecycle checks.
- Define the v1 format-support contract against the unsupported features in the
  package README. This work does not add EPUB roll, spreads, DRM or new codecs.

The package is internally installable and hardened, not certified feature-complete
or ready for a public release.
