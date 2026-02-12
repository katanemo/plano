.. _plano_access_logging:

Access Logging
==============

Access logging in Plano refers to the logging of detailed information about each request and response that flows through Plano.
It provides visibility into the traffic passing through Plano, which is crucial for monitoring, debugging, and analyzing the
behavior of AI applications and their interactions.

Key Features
^^^^^^^^^^^^
* **Per-Request Logging**:
  Each request that passes through Plano is logged. This includes important metadata such as HTTP method,
  path, response status code, request duration, upstream host, and more.
* **Integration with Monitoring Tools**:
  Access logs can be exported to centralized logging systems (e.g., ELK stack or Fluentd) or used to feed monitoring and alerting systems.
* **Structured Logging**: where each request is logged as a object, making it easier to parse and analyze using tools like Elasticsearch and Kibana.

How It Works
^^^^^^^^^^^^

Plano exposes access logs for every call it manages on your behalf. By default these access logs can be found under ``~/plano_logs``. For example:

.. code-block:: console

  $ tail -F ~/plano_logs/access_*.log

  ==> /Users/username/plano_logs/access_llm.log <==
  [2024-10-10T03:55:49.537Z] "POST /v1/chat/completions HTTP/1.1" 0 DC 0 0 770 - "-" "OpenAI/Python 1.51.0" "469793af-b25f-9b57-b265-f376e8d8c586" "api.openai.com" "162.159.140.245:443"

  ==> /Users/username/plano_logs/access_internal.log <==
  [2024-10-10T03:56:03.906Z] "POST /embeddings HTTP/1.1" 200 - 52 21797 54 53 "-" "-" "604197fe-2a5b-95a2-9367-1d6b30cfc845" "model_server" "192.168.65.254:51000"
  [2024-10-10T03:56:03.961Z] "POST /zeroshot HTTP/1.1" 200 - 106 218 87 87 "-" "-" "604197fe-2a5b-95a2-9367-1d6b30cfc845" "model_server" "192.168.65.254:51000"
  [2024-10-10T03:56:04.050Z] "POST /v1/chat/completions HTTP/1.1" 200 - 1301 614 441 441 "-" "-" "604197fe-2a5b-95a2-9367-1d6b30cfc845" "model_server" "192.168.65.254:51000"
  [2024-10-10T03:56:04.492Z] "POST /hallucination HTTP/1.1" 200 - 556 127 104 104 "-" "-" "604197fe-2a5b-95a2-9367-1d6b30cfc845" "model_server" "192.168.65.254:51000"
  [2024-10-10T03:56:04.598Z] "POST /insurance_claim_details HTTP/1.1" 200 - 447 125 17 17 "-" "-" "604197fe-2a5b-95a2-9367-1d6b30cfc845" "api_server" "192.168.65.254:18083"

  ==> /Users/username/plano_logs/access_ingress.log <==
  [2024-10-10T03:56:03.905Z] "POST /v1/chat/completions HTTP/1.1" 200 - 463 1022 1695 984 "-" "OpenAI/Python 1.51.0" "604197fe-2a5b-95a2-9367-1d6b30cfc845" "plano_llm_listener" "0.0.0.0:12000"


Log Format
^^^^^^^^^^
What do these logs mean? Let's break down the log format:

.. code-block:: console

  START_TIME METHOD ORIGINAL-PATH PROTOCOL RESPONSE_CODE RESPONSE_FLAGS
  BYTES_RECEIVED BYTES_SENT DURATION UPSTREAM-SERVICE-TIME X-FORWARDED-FOR
  USER-AGENT X-REQUEST-ID  AUTHORITY UPSTREAM_HOST

Most of these fields are self-explanatory, but here are a few key fields to note:

- UPSTREAM-SERVICE-TIME: The time taken by the upstream service to process the request.
- DURATION: The total time taken to process the request.

For example for following request:

.. code-block:: console

  [2024-10-10T03:56:03.905Z] "POST /v1/chat/completions HTTP/1.1" 200 - 463 1022 1695 984 "-" "OpenAI/Python 1.51.0" "604197fe-2a5b-95a2-9367-1d6b30cfc845" "plano_llm_listener" "0.0.0.0:12000"

Total duration was 1695ms, and the upstream service took 984ms to process the request. Bytes received and sent were 463 and 1022 respectively.
