"""Shared fixtures for mock-based tests.

Provides mock HTTP server handlers that simulate OpenAI and Anthropic API responses.
The gateway container routes to host.docker.internal:51001 where the mock server runs.
"""

import json
import pytest
from pytest_httpserver import HTTPServer
from pytest_httpserver.httpserver import HandlerType
from werkzeug.wrappers import Response


@pytest.fixture(scope="session")
def httpserver_listen_address():
    return ("0.0.0.0", 51001)


# ---------------------------------------------------------------------------
# OpenAI Chat Completions helpers
# ---------------------------------------------------------------------------


def make_openai_chat_response(
    content="Hello from mock!", model="gpt-5-mini-2025-08-07", tool_calls=None
):
    message = {"role": "assistant", "content": content}
    finish_reason = "stop"
    if tool_calls:
        message["content"] = None
        message["tool_calls"] = tool_calls
        finish_reason = "tool_calls"
    return {
        "id": "chatcmpl-mock-123",
        "object": "chat.completion",
        "created": 1234567890,
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
    }


def make_openai_chat_stream(content="Hello from mock!", model="gpt-5-mini-2025-08-07"):
    lines = []
    # Role chunk
    lines.append(
        f'data: {{"id":"chatcmpl-mock-123","object":"chat.completion.chunk","created":1234567890,'
        f'"model":"{model}","choices":[{{"index":0,"delta":{{"role":"assistant","content":""}},"finish_reason":null}}]}}\n\n'
    )
    # Content chunks (one per word)
    words = content.split(" ")
    for i, word in enumerate(words):
        prefix = " " if i > 0 else ""
        escaped = json.dumps(f"{prefix}{word}")[1:-1]  # strip quotes from json string
        lines.append(
            f'data: {{"id":"chatcmpl-mock-123","object":"chat.completion.chunk","created":1234567890,'
            f'"model":"{model}","choices":[{{"index":0,"delta":{{"content":"{escaped}"}},"finish_reason":null}}]}}\n\n'
        )
    # Stop chunk
    lines.append(
        f'data: {{"id":"chatcmpl-mock-123","object":"chat.completion.chunk","created":1234567890,'
        f'"model":"{model}","choices":[{{"index":0,"delta":{{}},"finish_reason":"stop"}}]}}\n\n'
    )
    lines.append("data: [DONE]\n\n")
    return "".join(lines)


def make_openai_tool_call_stream(
    model="gpt-5-mini-2025-08-07", tool_name="echo_tool", tool_args='{"text":"hello"}'
):
    lines = []
    # Role chunk
    lines.append(
        f'data: {{"id":"chatcmpl-mock-tool","object":"chat.completion.chunk","created":1234567890,'
        f'"model":"{model}","choices":[{{"index":0,"delta":{{"role":"assistant","content":null}},"finish_reason":null}}]}}\n\n'
    )
    # Tool call chunk - id + function name
    lines.append(
        f'data: {{"id":"chatcmpl-mock-tool","object":"chat.completion.chunk","created":1234567890,'
        f'"model":"{model}","choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"id":"call_mock_123","type":"function","function":{{"name":"{tool_name}","arguments":""}}}}]}},"finish_reason":null}}]}}\n\n'
    )
    # Tool call arguments chunk
    escaped_args = json.dumps(tool_args)[1:-1]
    lines.append(
        f'data: {{"id":"chatcmpl-mock-tool","object":"chat.completion.chunk","created":1234567890,'
        f'"model":"{model}","choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":"{escaped_args}"}}}}]}},"finish_reason":null}}]}}\n\n'
    )
    # Stop chunk
    lines.append(
        f'data: {{"id":"chatcmpl-mock-tool","object":"chat.completion.chunk","created":1234567890,'
        f'"model":"{model}","choices":[{{"index":0,"delta":{{}},"finish_reason":"tool_calls"}}]}}\n\n'
    )
    lines.append("data: [DONE]\n\n")
    return "".join(lines)


# ---------------------------------------------------------------------------
# Anthropic Messages helpers
# ---------------------------------------------------------------------------


def make_anthropic_response(
    content="Hello from mock!", model="claude-sonnet-4-20250514"
):
    return {
        "id": "msg-mock-123",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{"type": "text", "text": content}],
        "stop_reason": "end_turn",
        "stop_sequence": None,
        "usage": {"input_tokens": 10, "output_tokens": 5},
    }


