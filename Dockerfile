# Multi-arch image from the static musl binaries that the release already produced (no Rust build here).
# Build context = a directory containing linux/amd64/any-mcp-codemode and linux/arm64/any-mcp-codemode.
# Published by .github/workflows/docker.yml on each release.
FROM alpine:3.22 AS base
RUN apk add --no-cache ca-certificates && mkdir -p /data /config && chown 65532:65532 /data

FROM scratch
ARG TARGETOS
ARG TARGETARCH
COPY --from=base /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=base --chown=65532:65532 /data /data
COPY --from=base /config /config
COPY ${TARGETOS}/${TARGETARCH}/any-mcp-codemode /any-mcp-codemode
ENV AMC_CONFIG=/config/any-mcp-codemode.json \
    AMC_STORE=sqlite:/data/history.db?mode=rwc
VOLUME ["/data"]
WORKDIR /data
USER 65532:65532
ENTRYPOINT ["/any-mcp-codemode"]
CMD ["serve"]
