FROM rust:1.98-bookworm
WORKDIR /src
COPY . .
RUN cargo build --example sila2_server --features sila2
ENTRYPOINT ["cargo", "run", "--offline", "--example", "sila2_server", "--features", "sila2"]
