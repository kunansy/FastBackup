FROM rust:1.70-slim-buster as builder

RUN apt-get update  \
    && apt-get upgrade -y  \
    && apt-get install protobuf-compiler -y

WORKDIR build

COPY Cargo.toml Cargo.lock ./
COPY src src/
COPY proto proto/
COPY build.rs ./
COPY vendor vendor/
COPY .cargo/config.toml .cargo/config.toml

RUN cargo build --release --bin backup-server --offline -vv -j $(nproc)

FROM ubuntu:22.04

ENV RUST_BACKTRACE full
ENV DEBIAN_FRONTEND noninteractive
ENV TZ Etc/UTC

LABEL maintainer="Kirill <k@kunansy.ru>"

# install Postgresql 15
RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install apt-utils wget lsb-release gnupg -y \
    && apt-key adv --recv-keys --keyserver keyserver.ubuntu.com 7FCC7D46ACCC4CF8 \
    && echo "deb http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list \
    && wget -qO- https://www.postgresql.org/media/keys/ACCC4CF8.asc | tee /etc/apt/trusted.gpg.d/pgdg.asc \
    && apt-get update \
    && apt-get install -y openssl gzip postgresql-client ca-certificates \
    && apt-get remove wget lsb-release apt-utils -y \
    && apt-get autoremove -y \
    && apt-get clean && apt-get autoclean \
    && rm -rf /var/lib/apt/lists/*

WORKDIR app
COPY --from=builder /build/target/release/backup-server ./
