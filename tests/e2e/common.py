import json
import os


PROMPT_GATEWAY_ENDPOINT = os.getenv(
    "PROMPT_GATEWAY_ENDPOINT", "http://localhost:10000/v1/chat/completions"
)

PROMPT_GATEWAY_PATH = os.getenv("PROMPT_GATEWAY_PATH", "/v1/chat/completions")
MODEL_SERVER_FUNC_PATH = os.getenv("MODEL_SERVER_FUNC_PATH", "/function_calling")

LLM_GATEWAY_ENDPOINT = os.getenv(
    "LLM_GATEWAY_ENDPOINT", "http://localhost:12000/v1/chat/completions"
)
ARCH_STATE_HEADER = "x-arch-state"

PREFILL_LIST = [
    "May",
    "Could",
    "Sure",
    "Definitely",
    "Certainly",
    "Of course",
    "Can",
]

TEST_CASE_FIXTURES = {
    "SIMPLE": {
        "input": {
            "messages": [
                {
                    "role": "user",
                    "content": "how is the weather in seattle for next 2 days",
                }
            ]
        },
        "model_server_response": {
            "id": 0,
            "object": "chat_completion",
            "created": "",
            "choices": [
                {
                    "id": 0,
                    "message": {
                        "role": "",
                        "content": "",
                        "tool_call_id": "",
                        "tool_calls": [
                            {
                                "id": "call_6009",
                                "type": "function",
                                "function": {
                                    "name": "get_current_weather",
                                    "arguments": {
                                        "location": "Seattle, WA",
                                        "days": "2",
                                    },
                                },
                            }
                        ],
                    },
                    "finish_reason": "stop",
                }
            ],
            "model": "Arch-Function",
            "metadata": {"intent_latency": "455.092", "function_latency": "312.744"},
        },
        "api_server_response": [
            {
                "date": "2024-12-12",
                "temperature": {"min": 72, "max": 90},
                "units": "Farenheit",
                "query_time": "2024-12-12 22:06:30.420319+00:00",
            },
            {
                "date": "2024-12-13",
                "temperature": {"min": 52, "max": 70},
                "units": "Farenheit",
                "query_time": "2024-12-12 22:06:30.420349+00:00",
            },
        ],
    }
}


def get_data_chunks(stream, n=1):
    chunks = []
    for chunk in stream.iter_lines():
        if chunk:
            chunk = chunk.decode("utf-8")
            chunk_data_id = chunk[0:6]
            assert chunk_data_id == "data: "
            chunk_data = chunk[6:]
            chunk_data = chunk_data.strip()
            chunks.append(chunk_data)
            if len(chunks) >= n:
                break
    return chunks


def get_plano_messages(response_json):
    plano_messages = []
    if response_json and "metadata" in response_json:
        # load plano_state from metadata
        plano_state_str = response_json.get("metadata", {}).get(ARCH_STATE_HEADER, "{}")
        # parse plano_state into json object
        plano_state = json.loads(plano_state_str)
        # load messages from plano_state
        plano_messages_str = plano_state.get("messages", "[]")
        # parse messages into json object
        plano_messages = json.loads(plano_messages_str)
        # append messages from plano gateway to history
        return plano_messages
    return []
