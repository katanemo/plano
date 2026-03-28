"""
Demo metrics server.

Exposes two endpoints:
  GET /metrics  — Prometheus text format, P95 latency per model (scraped by Prometheus)
  GET /costs    — JSON cost data per model, compatible with cost_metrics source
"""
import json
from http.server import HTTPServer, BaseHTTPRequestHandler

PROMETHEUS_METRICS = """\
# HELP model_latency_p95_seconds P95 request latency in seconds per model
# TYPE model_latency_p95_seconds gauge
model_latency_p95_seconds{model_name="anthropic/claude-sonnet-4-20250514"} 0.85
model_latency_p95_seconds{model_name="openai/gpt-4o"} 1.20
model_latency_p95_seconds{model_name="openai/gpt-4o-mini"} 0.40
""".encode()

COST_DATA = {
    "anthropic/claude-sonnet-4-20250514": {
        "input_per_million": 3.0,
        "output_per_million": 15.0,
    },
    "openai/gpt-4o": {"input_per_million": 5.0, "output_per_million": 20.0},
    "openai/gpt-4o-mini": {"input_per_million": 0.15, "output_per_million": 0.6},
}


class MetricsHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/costs":
            body = json.dumps(COST_DATA).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(body)
        else:
            # /metrics and everything else → Prometheus format
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
            self.end_headers()
            self.wfile.write(PROMETHEUS_METRICS)

    def log_message(self, fmt, *args):
        pass  # suppress access logs


if __name__ == "__main__":
    server = HTTPServer(("", 8080), MetricsHandler)
    print("metrics server listening on :8080 (/metrics, /costs)", flush=True)
    server.serve_forever()
