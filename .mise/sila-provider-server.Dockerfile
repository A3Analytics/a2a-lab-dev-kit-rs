FROM rust:1.98-bookworm
WORKDIR /src
COPY . .
RUN cargo build --example sila_interface --features sila2
ENTRYPOINT ["cargo", "run", "--offline", "--example", "sila_interface", "--features", "sila2"]
