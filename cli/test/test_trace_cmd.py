import copy
import json
import re
from pathlib import Path

import pytest
from click.testing import CliRunner

from planoai.trace_cmd import trace
import planoai.trace_cmd as trace_cmd


def _load_success_traces() -> list[dict]:
    source_path = Path(__file__).parent / "source" / "success.json"
    payload = json.loads(source_path.read_text(encoding="utf-8"))
    return payload["traces"]


def _load_failure_traces() -> list[dict]:
    source_path = Path(__file__).parent / "source" / "failure.json"
    payload = json.loads(source_path.read_text(encoding="utf-8"))
    return payload["traces"]


def _build_trace_set() -> list[dict]:
    traces = copy.deepcopy(_load_success_traces())
    primary = traces[0]

    secondary = copy.deepcopy(primary)
    secondary["trace_id"] = "1234567890abcdef1234567890abcdef"
    for span in secondary.get("spans", []):
        span["traceId"] = secondary["trace_id"]
        if span.get("startTimeUnixNano", "").isdigit():
            span["startTimeUnixNano"] = str(
                int(span["startTimeUnixNano"]) - 1_000_000_000
            )
        if span.get("endTimeUnixNano", "").isdigit():
            span["endTimeUnixNano"] = str(int(span["endTimeUnixNano"]) - 1_000_000_000)

    return [primary, secondary]


def _json_from_output(output: str) -> dict:
    start = output.find("{")
    if start == -1:
        raise AssertionError(f"No JSON object found in output:\n{output}")
    return json.loads(output[start:])


def _plain_output(output: str) -> str:
    # Strip ANSI color/style sequences emitted by rich-click in CI terminals.
    return re.sub(r"\x1b\[[0-9;]*m", "", output)


@pytest.fixture
def runner() -> CliRunner:
    return CliRunner()


@pytest.fixture
def traces() -> list[dict]:
    return _build_trace_set()


@pytest.fixture
def failure_traces() -> list[dict]:
    return copy.deepcopy(_load_failure_traces())


class _FakeGrpcServer:
    def add_insecure_port(self, _address: str) -> int:
        raise RuntimeError("bind failed")

    def start(self) -> None:
        return None


def test_start_trace_server_raises_bind_error(monkeypatch):
    monkeypatch.setattr(
        trace_cmd.grpc, "server", lambda *_args, **_kwargs: _FakeGrpcServer()
    )
    monkeypatch.setattr(
        trace_cmd.trace_service_pb2_grpc,
        "add_TraceServiceServicer_to_server",
        lambda *_args, **_kwargs: None,
    )

    with pytest.raises(trace_cmd.TraceListenerBindError) as excinfo:
        trace_cmd._start_trace_server("0.0.0.0", 4317)

    assert "already in use" in str(excinfo.value)
    assert "planoai trace listen" in str(excinfo.value)


def test_trace_listen_starts_listener_with_defaults(runner, monkeypatch):
    seen = {}

    def fake_start(host: str, port: int) -> None:
        seen["host"] = host
        seen["port"] = port

    monkeypatch.setattr(trace_cmd, "_start_trace_listener", fake_start)

    result = runner.invoke(trace, ["listen"])

    assert result.exit_code == 0, result.output
    assert seen == {"host": "0.0.0.0", "port": trace_cmd.DEFAULT_GRPC_PORT}


def test_trace_down_prints_success_when_listener_stopped(runner, monkeypatch):
    monkeypatch.setattr(trace_cmd, "_stop_background_listener", lambda: True)

    result = runner.invoke(trace, ["down"])

    assert result.exit_code == 0, result.output
    assert "Trace listener stopped" in result.output


def test_trace_down_prints_no_listener_when_not_running(runner, monkeypatch):
    monkeypatch.setattr(trace_cmd, "_stop_background_listener", lambda: False)

    result = runner.invoke(trace, ["down"])

    assert result.exit_code == 0, result.output
    assert "No background trace listener running" in result.output


def test_trace_default_target_uses_last_and_builds_first_trace(
    runner, monkeypatch, traces
):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))
    seen = {}

    def fake_build_tree(trace_obj, _console, verbose=False):
        seen["trace_id"] = trace_obj["trace_id"]
        seen["verbose"] = verbose

    monkeypatch.setattr(trace_cmd, "_build_tree", fake_build_tree)

    result = runner.invoke(trace, [])

    assert result.exit_code == 0, result.output
    assert seen["trace_id"] == traces[0]["trace_id"]
    assert seen["verbose"] is False


def test_trace_list_any_prints_short_trace_ids(runner, monkeypatch, traces):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))

    result = runner.invoke(trace, ["--list", "--no-interactive", "any"])

    assert result.exit_code == 0, result.output
    assert "Trace IDs:" in result.output
    assert traces[0]["trace_id"][:8] in result.output
    assert traces[1]["trace_id"][:8] in result.output


def test_trace_list_target_conflict_errors(runner, traces, monkeypatch):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))

    result = runner.invoke(trace, ["--list", traces[0]["trace_id"]])

    assert result.exit_code != 0
    assert "Target and --list cannot be used together." in _plain_output(result.output)


def test_trace_json_list_with_limit_outputs_trace_ids(runner, monkeypatch, traces):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))

    result = runner.invoke(trace, ["--list", "any", "--json", "--limit", "1"])

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    assert payload == {"trace_ids": [traces[0]["trace_id"]]}


def test_trace_json_for_short_target_returns_one_trace(runner, monkeypatch, traces):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))
    short_target = traces[0]["trace_id"][:8]

    result = runner.invoke(trace, [short_target, "--json"])

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    assert len(payload["traces"]) == 1
    assert payload["traces"][0]["trace_id"] == traces[0]["trace_id"]


