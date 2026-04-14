#!/usr/bin/env python3
"""
Stress test for Plano routing service to detect memory leaks.

Sends sustained traffic to the routing endpoint and monitors memory
via the /debug/memstats and /debug/state_size endpoints.

Usage:
    # Against a local Plano instance (docker or native)
    python routing_stress.py --base-url http://localhost:12000

    # Custom parameters
    python routing_stress.py \
        --base-url http://localhost:12000 \
        --num-requests 5000 \
        --concurrency 20 \
        --poll-interval 5 \
        --growth-threshold 3.0

Requirements:
    pip install httpx
"""
from __future__ import annotations

import argparse
import asyncio
import json
import sys
import time
import uuid
from dataclasses import dataclass, field

import httpx


@dataclass
class MemSnapshot:
    timestamp: float
    allocated_bytes: int
    resident_bytes: int
    state_entries: int
    state_bytes: int
    requests_completed: int


@dataclass
class StressResult:
    snapshots: list[MemSnapshot] = field(default_factory=list)
    total_requests: int = 0
    total_errors: int = 0
    elapsed_secs: float = 0.0
    passed: bool = True
    failure_reason: str = ""


def make_routing_body(unique: bool = True) -> dict:
    """Build a minimal chat-completions body for the routing endpoint."""
    return {
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": f"test message {uuid.uuid4() if unique else 'static'}"}
        ],
    }


async def poll_debug_endpoints(
    client: httpx.AsyncClient,
    base_url: str,
    requests_completed: int,
) -> MemSnapshot | None:
    try:
        mem_resp = await client.get(f"{base_url}/debug/memstats", timeout=5)
        mem_data = mem_resp.json()

        state_resp = await client.get(f"{base_url}/debug/state_size", timeout=5)
        state_data = state_resp.json()

        return MemSnapshot(
            timestamp=time.time(),
            allocated_bytes=mem_data.get("allocated_bytes", 0),
            resident_bytes=mem_data.get("resident_bytes", 0),
            state_entries=state_data.get("entry_count", 0),
            state_bytes=state_data.get("estimated_bytes", 0),
            requests_completed=requests_completed,
        )
    except Exception as e:
        print(f"  [warn] failed to poll debug endpoints: {e}", file=sys.stderr)
        return None


async def send_requests(
    client: httpx.AsyncClient,
    url: str,
    count: int,
    semaphore: asyncio.Semaphore,
    counter: dict,
):
    """Send `count` routing requests, respecting the concurrency semaphore."""
    for _ in range(count):
        async with semaphore:
            try:
                body = make_routing_body(unique=True)
                resp = await client.post(url, json=body, timeout=30)
                if resp.status_code >= 400:
                    counter["errors"] += 1
            except Exception:
                counter["errors"] += 1
            finally:
                counter["completed"] += 1


