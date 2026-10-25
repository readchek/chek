FROM alpine:3.22
# ImageMagick, the font and zip bind the picture books; see library.list.
RUN apk add --no-cache curl jq tar zip imagemagick imagemagick-jpeg font-dejavu
COPY seed.sh corpus.parts library.list /seed/
ENTRYPOINT ["/bin/sh", "/seed/seed.sh"]
