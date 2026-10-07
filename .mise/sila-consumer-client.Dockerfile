FROM rust:1.98-bookworm
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY proto proto
COPY src src
COPY examples examples
COPY sila2-conformance sila2-conformance
RUN cargo build --example sila_client --features sila2
ENTRYPOINT ["cargo", "run", "--quiet", "--offline", "--example", "sila_client", "--features", "sila2"]
