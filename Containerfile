FROM docker.io/library/rust:1.89-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
RUN cargo build --release --locked

FROM build AS test
COPY schema.sql lottery.sqlite3 ./
COPY tests/ ./tests/
COPY data/raw/lotto/all.json ./data/raw/lotto/
COPY data/raw/pension720/prizes-1-342.json ./data/raw/pension720/
CMD ["cargo", "test", "--release", "--locked"]

FROM docker.io/library/debian:bookworm-slim
RUN install -d -m 0700 -o 10001 -g 10001 /var/lib/lottery
COPY --from=build /build/target/release/lottery-daemon /usr/local/bin/lottery-daemon
COPY lottery.sqlite3 /opt/lottery/seed.sqlite3
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/lottery-daemon"]
CMD ["run"]
