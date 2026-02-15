import pytest
import rich_click as click

from planoai import trace_cmd


class _FakeGrpcServer:
    def add_insecure_port(self, _address: str) -> int:
        raise RuntimeError("bind failed")

    def start(self) -> None:
        return None


def test_create_trace_server_raises_bind_error(monkeypatch):
    monkeypatch.setattr(
        trace_cmd.grpc, "server", lambda *_args, **_kwargs: _FakeGrpcServer()
    )
    monkeypatch.setattr(
        trace_cmd.trace_service_pb2_grpc,
        "add_TraceServiceServicer_to_server",
        lambda *_args, **_kwargs: None,
    )

    with pytest.raises(trace_cmd.TraceListenerBindError) as excinfo:
        trace_cmd._create_trace_server("0.0.0.0", 4317)

    assert "already in use" in str(excinfo.value)
    assert "planoai trace listen --port" in str(excinfo.value)


def test_start_trace_listener_converts_bind_error_to_click_exception(monkeypatch):
    monkeypatch.setattr(
        trace_cmd,
        "_create_trace_server",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(
            trace_cmd.TraceListenerBindError("port in use")
        ),
    )

    with pytest.raises(click.ClickException) as excinfo:
        trace_cmd._start_trace_listener("0.0.0.0", 4317)

    assert "port in use" in str(excinfo.value)
