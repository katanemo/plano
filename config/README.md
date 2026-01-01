# Envoy filter code for gateway

## Add toolchain

```sh
$ rustup target add wasm32-wasip1
```

## Building

```sh
$ cargo build --target wasm32-wasip1 --release
```

## Testing
```sh
$ cargo test
```

## Local development
- Build docker image for arch gateway. Note this needs to be built once.
  ```
  $ sh build_filter_image.sh
  ```

- Build filter binary,
  ```
  $ cargo build --target wasm32-wasip1 --release
  ```
- Start envoy with arch_config.yaml and test,
  ```
  $ docker compose -f docker-compose.dev.yaml up archgw
  ```
- dev version of docker-compose file uses following files that are mounted inside the container. That means no docker rebuild is needed if any of these files change. Just restart the container and chagne will be picked up,
  - envoy.template.yaml
  - intelligent_prompt_gateway.wasm
