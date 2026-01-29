docker build -f docs/Dockerfile . -t sphinx

# Clean build output locally
rm -rf docs/build
mkdir -p docs/build
chmod -R 777 docs/build

# Run make clean/html while keeping provider_models.yaml from the image
docker run --user $(id -u):$(id -g) --rm \
  -v $(pwd)/docs/source:/docs/source \
  -v $(pwd)/docs/Makefile:/docs/Makefile \
  -v $(pwd)/docs/build:/docs/build \
  sphinx make clean

docker run --user $(id -u):$(id -g) --rm \
  -v $(pwd)/docs/source:/docs/source \
  -v $(pwd)/docs/Makefile:/docs/Makefile \
  -v $(pwd)/docs/build:/docs/build \
  sphinx make html

chmod -R 777 docs/build/html
