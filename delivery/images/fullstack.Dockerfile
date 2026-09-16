ARG RUST_IMAGE
ARG KOTLIN_IMAGE
FROM ${RUST_IMAGE} AS rust
FROM ${KOTLIN_IMAGE}
USER root
RUN apt-get update && apt-get install -y --no-install-recommends clang lld pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY --from=rust /opt/cargo /opt/cargo
COPY --from=rust /opt/rustup /opt/rustup
RUN mkdir /opt/node && tar -xzf /opt/aio-kotlin-downloads/daf195adbe-node-v26.5.1-linux-x64.tar.gz --strip-components=1 -C /opt/node
ENV PATH=/opt/node/bin:/opt/cargo/bin:$PATH RUSTUP_HOME=/opt/rustup CARGO_HOME=/cache/cargo RUSTUP_TOOLCHAIN=nightly-2026-05-25
RUN apt-get update && apt-get install -y --no-install-recommends python3-venv && rm -rf /var/lib/apt/lists/*
RUN python3 -m venv /opt/zig && /opt/zig/bin/pip install --no-cache-dir ziglang==0.16.0 && ln -s "$(/opt/zig/bin/python -c 'import ziglang; print(ziglang.__path__[0]+"/zig")')" /usr/local/bin/zig
RUN CARGO_HOME=/opt/cargo cargo install --locked cargo-zigbuild --version 0.23.4
USER 65534:65534
