#!/bin/bash

echo "🏦 Credit Risk Case Copilot - Quick Start"
echo "=========================================="
echo ""

# Check if OPENAI_API_KEY is set
if [ -z "$OPENAI_API_KEY" ]; then
    echo "❌ Error: OPENAI_API_KEY environment variable is not set"
    echo ""
    echo "Please set your OpenAI API key:"
    echo "  export OPENAI_API_KEY='your-key-here'"
    echo ""
    echo "Or create a .env file:"
    echo "  cp .env.example .env"
    echo "  # Edit .env and add your key"
    exit 1
fi

echo "✅ OpenAI API key detected"
echo ""

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "❌ Error: Docker is not running"
    echo "Please start Docker and try again"
    exit 1
fi

echo "✅ Docker is running"
echo ""

# Start Docker services
echo "🚀 Starting Docker services..."
echo "   - Risk Crew Agent (10530)"
echo "   - Case Service (10540)"
echo "   - PII Filter (10550)"
echo "   - Streamlit UI (8501)"
echo "   - Jaeger (16686)"
echo ""

docker compose up -d --build

# Wait for services to be ready
echo ""
echo "⏳ Waiting for services to start..."
sleep 5

# Check service health
echo ""
echo "🔍 Checking service health..."

check_service() {
    local name=$1
    local url=$2
    
    if curl -s "$url" > /dev/null 2>&1; then
        echo "   ✅ $name is healthy"
        return 0
    else
        echo "   ❌ $name is not responding"
        return 1
    fi
}

check_service "Risk Crew Agent" "http://localhost:10530/health"
check_service "Case Service" "http://localhost:10540/health"
check_service "PII Filter" "http://localhost:10550/health"

echo ""
echo "=========================================="
echo "📋 Next Steps:"
echo "=========================================="
echo ""
echo "1. Start Plano orchestrator (in a new terminal):"
echo "   cd $(pwd)"
echo "   planoai up config.yaml"
echo ""
echo "   Or with uv:"
echo "   uvx planoai up config.yaml"
echo ""
echo "2. Access the applications:"
echo "   📊 Streamlit UI:  http://localhost:8501"
echo "   🔍 Jaeger Traces: http://localhost:16686"
echo ""
echo "3. View logs:"
echo "   docker compose logs -f"
echo ""
echo "4. Stop services:"
echo "   docker compose down"
echo ""
echo "=========================================="
