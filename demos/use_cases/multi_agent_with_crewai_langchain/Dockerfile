FROM python:3.13.11-slim

WORKDIR /app

# Install system dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends bash && \
    rm -rf /var/lib/apt/lists/* && \
    pip install --no-cache-dir uv

# Install Python dependencies
COPY pyproject.toml README.md ./
RUN uv pip install --system .

# Copy application code
COPY openai_protocol.py ./
COPY crewai/ ./crewai/
COPY langchain/ ./langchain/

# Runtime configuration
ENV PYTHONUNBUFFERED=1 \
    PYTHONPATH=/app

CMD ["uv", "run", "python", "crewai/flight_agent.py"]
