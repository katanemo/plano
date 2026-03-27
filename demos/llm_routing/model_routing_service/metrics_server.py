"""
Minimal Prometheus metrics server for demo purposes.
Exposes mock P95 latency data for model routing.
"""
from http.server import HTTPServer, BaseHTTPRequestHandler

METRICS = """\
# HELP model_latency_p95_seconds P95 request latency in seconds per model
# TYPE model_latency_p95_seconds gauge
model_latency_p95_seconds{model_name="anthropic/claude-sonnet-4-20250514"} 0.85
model_latency_p95_seconds{model_name="openai/gpt-4o"} 1.20
model_latency_p95_seconds{model_name="openai/gpt-4o-mini"} 0.40
""".encode()


class MetricsHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        self.end_headers()
        self.wfile.write(METRICS)

    def log_message(self, fmt, *args):
        pass  # suppress access logs


if __name__ == "__main__":
    server = HTTPServer(("", 8080), MetricsHandler)
    print("metrics server listening on :8080", flush=True)
    server.serve_forever()
