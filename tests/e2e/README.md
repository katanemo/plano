# e2e tests

e2e tests for the Plano LLM gateway (model listener) and related API translation.

To be able to run e2e tests successfully `run_e2e_tests.sh` prepares the environment as follows:

1. build, install and start the Plano CLI
1. build and start Plano gateway (using docker)
1. start e2e tests (using uv)
   1. runs LLM gateway API translation tests (OpenAI / Anthropic clients)
   2. runs model alias routing tests
   3. runs OpenAI responses API client tests
2. cleanup
   1. stops Plano gateway

## How to run

To run locally make sure that following requirements are met.

### Requirements

- Python 3.10+
- uv
- Docker

### Running tests locally

```bash
cd tests/e2e
./run_e2e_tests.sh
```
