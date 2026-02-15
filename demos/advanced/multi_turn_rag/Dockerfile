FROM python:3.12 AS base

FROM base AS builder

WORKDIR /src

COPY requirements.txt /src/
RUN pip install --prefix=/runtime --force-reinstall -r requirements.txt

COPY . /src

FROM python:3.12-slim AS output

COPY --from=builder /runtime /usr/local

COPY . /app
WORKDIR /app

CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "80", "--log-level", "info"]
