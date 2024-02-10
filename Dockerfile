FROM rust:1.75-slim-buster as builder

ARG TARGET=x86_64-unknown-linux-gnu

RUN apt-get update  \
    && apt-get upgrade -y  \
    && apt-get install protobuf-compiler ca-certificates build-essential -y  \
    && rustup target add ${TARGET}

WORKDIR build

COPY Cargo.toml Cargo.lock ./
COPY src src/
COPY proto proto/
COPY build.rs ./
COPY vendor vendor/
COPY .cargo/config.toml .cargo/config.toml

RUN cargo build --release --bin backup-server --offline --target ${TARGET} --jobs $(nproc) -vv

FROM ubuntu:22.04

ARG TARGET=x86_64-unknown-linux-gnu
ENV RUST_BACKTRACE full
ENV DEBIAN_FRONTEND noninteractive
ENV TZ Etc/UTC

LABEL maintainer="Kirill <k@kunansy.ru>"

WORKDIR app
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /build/target/${TARGET}/release/backup-server ./
