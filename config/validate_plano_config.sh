#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
failed_files=()

for file in $(find . -name config.yaml -o -name plano_config_full_reference.yaml); do
  echo "Validating ${file}..."

  planoai validate "$(pwd)/${file}" 2>&1 > /dev/null

  if [ $? -ne 0 ]; then
    echo "Validation failed for $file"
    failed_files+=("$file")
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
