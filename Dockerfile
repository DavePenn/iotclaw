FROM rust:1.94-slim AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY skills/ skills/

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/iotclaw /usr/local/bin/iotclaw
COPY skills/ /app/skills/
COPY .env.example /app/.env

WORKDIR /app
EXPOSE 3000

CMD ["iotclaw", "--server"]
