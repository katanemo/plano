# Function calling

This demo shows how you can use Plano's core function calling capabilities.

# Starting the demo

1. Please make sure the [pre-requisites](https://github.com/katanemo/arch/?tab=readme-ov-file#prerequisites) are installed correctly
2. Start Plano

3. ```sh
   sh run_demo.sh
   ```
4. Navigate to http://localhost:3001/
5. You can type in queries like "how is the weather?"

Here is a sample interaction,
<img width="575" alt="image" src="https://github.com/user-attachments/assets/e0929490-3eb2-4130-ae87-a732aea4d059">

## Tracing

To see a tracing dashboard, navigate to http://localhost:16686/ to open Jaeger UI.

### Stopping Demo

1. To end the demo, run the following command:
   ```sh
   sh run_demo.sh down
   ```
