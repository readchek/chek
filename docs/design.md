# Chek design decisions

This document records agreed choices for Chek's apps. It is a design reference;
it does not imply that the current app implements these choices.

## Starter palette

Palette selection reopened on 2026-10-06: the charcoal and blue study below did
not feel right for the app. The globally tinted Graphite, Umber, Grove, and Ink
studies were also rejected. Start with dark mode and leave room for light mode later.

Use simple neutral background and foreground separation. Do not tint the entire
interface to match the accent. Keep text, separators, and surfaces neutral;
reserve accent color for a small number of actions or selected states. Prefer
spacing and typography over filled containers. The latest neutral study is
exploratory; its palette and layout are not yet approved.

| Role | Color | Use |
| --- | --- | --- |
| Background | `#11110F` | Main content canvas |
| Accent | `#4A8CE0` | Primary action, reading progress, and selected controls |

Reference: [Emil Kowalski's charcoal and blue examples](https://x.com/emilkowalski/status/2089372767934115883).
These values were supplied and accepted by Arya on 2026-10-06, replacing the earlier
image-derived estimates. They are Chek's agreed starting point; the original
post's source design tokens have not been verified. Retain these values as a
record of the earlier study, not as the current palette decision.

Keep most of the interface neutral. Use the accent sparingly so that important
actions and states stand out. Warm grays support text and surface hierarchy;
they are not additional brand colors. Artwork retains its own colors.

Use `#A0A09C` for main text, including headings. Keep the overall feel muted and
papery, with hierarchy from size, weight, and spacing rather than bright white.
The updated study uses `#858580` for secondary text; that supporting value and
the earlier `#242424` raised control surface remain provisional.

Calculated contrast for solid colors:

- Main text against the background: 7.20:1.
- Secondary text against the background: 5.10:1.
- Blue against the background: 5.51:1.
- White against the blue: 3.43:1.
- Background-colored text against the blue: 5.51:1.

Button treatment under review: use warm gray `#D6D6CF` labels on a deeper blue
`#275B9E` fill, rather than dark labels on the accent. This pairing has 4.67:1
contrast. The main accent remains `#4A8CE0`.
Check contrast against the actual composited background when using translucent
materials.

## Platform direction

The iOS app should follow current Apple Human Interface Guidelines and use native
Liquid Glass controls where appropriate. Keep the reading canvas quiet; reserve
glass for navigation and controls, with blue applied selectively through native
tinting. Let native materials adapt to the content and accessibility settings.

Reference: [Apple's Liquid Glass design guidance](https://developer.apple.com/videos/play/wwdc2025/219/).

Keep background, accent, text, and surface roles separate in future implementations
so that light mode can supply its own values. A light palette is not decided yet.

## Typography

Accepted on 2026-10-06: use Apple's system font for the first app, which is iOS.
SF Pro provides the Latin interface typography, with native fallback for other
scripts. Use semantic system text styles and support Dynamic Type. Do not bundle
SF Pro or select it by a hardcoded font name.

Verify chapter titles, progress numbers, long series names, Japanese and Korean
text, and larger accessibility text sizes. The browser study uses the Apple
system font on Apple devices; it is not a native Dynamic Type implementation.

Reference: [Apple's font guidance](https://developer.apple.com/documentation/technologyoverviews/fonts).

Interface typography for other platforms remains open. Evaluate reflowable book
fonts separately. Manga and manhwa pages retain their embedded lettering.

The earlier layout studies were exploratory. Their layouts, shapes, and icon
choices are not approved design decisions.
