# syntax=docker/dockerfile:1

FROM ubuntu:24.04 AS builder

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:$PATH

RUN apt-get update -qq \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        libgtk-4-dev \
        libwebkitgtk-6.0-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain 1.85.1

WORKDIR /src
COPY . .
RUN cargo build --release --locked --workspace

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    WEBKIT_DISABLE_DMABUF_RENDERER=1 \
    HOME=/home/hwatu \
    XDG_RUNTIME_DIR=/run/user/10001

RUN apt-get update -qq \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        cage \
        dbus-daemon \
        libgtk-4-1 \
        libwebkitgtk-6.0-4 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /usr/sbin/nologin hwatu \
    && mkdir -p /run/user/10001 \
    && chown hwatu:hwatu /run/user/10001

COPY --from=builder /src/target/release/hwatu /usr/local/bin/hwatu
COPY --from=builder /src/target/release/hwatud /usr/local/bin/hwatud
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh

USER hwatu
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
