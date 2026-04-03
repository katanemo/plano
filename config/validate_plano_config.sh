#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLI_DIR="$REPO_ROOT/cli"

# Use uv run if available and cli/ has a pyproject.toml, otherwise fall back to bare python
if command -v uv &> /dev/null && [ -f "$CLI_DIR/pyproject.toml" ]; then
  PYTHON_CMD="uv run --directory $CLI_DIR python"
else
  PYTHON_CMD="python"
fi

failed_files=()

for file in $(find . -name config.yaml -o -name plano_config_full_reference.yaml); do
  echo "Validating ${file}..."
  rendered_file="$(pwd)/${file}_rendered"
  touch "$rendered_file"

  PLANO_CONFIG_FILE="$(pwd)/${file}" \
  PLANO_CONFIG_SCHEMA_FILE="${SCRIPT_DIR}/plano_config_schema.yaml" \
  TEMPLATE_ROOT="${SCRIPT_DIR}" \
  ENVOY_CONFIG_TEMPLATE_FILE="envoy.template.yaml" \
  PLANO_CONFIG_FILE_RENDERED="$rendered_file" \
  ENVOY_CONFIG_FILE_RENDERED="/dev/null" \
  $PYTHON_CMD -m planoai.config_generator 2>&1 > /dev/null

  if [ $? -ne 0 ]; then
    echo "Validation failed for $file"
    failed_files+=("$file")
  fi

  RENDERED_CHECKED_IN_FILE=$(echo $file | sed 's/\.yaml$/_rendered.yaml/')
  if [ -f "$RENDERED_CHECKED_IN_FILE" ]; then
    echo "Checking rendered file against checked-in version..."
    if ! diff -q "$rendered_file" "$RENDERED_CHECKED_IN_FILE" > /dev/null; then
      echo "Rendered file $rendered_file does not match checked-in version ${RENDERED_CHECKED_IN_FILE}"
      failed_files+=("$rendered_file")
    else
      echo "Rendered file matches checked-in version."
    fi
  fi
done

# Print summary of failed files
if [ ${#failed_files[@]} -ne 0 ]; then
  echo -e "\nValidation failed for the following files:"
  printf '%s\n' "${failed_files[@]}"
  exit 1
else
  echo -e "\nAll files validated successfully!"
fi
