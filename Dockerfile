FROM rust:1.70-slim-buster as builder

RUN apt-get update  \
    && apt-get upgrade -y

WORKDIR build

COPY Cargo.toml Cargo.lock ./
COPY src src/
COPY vendor vendor/
COPY .cargo/config.toml .cargo/config.toml

RUN cargo build --release --bin backup-server --offline -vv -j $(nproc)

FROM ubuntu:22.04

ENV RUST_BACKTRACE full
ENV DEBIAN_FRONTEND noninteractive
ENV TZ Etc/UTC

LABEL maintainer="Kirill <k@kunansy.ru>"

RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install apt-utils wget lsb-release -y
# install Postgresql 15
RUN echo "deb http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list \
    && wget -qO- https://www.postgresql.org/media/keys/ACCC4CF8.asc | tee /etc/apt/trusted.gpg.d/pgdg.asc &>/dev/null \
    && apt-get update

RUN apt-get upgrade -y \
    && apt-get install -y openssl gzip postgresql-client ca-certificates \
    && apt-get remove wget lsb-release apt-utils -y \
    && apt-get autoremove -y \
    && apt-get clean && apt-get autoclean \
    && rm -rf /var/lib/apt/lists/*

WORKDIR app
COPY --from=builder /build/target/release/backup-server ./
