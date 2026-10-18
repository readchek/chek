FROM alpine:3.22
RUN apk add --no-cache curl jq tar
COPY seed.sh corpus.parts /seed/
ENTRYPOINT ["/bin/sh", "/seed/seed.sh"]