def make_anthropic_stream(content="Hello from mock!", model="claude-sonnet-4-20250514"):
    lines = []
    msg = {
        "id": "msg-mock-123",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [],
        "stop_reason": None,
        "stop_sequence": None,
        "usage": {"input_tokens": 10, "output_tokens": 0},
    }
    lines.append(
        f"event: message_start\ndata: {json.dumps({'type': 'message_start', 'message': msg})}\n\n"
    )
    lines.append(
        f'event: content_block_start\ndata: {{"type":"content_block_start","index":0,"content_block":{{"type":"text","text":""}}}}\n\n'
    )

    words = content.split(" ")
    for i, word in enumerate(words):
        prefix = " " if i > 0 else ""
        text = f"{prefix}{word}"
        escaped = json.dumps(text)[1:-1]
        lines.append(
            f'event: content_block_delta\ndata: {{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{escaped}"}}}}\n\n'
        )

    lines.append(
        f'event: content_block_stop\ndata: {{"type":"content_block_stop","index":0}}\n\n'
    )
    lines.append(
        f'event: message_delta\ndata: {{"type":"message_delta","delta":{{"stop_reason":"end_turn","stop_sequence":null}},"usage":{{"output_tokens":5}}}}\n\n'
    )
    lines.append(f'event: message_stop\ndata: {{"type":"message_stop"}}\n\n')
    return "".join(lines)


def make_anthropic_thinking_stream(
    content="The answer is 4.",
    thinking="Let me think... 2+2=4",
    model="claude-sonnet-4-20250514",
):
    lines = []
    msg = {
        "id": "msg-mock-think",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [],
        "stop_reason": None,
        "stop_sequence": None,
        "usage": {"input_tokens": 10, "output_tokens": 0},
    }
    lines.append(
        f"event: message_start\ndata: {json.dumps({'type': 'message_start', 'message': msg})}\n\n"
    )

    # Thinking block
    lines.append(
        f'event: content_block_start\ndata: {{"type":"content_block_start","index":0,"content_block":{{"type":"thinking","thinking":""}}}}\n\n'
    )
    for word in thinking.split(" "):
        escaped = json.dumps(word)[1:-1]
        lines.append(
            f'event: content_block_delta\ndata: {{"type":"content_block_delta","index":0,"delta":{{"type":"thinking_delta","thinking":"{escaped} "}}}}\n\n'
        )
    lines.append(
        f'event: content_block_stop\ndata: {{"type":"content_block_stop","index":0}}\n\n'
    )

    # Text block
    lines.append(
        f'event: content_block_start\ndata: {{"type":"content_block_start","index":1,"content_block":{{"type":"text","text":""}}}}\n\n'
    )
    for i, word in enumerate(content.split(" ")):
        prefix = " " if i > 0 else ""
        escaped = json.dumps(f"{prefix}{word}")[1:-1]
        lines.append(
            f'event: content_block_delta\ndata: {{"type":"content_block_delta","index":1,"delta":{{"type":"text_delta","text":"{escaped}"}}}}\n\n'
        )
    lines.append(
        f'event: content_block_stop\ndata: {{"type":"content_block_stop","index":1}}\n\n'
    )

    lines.append(
        f'event: message_delta\ndata: {{"type":"message_delta","delta":{{"stop_reason":"end_turn","stop_sequence":null}},"usage":{{"output_tokens":20}}}}\n\n'
    )
    lines.append(f'event: message_stop\ndata: {{"type":"message_stop"}}\n\n')
    return "".join(lines)


# ---------------------------------------------------------------------------
# OpenAI Responses API helpers
# ---------------------------------------------------------------------------


def make_responses_api_response(
    content="Hello from mock!",
    model="gpt-5-mini-2025-08-07",
    response_id="resp-mock-123",
):
    return {
        "id": response_id,
        "object": "response",
        "created_at": 1234567890,
        "model": model,
        "output": [
            {
                "type": "message",
                "id": "msg_mock_123",
                "status": "completed",
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": content, "annotations": []}
                ],
            }
        ],
        "status": "completed",
        "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
    }


