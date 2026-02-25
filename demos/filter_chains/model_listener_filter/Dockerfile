FROM python:3.14-slim

WORKDIR /app

RUN pip install --no-cache-dir fastapi uvicorn pydantic

COPY content_guard.py .

EXPOSE 10500

CMD ["uvicorn", "content_guard:app", "--host", "0.0.0.0", "--port", "10500"]
