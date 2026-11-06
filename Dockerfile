FROM rust:1.99.1-slim-bookworm AS builder

ARG TARGET=x86_64-unknown-linux-gnu

RUN apt-get update  \
    && apt-get upgrade -y  \
    && apt-get install protobuf-compiler ca-certificates build-essential -y  \
    && rustup target add ${TARGET}

# Production host is an Intel Ice Lake VM: AVX-512 is exposed, SHA-NI is not.
# Applies only to the x86_64 target, not to build scripts or the aarch64 image.
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C target-cpu=x86-64-v4"

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates crates/
COPY vendor vendor/
COPY .cargo/config.toml .cargo/config.toml

RUN cargo build --release --bin backup-server --offline --target ${TARGET} --jobs $(nproc)

FROM debian:bookworm-slim

ARG TARGET=x86_64-unknown-linux-gnu
ENV RUST_BACKTRACE=full
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=Etc/UTC

LABEL maintainer="Kirill <k@kunansy.ru>"

WORKDIR /app
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /build/target/${TARGET}/release/backup-server ./