@pytest.mark.parametrize(
    ("target", "message"),
    [
        ("abc", "Trace ID must be 8 or 32 hex characters."),
        ("00000000", "Short trace ID must be 8 hex characters."),
        ("0" * 32, "Trace ID must be 32 hex characters."),
    ],
)
def test_trace_target_validation_errors(runner, target, message):
    result = runner.invoke(trace, [target])
    assert result.exit_code != 0
    assert message in _plain_output(result.output)


def test_trace_where_invalid_format_errors(runner):
    result = runner.invoke(trace, ["any", "--where", "bad-format"])

    assert result.exit_code != 0
    assert "Invalid --where filter(s): bad-format. Use key=value." in _plain_output(
        result.output
    )


def test_trace_where_unknown_key_errors(runner, monkeypatch, traces):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))

    result = runner.invoke(trace, ["any", "--where", "not.a.real.key=value"])

    assert result.exit_code != 0
    assert "Unknown --where key(s): not.a.real.key" in _plain_output(result.output)


def test_trace_where_filters_to_matching_trace(runner, monkeypatch, traces):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))

    result = runner.invoke(
        trace, ["any", "--where", "agent_id=weather_agent", "--json"]
    )

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    assert [trace_item["trace_id"] for trace_item in payload["traces"]] == [
        traces[0]["trace_id"],
        traces[1]["trace_id"],
    ]


def test_trace_where_and_filters_can_exclude_all(runner, monkeypatch, traces):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))

    result = runner.invoke(
        trace,
        [
            "any",
            "--where",
            "agent_id=weather_agent",
            "--where",
            "http.status_code=500",
            "--json",
        ],
    )

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    assert payload == {"traces": []}


def test_trace_filter_restricts_attributes_by_pattern(runner, monkeypatch, traces):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))

    result = runner.invoke(trace, ["any", "--filter", "http.*", "--json"])

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    for trace_item in payload["traces"]:
        for span in trace_item["spans"]:
            for attr in span.get("attributes", []):
                assert attr["key"].startswith("http.")


def test_trace_filter_unmatched_warns_and_returns_unfiltered(
    runner, monkeypatch, traces
):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))

    result = runner.invoke(trace, ["any", "--filter", "not-found-*", "--json"])

    assert result.exit_code == 0, result.output
    assert (
        "Filter key(s) not found: not-found-*. Returning unfiltered traces."
        in result.output
    )
    payload = _json_from_output(result.output)
    assert len(payload["traces"]) == len(traces)


def test_trace_since_can_filter_out_old_traces(runner, monkeypatch, traces):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(traces))
    monkeypatch.setattr(trace_cmd.time, "time", lambda: 1_999_999_999.0)

    result = runner.invoke(trace, ["any", "--since", "1m", "--json"])

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    assert payload == {"traces": []}


def test_trace_negative_limit_errors(runner):
    result = runner.invoke(trace, ["any", "--limit", "-1"])

    assert result.exit_code != 0
    assert "Limit must be greater than or equal to 0." in _plain_output(result.output)


def test_trace_empty_data_prints_no_traces_found(runner, monkeypatch):
    monkeypatch.setattr(trace_cmd, "_fetch_traces_raw", lambda: [])

    result = runner.invoke(trace, [])

    assert result.exit_code == 0, result.output
    assert "No traces found." in result.output


def test_trace_invalid_filter_token_errors(runner):
    result = runner.invoke(trace, ["any", "--filter", "http.method,"])

    assert result.exit_code != 0
    assert "Filter contains empty tokens." in _plain_output(result.output)


def test_trace_failure_json_any_contains_all_fixture_trace_ids(
    runner, monkeypatch, failure_traces
):
    monkeypatch.setattr(
        trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(failure_traces)
    )

    result = runner.invoke(trace, ["any", "--json"])

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    assert [item["trace_id"] for item in payload["traces"]] == [
        "f7a31829c4b5d6e8a9f0b1c2d3e4f5a6",
        "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6",
        "b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7",
    ]


@pytest.mark.parametrize(
    ("status_code", "expected_trace_ids"),
    [
        ("503", ["f7a31829c4b5d6e8a9f0b1c2d3e4f5a6"]),
        ("429", ["a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"]),
        ("500", ["b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7"]),
    ],
)
def test_trace_failure_where_status_filters_expected_traces(
    runner, monkeypatch, failure_traces, status_code, expected_trace_ids
):
    monkeypatch.setattr(
        trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(failure_traces)
    )

    result = runner.invoke(
        trace, ["any", "--where", f"http.status_code={status_code}", "--json"]
    )

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    assert [item["trace_id"] for item in payload["traces"]] == expected_trace_ids


def test_trace_failure_default_render_shows_service_unavailable_banner(
    runner, monkeypatch, failure_traces
):
    monkeypatch.setattr(
        trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(failure_traces)
    )

    result = runner.invoke(trace, [])

    assert result.exit_code == 0, result.output
    assert "Service Unavailable" in result.output
    assert "503" in result.output


def test_trace_failure_filter_keeps_http_status_code_attributes(
    runner, monkeypatch, failure_traces
):
    monkeypatch.setattr(
        trace_cmd, "_fetch_traces_raw", lambda: copy.deepcopy(failure_traces)
    )

    result = runner.invoke(trace, ["any", "--filter", "http.status_code", "--json"])

    assert result.exit_code == 0, result.output
    payload = _json_from_output(result.output)
    assert payload["traces"], "Expected traces in failure fixture"
    for trace_item in payload["traces"]:
        for span in trace_item["spans"]:
            keys = [attr["key"] for attr in span.get("attributes", [])]
            assert set(keys).issubset({"http.status_code"})
