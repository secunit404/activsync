# syntax=docker/dockerfile:1.7
FROM python:3.12-slim

WORKDIR /app

COPY pyproject.toml ./
COPY src ./src

RUN --mount=type=cache,target=/root/.cache/pip \
    pip install .

VOLUME ["/config"]
EXPOSE 8381

CMD ["uvicorn", "activsync.main:app", "--host", "0.0.0.0", "--port", "8381"]
