FROM sphinxdoc/sphinx

WORKDIR /docs
ADD docs/requirements.txt /docs
RUN python3 -m pip install -r requirements.txt
RUN pip freeze

# Copy provider_models.yaml from the repo for documentation
COPY crates/hermesllm/src/bin/provider_models.yaml /docs/provider_models.yaml
