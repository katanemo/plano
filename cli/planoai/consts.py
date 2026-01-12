import os

# Brand color - Plano purple
PLANO_COLOR = "#969FF4"

SERVICE_NAME_ARCHGW = "plano"
PLANO_DOCKER_NAME = "plano"
PLANO_DOCKER_IMAGE = os.getenv("PLANO_DOCKER_IMAGE", "katanemo/plano:0.4.2")
