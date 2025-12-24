FROM rust:1.85-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations

RUN cargo build --manifest-path crates/app/Cargo.toml --release --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -u 10001 -g root inkstone

WORKDIR /var/lib/inkstone

COPY --from=builder /app/target/release/inkstone-app /usr/local/bin/inkstone-app

ENV INKSTONE_HTTP_ADDR=0.0.0.0:8080 \
    INKSTONE_INDEX_DIR=/data/index

VOLUME ["/data"]

EXPOSE 8080

USER 10001

ENTRYPOINT ["inkstone-app"]
CMD ["--mode", "all"]
