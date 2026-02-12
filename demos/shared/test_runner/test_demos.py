import json
import os
from common import get_plano_messages
import pytest
import requests
from deepdiff import DeepDiff
import logging
import yaml

logger = logging.getLogger(__name__)
logger.setLevel(logging.DEBUG)

ARCHGW_ENDPOINT = os.getenv(
    "ARCHGW_ENDPOINT", "http://localhost:10000/v1/chat/completions"
)

# Load test data from YAML file
with open(os.getenv("TEST_DATA", "test_data.yaml"), "r") as file:
    test_data_yaml = yaml.safe_load(file)


@pytest.mark.parametrize(
    "test_data",
    [
        pytest.param(test_case, id=test_case["id"])
        for test_case in test_data_yaml["test_cases"]
    ],
)
def test_demos(test_data):
    input = test_data["input"]
    expected_tools = test_data["expected_tools"]
    expected_output_contains = test_data["expected_output_contains"]

    response = requests.post(ARCHGW_ENDPOINT, json=input)
    assert response.status_code == 200
    # ensure that response is json
    assert response.headers["content-type"] == "application/json"

    response_json = response.json()
    assert response_json.get("model").startswith("gpt-4o")
    choices = response_json.get("choices", [])
    assert len(choices) > 0

    # ensure that model responded according to the expectation
    assert "role" in choices[0]["message"]
    assert choices[0]["message"]["role"] == "assistant"
    assert expected_output_contains.lower() in choices[0]["message"]["content"].lower()

    # now verify plano_messages (tool call and api response) that are sent as response metadata
    plano_messages = get_plano_messages(response_json)
    assert len(plano_messages) == 2
    tool_calls_message = plano_messages[0]
    tool_calls = tool_calls_message.get("tool_calls", [])
    assert len(tool_calls) > 0

    # remove dynamic id from tool_calls
    for tool_call in tool_calls:
        tool_call.pop("id", None)
    diff = DeepDiff(expected_tools, tool_calls, ignore_string_case=True)
    assert not diff
