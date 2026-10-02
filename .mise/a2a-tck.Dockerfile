FROM ghcr.io/astral-sh/uv:python3.12-bookworm-slim

WORKDIR /tck
COPY . .
RUN uv sync --frozen --no-dev

ENTRYPOINT ["uv", "run", "python", "run_tck.py"]
