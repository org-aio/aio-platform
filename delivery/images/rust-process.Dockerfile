# 为 Dioxus + 原生 Rust process 构建复用已有固定版本工具。
ARG RUST_IMAGE
ARG FULLSTACK_IMAGE
FROM ${FULLSTACK_IMAGE} AS tools
FROM ${RUST_IMAGE}
USER root
COPY --from=tools /opt/node /opt/node
COPY --from=tools /opt/zig/lib/python3.14/site-packages/ziglang /opt/ziglang
COPY --from=tools /opt/cargo/bin/cargo-zigbuild /opt/cargo/bin/cargo-zigbuild
RUN ln -s /opt/ziglang/zig /usr/local/bin/zig
ENV PATH=/opt/node/bin:$PATH
USER 65534:65534
