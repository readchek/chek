# Chek product direction

Clarified on 2026-10-08. This document records product intent and the predecessor's
behavior as described by the project owner. It is not a feature inventory of the
code in this repository.

## What Chek is

Chek is a ground-up redesign and direct successor to Chibi Manga. It is a versatile
e-reader for your books and comics, wherever you keep them. Manga and manhwa are
central to its identity, alongside e-books and other comics.

The goal is a clean, reliable, tension-free reading experience. Users choose their
own library and provider; they do not need to adopt Chek's server to use its clients.

## Clients

The new clients are planned to support both:

- Local files in a wide range of reading formats.
- Libraries served by Komga, Kavita, Chek's own server, and potentially other providers.

The first new app is iOS. More platforms are planned. The exact file-format list,
provider support at launch, and delivery sequence are not finalized.

Keep the reading experience clear and familiar across sources. Where a provider
has extra features, clients should expose them only when supported. The capability
contract for the new system remains a design decision.

## Chek server

Chek's backend is planned to be open-source and serve as a library server in its
own right, in the same category as Komga and Kavita. It is not just a translation
layer for another service.

An extension or import mechanism is planned, but its contracts, supported sources,
and packaging are not decided. Personal integrations can remain in separate private
repositories; they are not part of the public product promise or required to use
Chek.

## What exists in Chibi Manga today

The live predecessor has reader clients that connect to Komga and Kavita. Its
custom backend translates Suwayomi data into Komga-shaped responses for the app.
Additional features are advertised through API capabilities, which clients check
before enabling the corresponding UI or behavior.

Chek keeps the core idea of versatile reader clients, while broadening local-file
reading and making its own server a standalone product. The predecessor's
translation architecture should not be described as the new backend architecture.

## Public message

Lead with the reader and the freedom to choose a source:

> Your books and comics. Your choice of source.

Explain local files and server libraries early. Name Komga, Kavita, and Chek's own
server as planned choices. Manga and manhwa can provide the visual personality,
but the copy should also make e-book reading clear.

Introduce the project plainly, rather than selling a service. Keep the tone
personal, useful, and inviting. Explain what the reader does and what is being
built. Avoid sales funnels, inflated benefit claims, and SaaS-style pitches.

Call the new clients and server work in development. Avoid presenting concept
screens as app screenshots or claiming an undecided file-format list. Historical
translator details and personal integration plans are not landing-page messaging.
