FROM rust:1.99.1-slim-bookworm AS builder

ARG TARGET=x86_64-unknown-linux-gnu

RUN apt-get update  \
    && apt-get upgrade -y  \
    && apt-get install protobuf-compiler libprotobuf-dev ca-certificates build-essential -y  \
    && rustup target add ${TARGET}

# Production host is an Intel Ice Lake VM: AVX-512 is exposed, SHA-NI is not.
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C target-cpu=x86-64-v4"

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates crates/
COPY vendor vendor/
COPY .cargo/config.toml .cargo/config.toml

RUN cargo build --release --bin backup-server --offline --target ${TARGET} --jobs $(nproc)

FROM debian:bookworm-slim

ARG TARGET=x86_64-unknown-linux-gnu
# Client major version must be >= the server's one.
ARG PG_MAJOR=18
ENV RUST_BACKTRACE=full
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=Etc/UTC

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && install -d /usr/share/postgresql-common/pgdg \
    && curl -fsSL -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc https://www.postgresql.org/media/keys/ACCC4CF8.asc \
    && echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] https://apt.postgresql.org/pub/repos/apt bookworm-pgdg main" > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends postgresql-client-${PG_MAJOR} \
    && apt-get purge -y curl && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

LABEL maintainer="Kirill <k@kunansy.ru>"

RUN groupadd --gid 10001 app \
    && useradd --uid 10001 --gid app --no-create-home --shell /usr/sbin/nologin app

WORKDIR /app
RUN mkdir data && chown app:app data
COPY --from=builder /build/target/${TARGET}/release/backup-server ./

USER 10001:10001
