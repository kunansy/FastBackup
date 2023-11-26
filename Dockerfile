FROM rust:1.74-slim-buster as builder

RUN apt-get update  \
    && apt-get upgrade -y  \
    && apt-get install protobuf-compiler ca-certificates build-essential -y

WORKDIR build

COPY Cargo.toml Cargo.lock ./
COPY src src/
COPY proto proto/
COPY build.rs ./
COPY vendor vendor/
COPY .cargo/config.toml .cargo/config.toml

RUN cargo build --release --bin backup-server --offline --jobs $(nproc) -vv

FROM ubuntu:22.04

ENV RUST_BACKTRACE full
ENV DEBIAN_FRONTEND noninteractive
ENV TZ Etc/UTC

LABEL maintainer="Kirill <k@kunansy.ru>"

RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get autoremove -y \
    && apt-get clean && apt-get autoclean \
    && rm -rf /var/lib/apt/lists/*

WORKDIR app
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /build/target/release/backup-server ./
