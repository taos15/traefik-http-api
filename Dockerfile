FROM rust:1.99 AS build
RUN rustup target add wasm32-unknown-unknown && cargo install wasm-bindgen-cli --version 0.2.130 --locked
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
COPY web ./web
RUN cargo build --release --no-default-features --features web --target wasm32-unknown-unknown && \
    wasm-bindgen --target web --out-dir web/dist --out-name app target/wasm32-unknown-unknown/release/traefik-web.wasm && \
    cargo build --release --no-default-features --features server --bin traefik-server

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates openssh-client && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /app/target/release/traefik-server /app/traefik-server
COPY --from=build /app/web/dist /app/web/dist
RUN mkdir -p /app/data
EXPOSE 4000
VOLUME /app/data
CMD ["/app/traefik-server"]
