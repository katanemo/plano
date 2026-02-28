FROM python:3.11-slim

WORKDIR /app

# Install system dependencies
RUN apt-get update && \
    apt-get install -y bash curl && \
    rm -rf /var/lib/apt/lists/*

# Install uv package manager
RUN pip install --no-cache-dir uv

# Copy dependency files
COPY pyproject.toml README.md* ./
COPY scenarios/ ./scenarios/

# Install dependencies
RUN uv sync --no-dev || uv pip install --system -e .

# Copy application code
COPY src/ ./src/

# Set environment variables
ENV PYTHONUNBUFFERED=1
ENV PYTHONPATH=/app

# Default command (overridden in docker-compose)
CMD ["uv", "run", "python", "src/credit_risk_demo/risk_crew_agent.py"]
