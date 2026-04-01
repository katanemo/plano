.. _logging:

Logging
=======

Plano supports dynamic log level changes at runtime, allowing you to increase
verbosity for debugging without restarting the service.

Setting the Log Level at Startup
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Set the ``LOG_LEVEL`` environment variable before starting Plano:

.. code-block:: bash

   LOG_LEVEL=debug planoai up config.yaml

This controls both the brightstaff service (``RUST_LOG``) and Envoy's WASM
component log level.

Changing the Log Level at Runtime
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Use the ``planoai log-level`` command to change levels on a running instance:

.. code-block:: bash

   # Set both services to debug
   planoai log-level debug

   # Set both services to info
   planoai log-level info

   # Show current log levels
   planoai log-level --show

   # For Docker-based instances
   planoai log-level debug --docker

The brightstaff service also accepts granular ``RUST_LOG``-style filters:

.. code-block:: bash

   # Debug for brightstaff crate only, info for everything else
   planoai log-level "brightstaff=debug,info"

Available log levels (from most to least verbose): ``trace``, ``debug``,
``info``, ``warn``, ``error``.

Direct API Access
~~~~~~~~~~~~~~~~~

You can also change log levels directly via HTTP:

**Brightstaff** (port 9091, or 19091 in Docker mode):

.. code-block:: bash

   # Get current level
   curl http://localhost:9091/admin/log-level

   # Set level
   curl -X PUT http://localhost:9091/admin/log-level -d "debug"

**Envoy** (port 9901, or 19901 in Docker mode):

.. code-block:: bash

   # View all logger levels
   curl http://localhost:9901/logging

   # Set all loggers to debug
   curl -X POST "http://localhost:9901/logging?level=debug"

   # Set only WASM component to debug
   curl -X POST "http://localhost:9901/logging?wasm=debug"
