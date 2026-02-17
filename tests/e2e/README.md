# e2e tests

e2e tests for arch llm gateway and prompt gateway

To be able to run e2e tests successfully run_e2e_script prepares environment in following way,

1. build and start weather_forecast demo (using docker compose)
1. build, install and start model server async (using uv)
1. build and start Plano gateway (using docker compose)
1. wait for model server to be ready
1. wait for Plano gateway to be ready
1. start e2e tests (using uv)
   1. runs llm gateway tests for llm routing
   2. runs prompt gateway tests to test function calling, parameter gathering and summarization
2. cleanup
   1. stops Plano gateway
   2. stops model server
   3. stops weather_forecast demo

## How to run

To run locally make sure that following requirements are met.

### Requirements

- Python 3.10+
- uv
- Docker

### Running tests locally

```sh
sh run_e2e_test.sh
```
