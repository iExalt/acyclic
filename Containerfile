# The server's image: the static `acyclic` binary and its notices on Alpine, with git for syncing and CA certificates
# for HTTPS, and nothing else: no endpoints, tokens or vendor CLIs. Built one architecture per native runner by
# .github/workflows/build.yml, from a release binary staged in dist/<arch>/ and the notices in dist/.
FROM docker.io/library/alpine:3.24.2@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6

ARG TARGETARCH

RUN apk add --no-cache git ca-certificates \
    && mkdir -p /usr/share/acyclic/app \
    && apk info -v | sort > /usr/share/acyclic/packages.txt \
    && adduser -D -u 10001 acyclic

COPY LICENSE dist/THIRD_PARTY_NOTICES.md /usr/share/acyclic/
COPY dist/${TARGETARCH}/acyclic /usr/local/bin/acyclic

LABEL org.opencontainers.image.source="https://github.com/iExalt/acyclic" \
      org.opencontainers.image.description="The acyclic tracker's server" \
      org.opencontainers.image.licenses="MIT"

USER acyclic
WORKDIR /home/acyclic
ENTRYPOINT ["acyclic"]
