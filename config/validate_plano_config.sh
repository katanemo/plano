#!/bin/bash

failed_files=()

for file in $(find . -name config.yaml -o -name plano_config_full_reference.yaml); do
  echo "Validating ${file}..."
  touch $(pwd)/${file}_rendered
  if ! docker run --rm -v "$(pwd)/${file}:/app/plano_config.yaml:ro" -v "$(pwd)/${file}_rendered:/app/plano_config_rendered.yaml:rw" --entrypoint /bin/sh ${PLANO_DOCKER_IMAGE:-katanemo/plano:0.4.8} -c "python -m planoai.config_generator" 2>&1 > /dev/null ; then
    echo "Validation failed for $file"
    failed_files+=("$file")
  fi
  RENDERED_CHECKED_IN_FILE=$(echo $file | sed 's/\.yaml$/_rendered.yaml/')
  if [ -f "$RENDERED_CHECKED_IN_FILE" ]; then
    echo "Checking rendered file against checked-in version..."
    if ! diff -q "${file}_rendered" "$RENDERED_CHECKED_IN_FILE" > /dev/null; then
      echo "Rendered file ${file}_rendered does not match checked-in version ${RENDERED_CHECKED_IN_FILE}"
      failed_files+=("${file}_rendered")
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
