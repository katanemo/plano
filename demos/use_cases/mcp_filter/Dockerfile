FROM python:3.14-slim

WORKDIR /app

# Install bash and uv
RUN apt-get update && apt-get install -y bash && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir uv

# Copy dependency files
COPY pyproject.toml README.md ./

# Copy source code
COPY src/ ./src/
COPY start_agents.sh ./

# Install dependencies using uv
RUN uv pip install --system --no-cache click fastmcp pydantic fastapi uvicorn openai

# Make start script executable
RUN chmod +x start_agents.sh

# Expose ports for all agents
EXPOSE 10500 10501 10502 10505

# Run the start script with bash
CMD ["bash", "./start_agents.sh"]
