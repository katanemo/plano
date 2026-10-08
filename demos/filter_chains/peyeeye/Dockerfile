FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_NO_CACHE_DIR=1

WORKDIR /app

RUN pip install --no-cache-dir uv

COPY pyproject.toml ./
RUN uv pip install --system --no-cache-dir \
        "fastapi>=0.104.1" \
        "uvicorn>=0.24.0" \
        "httpx>=0.27.0" \
        "pydantic>=2.0.0"

COPY peyeeye.py ./

EXPOSE 10502

CMD ["uvicorn", "peyeeye:app", "--host", "0.0.0.0", "--port", "10502"]
