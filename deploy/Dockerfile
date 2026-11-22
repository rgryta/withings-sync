# Built from the repo root:
#   docker build -f deploy/Dockerfile -t withings-sync:latest .
#
# One image, two binaries: the services differ only by `command`. Both are Rust, so the
# runtime stage carries nothing but the two binaries. The Chromium harvester this replaced
# needed a 2.7 GB Playwright image; Withings's own web client is no longer in the loop.

# The latest stable Rust, matching the manifest's rust-version. Bump both together.
FROM rust:1.100-slim-trixie@sha256:6f14b2b4b278ec0d50aa1d69541c304ce0f95e0f6268487aa9924af1c2e9aebe AS rust
WORKDIR /build
RUN rustup target add x86_64-unknown-linux-musl \
    && apt-get update -qq \
    && apt-get install -y -qq musl-tools \
    && rm -rf /var/lib/apt/lists/*
# `db-migrations/` is here because `crates/core/src/db.rs` embeds the migrations with
# `include_str!`, so the build needs them even though the binary then carries its own copy.
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY src/ ./src/
COPY db-migrations/ ./db-migrations/
# `--workspace --bins` rather than `--bin`: `--bin` resolves only inside the default-run
# package, so naming `withings-ingest` that way fails.
RUN cargo build --release --workspace --bins --target x86_64-unknown-linux-musl

# Nothing but the binaries. No libc, no shell, no certificate store, and each absence is
# fine on purpose: TLS trust is bundled (`webpki-roots` in reqwest and tokio-tungstenite),
# every time is UTC, nothing writes a temporary file, and musl reads the /etc/resolv.conf
# Docker mounts in.
FROM scratch
# Root-owned and merely executable, so neither service can rewrite its own binary.
COPY --from=rust /build/target/x86_64-unknown-linux-musl/release/withings-mcp /usr/local/bin/withings-mcp
COPY --from=rust /build/target/x86_64-unknown-linux-musl/release/withings-ingest /usr/local/bin/withings-ingest
# Numeric: there is no /etc/passwd here. 65532 is distroless' `nonroot`.
USER 65532:65532
EXPOSE 8000
# GIT_SHA last, on purpose: above the COPYs it would enter their layer cache key.
ARG GIT_SHA=unknown
ENV WITHINGS_GIT_SHA=$GIT_SHA
# The ingest service overrides this with `withings-ingest`.
CMD ["/usr/local/bin/withings-mcp"]