def make_responses_api_stream(
    content="Hello from mock!",
    model="gpt-5-mini-2025-08-07",
    response_id="resp-mock-123",
):
    lines = []
    resp_base = {
        "id": response_id,
        "object": "response",
        "created_at": 1234567890,
        "model": model,
        "output": [],
        "status": "in_progress",
    }
    lines.append(
        f"event: response.created\ndata: {json.dumps({'type': 'response.created', 'response': resp_base})}\n\n"
    )
    lines.append(
        f'event: response.output_item.added\ndata: {{"type":"response.output_item.added","output_index":0,'
        f'"item":{{"type":"message","id":"msg_mock_123","status":"in_progress","role":"assistant","content":[]}}}}\n\n'
    )
    lines.append(
        f'event: response.content_part.added\ndata: {{"type":"response.content_part.added","output_index":0,'
        f'"content_index":0,"part":{{"type":"output_text","text":"","annotations":[]}}}}\n\n'
    )

    words = content.split(" ")
    for i, word in enumerate(words):
        prefix = " " if i > 0 else ""
        escaped = json.dumps(f"{prefix}{word}")[1:-1]
        lines.append(
            f'event: response.output_text.delta\ndata: {{"type":"response.output_text.delta","output_index":0,'
            f'"content_index":0,"delta":"{escaped}"}}\n\n'
        )

    lines.append(
        f'event: response.output_text.done\ndata: {{"type":"response.output_text.done","output_index":0,'
        f'"content_index":0,"text":"{json.dumps(content)[1:-1]}"}}\n\n'
    )

    final_item = {
        "type": "message",
        "id": "msg_mock_123",
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": content, "annotations": []}],
    }
    lines.append(
        f"event: response.output_item.done\ndata: {json.dumps({'type': 'response.output_item.done', 'output_index': 0, 'item': final_item})}\n\n"
    )

    final_resp = dict(
        resp_base,
        output=[final_item],
        status="completed",
        usage={"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
    )
    lines.append(
        f"event: response.completed\ndata: {json.dumps({'type': 'response.completed', 'response': final_resp})}\n\n"
    )
    return "".join(lines)


# ---------------------------------------------------------------------------
# Mock server setup helpers
# ---------------------------------------------------------------------------


def setup_openai_chat_mock(
    httpserver: HTTPServer, content="Hello from mock!", tool_calls=None
):
    """Register a permanent handler for /v1/chat/completions on the mock server.
    Returns a list that will be populated with captured request bodies.
    """
    captured = []

    def handler(request):
        body = json.loads(request.data)
        captured.append(body)
        is_stream = body.get("stream", False)
        model = body.get("model", "gpt-5-mini-2025-08-07")

        if tool_calls and not is_stream:
            return Response(
                json.dumps(
                    make_openai_chat_response(model=model, tool_calls=tool_calls)
                ),
                status=200,
                content_type="application/json",
            )
        if is_stream:
            return Response(
                make_openai_chat_stream(content=content, model=model),
                status=200,
                content_type="text/event-stream",
            )
        return Response(
            json.dumps(make_openai_chat_response(content=content, model=model)),
            status=200,
            content_type="application/json",
        )

    httpserver.expect_request(
        "/v1/chat/completions",
        method="POST",
        handler_type=HandlerType.PERMANENT,
    ).respond_with_handler(handler)
    return captured


def setup_anthropic_mock(
    httpserver: HTTPServer, content="Hello from mock!", thinking=False
):
    """Register a permanent handler for /v1/messages on the mock server.
    Returns a list that will be populated with captured request bodies.
    """
    captured = []

    def handler(request):
        body = json.loads(request.data)
        captured.append(body)
        is_stream = body.get("stream", False)
        model = body.get("model", "claude-sonnet-4-20250514")

        if thinking and is_stream:
            return Response(
                make_anthropic_thinking_stream(model=model),
                status=200,
                content_type="text/event-stream",
            )
        if is_stream:
            return Response(
                make_anthropic_stream(content=content, model=model),
                status=200,
                content_type="text/event-stream",
            )
        return Response(
            json.dumps(make_anthropic_response(content=content, model=model)),
            status=200,
            content_type="application/json",
        )

    httpserver.expect_request(
        "/v1/messages",
        method="POST",
        handler_type=HandlerType.PERMANENT,
    ).respond_with_handler(handler)
    return captured


def setup_responses_api_mock(httpserver: HTTPServer, content="Hello from mock!"):
    """Register a permanent handler for /v1/responses on the mock server.
    Returns a list that will be populated with captured request bodies.
    """
    captured = []
    call_count = [0]

    def handler(request):
        body = json.loads(request.data)
        captured.append(body)
        call_count[0] += 1
        is_stream = body.get("stream", False)
        model = body.get("model", "gpt-5-mini-2025-08-07")
        response_id = f"resp-mock-{call_count[0]}"

        if is_stream:
            return Response(
                make_responses_api_stream(
                    content=content, model=model, response_id=response_id
                ),
                status=200,
                content_type="text/event-stream",
            )
        return Response(
            json.dumps(
                make_responses_api_response(
                    content=content, model=model, response_id=response_id
                )
            ),
            status=200,
            content_type="application/json",
        )

    httpserver.expect_request(
        "/v1/responses",
        method="POST",
        handler_type=HandlerType.PERMANENT,
    ).respond_with_handler(handler)
    return captured


def setup_error_mock(
    httpserver: HTTPServer, path="/v1/chat/completions", status=400, body=None
):
    """Register a handler that returns an error response."""
    error_body = body or json.dumps(
        {
            "error": {
                "message": "Bad Request",
                "type": "invalid_request_error",
                "code": "bad_request",
            }
        }
    )
    httpserver.expect_request(path, method="POST").respond_with_data(
        error_body,
        status=status,
        content_type="application/json",
    )
