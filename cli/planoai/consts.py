import os

# Brand color - Plano purple
PLANO_COLOR = "#969FF4"

SERVICE_NAME_ARCHGW = "plano"
PLANO_DOCKER_NAME = "plano"
PLANO_DOCKER_IMAGE = os.getenv("PLANO_DOCKER_IMAGE", "katanemo/plano:0.4.8")
DEFAULT_OTEL_TRACING_GRPC_ENDPOINT = "http://host.docker.internal:4317"

# Native mode constants
PLANO_HOME = os.path.join(os.path.expanduser("~"), ".plano")
PLANO_RUN_DIR = os.path.join(PLANO_HOME, "run")
PLANO_BIN_DIR = os.path.join(PLANO_HOME, "bin")
PLANO_PLUGINS_DIR = os.path.join(PLANO_HOME, "plugins")
ENVOY_VERSION = "v1.37.0"  # keep in sync with Dockerfile ARG ENVOY_VERSION
NATIVE_PID_FILE = os.path.join(PLANO_RUN_DIR, "plano.pid")
DEFAULT_NATIVE_OTEL_TRACING_GRPC_ENDPOINT = "http://localhost:4317"
