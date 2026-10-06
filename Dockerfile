# syntax=docker/dockerfile:1

# ---- 1. Frontend (Svelte + Vite) -------------------------------------------
FROM node:22-bookworm-slim AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

# ---- 2. Backend (Rust + axum) ----------------------------------------------
FROM rust:1-bookworm AS backend
WORKDIR /app/backend
COPY backend/ ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/backend/target \
    cargo build --release --locked --bin dji-log-viewer \
    && cp target/release/dji-log-viewer /usr/local/bin/dji-log-viewer

# ---- 3. Runtime ------------------------------------------------------------
FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin app \
    && mkdir -p /data \
    && chown app:app /data

# TLS roots for calls to the DJI API (taken from the builder, no apt needed).
COPY --from=backend /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=backend /usr/local/bin/dji-log-viewer /usr/local/bin/dji-log-viewer
COPY --from=frontend /app/frontend/dist /app/static

ENV PORT=8080 \
    DATA_DIR=/data \
    STATIC_DIR=/app/static \
    RUST_LOG=info

USER app
WORKDIR /app
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["dji-log-viewer", "healthcheck"]
CMD ["dji-log-viewer"]
