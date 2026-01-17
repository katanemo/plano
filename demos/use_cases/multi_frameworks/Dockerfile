FROM python:3.13-slim

WORKDIR /app

# Install bash and uv
RUN apt-get update && apt-get install -y bash && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir uv

# Copy dependency files
COPY pyproject.toml ./
COPY README.md ./

# Install dependencies to system Python
RUN uv pip install --system .

# Copy shared utilities
COPY openai_protocol.py ./

# Copy application code
COPY crewai/ ./crewai/
COPY langchain/ ./langchain/

# Set environment variables
ENV PYTHONUNBUFFERED=1
ENV PYTHONPATH=/app:$PYTHONPATH

# Default command (will be overridden in docker-compose)
CMD ["uv", "run", "python", "crewai/flight_agent.py"]