async def run_stress_test(
    base_url: str,
    num_requests: int,
    concurrency: int,
    poll_interval: float,
    growth_threshold: float,
) -> StressResult:
    result = StressResult()
    routing_url = f"{base_url}/routing/v1/chat/completions"

    print(f"Stress test config:")
    print(f"  base_url:         {base_url}")
    print(f"  routing_url:      {routing_url}")
    print(f"  num_requests:     {num_requests}")
    print(f"  concurrency:      {concurrency}")
    print(f"  poll_interval:    {poll_interval}s")
    print(f"  growth_threshold: {growth_threshold}x")
    print()

    async with httpx.AsyncClient() as client:
        # Take baseline snapshot
        baseline = await poll_debug_endpoints(client, base_url, 0)
        if baseline:
            result.snapshots.append(baseline)
            print(f"[baseline] allocated={baseline.allocated_bytes:,}B "
                  f"resident={baseline.resident_bytes:,}B "
                  f"state_entries={baseline.state_entries}")
        else:
            print("[warn] could not get baseline snapshot, continuing anyway")

        counter = {"completed": 0, "errors": 0}
        semaphore = asyncio.Semaphore(concurrency)

        start = time.time()

        # Launch request sender and poller concurrently
        sender = asyncio.create_task(
            send_requests(client, routing_url, num_requests, semaphore, counter)
        )

        # Poll memory while requests are in flight
        while not sender.done():
            await asyncio.sleep(poll_interval)
            snapshot = await poll_debug_endpoints(client, base_url, counter["completed"])
            if snapshot:
                result.snapshots.append(snapshot)
                print(
                    f"  [{counter['completed']:>6}/{num_requests}] "
                    f"allocated={snapshot.allocated_bytes:,}B "
                    f"resident={snapshot.resident_bytes:,}B "
                    f"state_entries={snapshot.state_entries} "
                    f"state_bytes={snapshot.state_bytes:,}B"
                )

        await sender
        result.elapsed_secs = time.time() - start
        result.total_requests = counter["completed"]
        result.total_errors = counter["errors"]

        # Final snapshot
        final = await poll_debug_endpoints(client, base_url, counter["completed"])
        if final:
            result.snapshots.append(final)

    # Analyze results
    print()
    print(f"Completed {result.total_requests} requests in {result.elapsed_secs:.1f}s "
          f"({result.total_errors} errors)")

    if len(result.snapshots) >= 2:
        first = result.snapshots[0]
        last = result.snapshots[-1]

        if first.resident_bytes > 0:
            growth_ratio = last.resident_bytes / first.resident_bytes
            print(f"Memory growth: {first.resident_bytes:,}B -> {last.resident_bytes:,}B "
                  f"({growth_ratio:.2f}x)")

            if growth_ratio > growth_threshold:
                result.passed = False
                result.failure_reason = (
                    f"Memory grew {growth_ratio:.2f}x (threshold: {growth_threshold}x). "
                    f"Likely memory leak detected."
                )
                print(f"FAIL: {result.failure_reason}")
            else:
                print(f"PASS: Memory growth {growth_ratio:.2f}x is within {growth_threshold}x threshold")

        print(f"State store: {last.state_entries} entries, {last.state_bytes:,}B")
    else:
        print("[warn] not enough snapshots to analyze memory growth")

    return result


def main():
    parser = argparse.ArgumentParser(description="Plano routing service stress test")
    parser.add_argument("--base-url", default="http://localhost:12000",
                        help="Base URL of the Plano instance")
    parser.add_argument("--num-requests", type=int, default=2000,
                        help="Total number of requests to send")
    parser.add_argument("--concurrency", type=int, default=10,
                        help="Max concurrent requests")
    parser.add_argument("--poll-interval", type=float, default=5.0,
                        help="Seconds between memory polls")
    parser.add_argument("--growth-threshold", type=float, default=3.0,
                        help="Max allowed memory growth ratio (fail if exceeded)")
    args = parser.parse_args()

    result = asyncio.run(run_stress_test(
        base_url=args.base_url,
        num_requests=args.num_requests,
        concurrency=args.concurrency,
        poll_interval=args.poll_interval,
        growth_threshold=args.growth_threshold,
    ))

    # Write results JSON for CI consumption
    report = {
        "passed": result.passed,
        "failure_reason": result.failure_reason,
        "total_requests": result.total_requests,
        "total_errors": result.total_errors,
        "elapsed_secs": result.elapsed_secs,
        "snapshots": [
            {
                "timestamp": s.timestamp,
                "allocated_bytes": s.allocated_bytes,
                "resident_bytes": s.resident_bytes,
                "state_entries": s.state_entries,
                "state_bytes": s.state_bytes,
                "requests_completed": s.requests_completed,
            }
            for s in result.snapshots
        ],
    }
    print()
    print(json.dumps(report, indent=2))

    sys.exit(0 if result.passed else 1)


if __name__ == "__main__":
    main()
