# --- Dependency cache ---
FROM rust:1.93.0 AS deps
RUN rustup -v target add wasm32-wasip1
WORKDIR /arch

COPY crates/Cargo.toml crates/Cargo.lock ./
COPY crates/common/Cargo.toml         common/Cargo.toml
COPY crates/hermesllm/Cargo.toml      hermesllm/Cargo.toml
COPY crates/prompt_gateway/Cargo.toml prompt_gateway/Cargo.toml
COPY crates/llm_gateway/Cargo.toml    llm_gateway/Cargo.toml
COPY crates/brightstaff/Cargo.toml    brightstaff/Cargo.toml

# Dummy sources to pre-compile dependencies
RUN mkdir -p common/src && echo "" > common/src/lib.rs && \
    mkdir -p hermesllm/src && echo "" > hermesllm/src/lib.rs && \
    mkdir -p hermesllm/src/bin && echo "fn main() {}" > hermesllm/src/bin/fetch_models.rs && \
    mkdir -p prompt_gateway/src && echo "#[no_mangle] pub fn _start() {}" > prompt_gateway/src/lib.rs && \
    mkdir -p llm_gateway/src && echo "#[no_mangle] pub fn _start() {}" > llm_gateway/src/lib.rs && \
    mkdir -p brightstaff/src && echo "fn main() {}" > brightstaff/src/main.rs && echo "" > brightstaff/src/lib.rs

RUN cargo build --release --target wasm32-wasip1 -p prompt_gateway -p llm_gateway || true
RUN cargo build --release -p brightstaff || true

# --- WASM plugins ---
FROM deps AS wasm-builder
RUN rm -rf common/src hermesllm/src prompt_gateway/src llm_gateway/src
COPY crates/common/src         common/src
COPY crates/hermesllm/src      hermesllm/src
COPY crates/prompt_gateway/src prompt_gateway/src
COPY crates/llm_gateway/src    llm_gateway/src
RUN find common hermesllm prompt_gateway llm_gateway -name "*.rs" -exec touch {} +
RUN cargo build --release --target wasm32-wasip1 -p prompt_gateway -p llm_gateway

# --- Brightstaff binary ---
FROM deps AS brightstaff-builder
RUN rm -rf common/src hermesllm/src brightstaff/src
COPY crates/common/src         common/src
COPY crates/hermesllm/src      hermesllm/src
COPY crates/brightstaff/src    brightstaff/src
RUN find common hermesllm brightstaff -name "*.rs" -exec touch {} +
RUN cargo build --release -p brightstaff

FROM docker.io/envoyproxy/envoy:v1.37.0 AS envoy

FROM python:3.13.6-slim AS arch

RUN set -eux; \
  apt-get update; \
  apt-get install -y --no-install-recommends supervisor gettext-base curl; \
  apt-get clean; rm -rf /var/lib/apt/lists/*

# Remove PAM packages (CVE-2025-6020)
RUN set -eux; \
  dpkg -r --force-depends libpam-modules libpam-modules-bin libpam-runtime libpam0g || true; \
  dpkg -P --force-all libpam-modules libpam-modules-bin libpam-runtime libpam0g || true; \
  rm -rf /etc/pam.d /lib/*/security /usr/lib/security || true

COPY --from=envoy /usr/local/bin/envoy /usr/local/bin/envoy

WORKDIR /app

RUN pip install --no-cache-dir uv

COPY cli/pyproject.toml ./
COPY cli/uv.lock ./
COPY cli/README.md ./

RUN uv run pip install --no-cache-dir .

COPY cli/planoai planoai/
COPY config/envoy.template.yaml .
COPY config/arch_config_schema.yaml .
COPY config/supervisord.conf /etc/supervisor/conf.d/supervisord.conf

COPY --from=wasm-builder /arch/target/wasm32-wasip1/release/prompt_gateway.wasm /etc/envoy/proxy-wasm-plugins/prompt_gateway.wasm
COPY --from=wasm-builder /arch/target/wasm32-wasip1/release/llm_gateway.wasm /etc/envoy/proxy-wasm-plugins/llm_gateway.wasm
COPY --from=brightstaff-builder /arch/target/release/brightstaff /app/brightstaff

RUN mkdir -p /var/log/supervisor && \
    touch /var/log/envoy.log /var/log/supervisor/supervisord.log \
          /var/log/access_ingress.log /var/log/access_ingress_prompt.log \
          /var/log/access_internal.log /var/log/access_llm.log /var/log/access_agent.log

ENTRYPOINT ["/usr/bin/supervisord"]
