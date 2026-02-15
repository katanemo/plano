# Blazing fast Python Docker builds with uv

# The builder image, used to build the virtual environment
FROM python:3.14 as builder

# Install uv
RUN pip install --no-cache-dir uv

# Set working directory
WORKDIR /code

# Copy dependency files
COPY pyproject.toml uv.lock ./
RUN touch README.md

# Install dependencies using uv
RUN uv sync --frozen --no-dev

# The runtime image, used to just run the code provided its virtual environment
FROM python:3.14-slim as runtime

RUN apt-get update && apt-get install -y curl

WORKDIR /code

ENV VIRTUAL_ENV=/code/.venv \
    PATH="/code/.venv/bin:$PATH"

COPY --from=builder ${VIRTUAL_ENV} ${VIRTUAL_ENV}

COPY main.py ./

HEALTHCHECK \
    --interval=5s \
    --timeout=1s \
    --start-period=1s \
    --retries=3 \
    CMD curl http://localhost:80/healthz

CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "80", "--log-level", "debug"]
