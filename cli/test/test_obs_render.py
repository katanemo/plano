from datetime import datetime, timedelta, timezone

from planoai.obs.collector import LLMCall
from planoai.obs.render import aggregates, model_rollups, route_hits


def _call(model: str, ts: datetime, prompt=0, completion=0, cost=None, route=None, session=None, cache_read=0, cache_write=0):
    return LLMCall(
        request_id="r",
        timestamp=ts,
        model=model,
        prompt_tokens=prompt,
        completion_tokens=completion,
        cached_input_tokens=cache_read,
        cache_creation_tokens=cache_write,
        cost_usd=cost,
        route_name=route,
        session_id=session,
    )


def test_aggregates_sum_and_session_counts():
    now = datetime.now(tz=timezone.utc).astimezone()
    calls = [
        _call("m1", now - timedelta(seconds=50), prompt=10, completion=5, cost=0.001, session="s1"),
        _call("m2", now - timedelta(seconds=40), prompt=20, completion=10, cost=0.002, session="s1"),
        _call("m1", now - timedelta(seconds=30), prompt=30, completion=15, cost=0.003, session="s2"),
    ]
    stats = aggregates(calls)
    assert stats.count == 3
    assert stats.total_cost_usd == 0.006
    assert stats.total_input_tokens == 60
    assert stats.total_output_tokens == 30
    assert stats.distinct_sessions == 2
    assert stats.current_session == "s2"


def test_rollups_split_by_model_and_cache():
    now = datetime.now(tz=timezone.utc).astimezone()
    calls = [
        _call("m1", now, prompt=10, completion=5, cost=0.001, cache_write=3, cache_read=7),
        _call("m1", now, prompt=20, completion=10, cost=0.002, cache_read=1),
        _call("m2", now, prompt=30, completion=15, cost=0.004),
    ]
    rollups = model_rollups(calls)
    by_model = {r.model: r for r in rollups}
    assert by_model["m1"].requests == 2
    assert by_model["m1"].input_tokens == 30
    assert by_model["m1"].cache_write == 3
    assert by_model["m1"].cache_read == 8
    assert by_model["m2"].input_tokens == 30


def test_route_hits_only_for_routed_calls():
    now = datetime.now(tz=timezone.utc).astimezone()
    calls = [
        _call("m", now, route="code"),
        _call("m", now, route="code"),
        _call("m", now, route="summarization"),
        _call("m", now),  # no route
    ]
    hits = route_hits(calls)
    # Only calls with route names are counted.
    assert sum(n for _, n, _ in hits) == 3
    hits_by_name = {name: (n, pct) for name, n, pct in hits}
    assert hits_by_name["code"][0] == 2
    assert hits_by_name["summarization"][0] == 1


def test_route_hits_empty_when_no_routes():
    now = datetime.now(tz=timezone.utc).astimezone()
    calls = [_call("m", now), _call("m", now)]
    assert route_hits(calls) == []
