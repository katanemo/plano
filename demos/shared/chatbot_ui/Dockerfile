FROM python:3.14 AS base

FROM base AS builder

WORKDIR /src

COPY requirements.txt /src/

RUN pip install --prefix=/runtime --force-reinstall -r requirements.txt

FROM python:3.14-slim AS output

COPY --from=builder /runtime /usr/local

WORKDIR /app
COPY *.py .

CMD ["python", "run_stream.py"]
