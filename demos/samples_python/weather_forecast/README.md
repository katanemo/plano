# Function calling

This demo shows how you can use Plano's core function calling capabilities.

# Starting the demo

1. Please make sure the [pre-requisites](https://github.com/katanemo/arch/?tab=readme-ov-file#prerequisites) are installed correctly
2. Start Plano

3. ```sh
   sh run_demo.sh
   ```
4. Navigate to http://localhost:18080/
5. You can type in queries like "how is the weather?"

# Observability

Plano gateway publishes stats endpoint at http://localhost:19901/stats. In this demo we are using prometheus to pull stats from Plano and we are using grafana to visalize the stats in dashboard. To see grafana dashboard follow instructions below,

1. Start grafana and prometheus using following command
   ```yaml
   docker compose --profile monitoring up
   ```
2. Navigate to http://localhost:3000/ to open grafana UI (use admin/grafana as credentials)
3. From grafana left nav click on dashboards and select "Intelligent Gateway Overview" to view Plano gateway stats

Here is a sample interaction,
<img width="575" alt="image" src="https://github.com/user-attachments/assets/e0929490-3eb2-4130-ae87-a732aea4d059">

## Tracing

To see a tracing dashboard follow instructions below,

1. For Jaeger, you can either use the default run_demo.sh script or run the following command:

```sh
sh run_demo.sh jaeger
```

2. For Logfire, first make sure to add a LOGFIRE_API_KEY to the .env file. You can either use the default run_demo.sh script or run the following command:

```sh
sh run_demo.sh logfire
```

3. For Signoz, you can either use the default run_demo.sh script or run the following command:

```sh
sh run_demo.sh signoz
```

If using Jaeger, navigate to http://localhost:16686/ to open Jaeger UI

If using Signoz, navigate to http://localhost:3301/ to open Signoz UI

If using Logfire, navigate to your logfire dashboard that you got the write key from to view the dashboard

### Stopping Demo

1. To end the demo, run the following command:
   ```sh
   sh run_demo.sh down
   ```
