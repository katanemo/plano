#!/usr/bin/env python3
"""
verify_affinity.py — Prove that Redis-backed session affinity works across Plano replicas.

Strategy
--------
Kubernetes round-robin is non-deterministic, so simply hammering the LoadBalancer
service is not a reliable proof. Instead this script:

  1. Discovers the two (or more) Plano pod names with kubectl.
  2. Opens a kubectl port-forward tunnel to EACH pod on a separate local port.
  3. Pins a session via Pod 0 (writes the Redis key).
  4. Reads the same session via Pod 1 (must return the same model — reads Redis).
  5. Repeats across N sessions, round-robining which pod sets vs. reads the pin.

If every round returns the same model, Redis is the shared source of truth and
multi-replica affinity is proven.

Usage
-----
  # From inside the cluster network (e.g. CI job or jumpbox):
  python verify_affinity.py --url http://<LoadBalancer-IP>:12000

  # From your laptop (uses kubectl port-forward automatically):
  python verify_affinity.py

  # More sessions / rounds:
  python verify_affinity.py --sessions 5 --rounds 6

Requirements
------------
  kubectl   — configured to reach the plano-demo namespace
  Python 3.11+
"""

import argparse
import http.client
import json
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from contextlib import contextmanager

NAMESPACE = "plano-demo"
BASE_LOCAL_PORT = 19100  # port-forward starts here, increments per pod

PROMPTS = [
    "Explain the difference between TCP and UDP in detail.",
    "Write a merge sort implementation in Python.",
    "What is quantum entanglement?",
    "Describe the CAP theorem with examples.",
    "How does gradient descent work in neural networks?",
    "What is the time complexity of Dijkstra's algorithm?",
]


# ---------------------------------------------------------------------------
# kubectl helpers
# ---------------------------------------------------------------------------


def get_pod_names() -> list[str]:
    """Return running Plano pod names in the plano-demo namespace."""
    result = subprocess.run(
        [
            "kubectl",
            "get",
            "pods",
            "-n",
            NAMESPACE,
            "-l",
            "app=plano",
            "--field-selector=status.phase=Running",
            "-o",
            "jsonpath={.items[*].metadata.name}",
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    pods = result.stdout.strip().split()
    if not pods or pods == [""]:
        raise RuntimeError(
            f"No running Plano pods found in namespace '{NAMESPACE}'.\n"
            "Is the cluster deployed? Run: ./deploy.sh"
        )
    return pods


@contextmanager
def port_forward(pod_name: str, local_port: int, remote_port: int = 12000):
    """Context manager that starts and stops a kubectl port-forward."""
    proc = subprocess.Popen(
        [
            "kubectl",
            "port-forward",
            f"pod/{pod_name}",
            f"{local_port}:{remote_port}",
            "-n",
            NAMESPACE,
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    # Give the tunnel a moment to establish
    time.sleep(1.5)
    try:
        yield f"http://localhost:{local_port}"
    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()


# ---------------------------------------------------------------------------
# HTTP helpers
# ---------------------------------------------------------------------------


def chat(
    base_url: str,
    session_id: str | None,
    message: str,
    model: str = "openai/gpt-4o-mini",
    retries: int = 3,
    retry_delay: float = 5.0,
) -> dict:
    payload = json.dumps(
        {
            "model": model,
            "messages": [{"role": "user", "content": message}],
        }
    ).encode()

    headers = {"Content-Type": "application/json"}
    if session_id:
        headers["x-model-affinity"] = session_id

    req = urllib.request.Request(
        f"{base_url}/v1/chat/completions",
        data=payload,
        headers=headers,
        method="POST",
    )
    last_err: Exception | None = None
    for attempt in range(retries):
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                body = resp.read()
                if not body:
                    raise RuntimeError(f"Empty response body from {base_url}")
                return json.loads(body)
        except urllib.error.HTTPError as e:
            if e.code in (503, 502, 429) and attempt < retries - 1:
                time.sleep(retry_delay * (attempt + 1))
                last_err = e
                continue
            raise RuntimeError(f"Request to {base_url} failed: {e}") from e
        except (
            urllib.error.URLError,
            http.client.RemoteDisconnected,
            RuntimeError,
        ) as e:
            if attempt < retries - 1:
                time.sleep(retry_delay * (attempt + 1))
                last_err = e
                continue
            raise RuntimeError(f"Request to {base_url} failed: {e}") from e
        except json.JSONDecodeError as e:
            raise RuntimeError(f"Invalid JSON from {base_url}: {e}") from e
    raise RuntimeError(
        f"Request to {base_url} failed after {retries} attempts: {last_err}"
    )


def extract_model(response: dict) -> str:
    return response.get("model", "<unknown>")


# ---------------------------------------------------------------------------
# Verification phases
# ---------------------------------------------------------------------------


def phase_loadbalancer(url: str, rounds: int) -> None:
    """Phase 0: quick smoke test against the LoadBalancer / provided URL."""
    print("=" * 66)
    print(f"Phase 0: Smoke test against {url}")
    print("=" * 66)
    for i in range(rounds):
        resp = chat(url, None, PROMPTS[i % len(PROMPTS)])
        print(f"  Request {i + 1}: model = {extract_model(resp)}")
    print()


def phase_cross_replica(
    pod_urls: dict[str, str], num_sessions: int, rounds: int
) -> bool:
    """
    Phase 1 — Cross-replica pinning.

    For each session:
      • Round 1: send to pod_A  (sets the Redis key)
      • Rounds 2+: alternate between pod_A and pod_B
      • Assert every round returns the same model.
    """
    pod_names = list(pod_urls.keys())
    all_passed = True
    session_results: dict[str, dict] = {}

    print("=" * 66)
    print("Phase 1: Cross-replica session pinning")
    print(f"  Pods under test : {', '.join(pod_names)}")
    print(f"  Sessions        : {num_sessions}")
    print(f"  Rounds/session  : {rounds}")
    print()
    print("  Each session is PINNED via one pod and VERIFIED via another.")
    print("  If Redis is shared, every round must return the same model.")
    print("=" * 66)

    for s in range(num_sessions):
        session_id = f"k8s-session-{s + 1:03d}"
        models_seen = []
        pod_sequence = []

        for r in range(rounds):
            # Alternate which pod handles each round
            pod_name = pod_names[r % len(pod_names)]
            url = pod_urls[pod_name]

            try:
                resp = chat(url, session_id, PROMPTS[(s + r) % len(PROMPTS)])
                model = extract_model(resp)
            except RuntimeError as e:
                print(f"  ERROR on {pod_name} round {r + 1}: {e}")
                all_passed = False
                continue

            models_seen.append(model)
            pod_sequence.append(pod_name)

        unique_models = set(models_seen)
        passed = len(unique_models) == 1

        session_results[session_id] = {
            "passed": passed,
            "model": models_seen[0] if models_seen else "<none>",
            "unique_models": unique_models,
            "pod_sequence": pod_sequence,
        }

        status = "PASS" if passed else "FAIL"
        detail = models_seen[0] if passed else str(unique_models)
        print(f"\n  {status}  {session_id}")
        print(f"        model      : {detail}")
        print(f"        pod order  : {' → '.join(pod_sequence)}")

        if not passed:
            all_passed = False

    return all_passed


def phase_redis_inspect(num_sessions: int) -> None:
    """Phase 2: read keys directly from Redis to show what's stored."""
    print()
    print("=" * 66)
    print("Phase 2: Redis key inspection")
    print("=" * 66)
    for s in range(num_sessions):
        session_id = f"k8s-session-{s + 1:03d}"
        result = subprocess.run(
            [
                "kubectl",
                "exec",
                "-n",
                NAMESPACE,
                "redis-0",
                "--",
                "redis-cli",
                "GET",
                session_id,
            ],
            capture_output=True,
            text=True,
        )
        raw = result.stdout.strip()
        ttl_result = subprocess.run(
            [
                "kubectl",
                "exec",
                "-n",
                NAMESPACE,
                "redis-0",
                "--",
                "redis-cli",
                "TTL",
                session_id,
            ],
            capture_output=True,
            text=True,
        )
        ttl = ttl_result.stdout.strip()

        if raw and raw != "(nil)":
            try:
                data = json.loads(raw)
                print(f"  {session_id}")
                print(f"    model_name : {data.get('model_name', '?')}")
                print(f"    route_name : {data.get('route_name', 'null')}")
                print(f"    TTL        : {ttl}s remaining")
            except json.JSONDecodeError:
                print(f"  {session_id}: (raw) {raw}")
        else:
            print(f"  {session_id}: key not found or expired")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--url",
        default=None,
        help="LoadBalancer URL to use instead of per-pod port-forwards. "
        "When set, cross-replica proof is skipped (no pod targeting).",
    )
    parser.add_argument(
        "--sessions", type=int, default=4, help="Number of sessions (default 4)"
    )
    parser.add_argument(
        "--rounds", type=int, default=4, help="Rounds per session (default 4)"
    )
    parser.add_argument(
        "--skip-redis-inspect", action="store_true", help="Skip Redis key inspection"
    )
    args = parser.parse_args()

    if args.url:
        # Simple mode: hit the LoadBalancer directly
        print(f"Mode: LoadBalancer ({args.url})")
        print()
        phase_loadbalancer(args.url, args.rounds)
        print("To get the full cross-replica proof, run without --url.")
        sys.exit(0)

    # Full mode: port-forward to each pod individually
    print("Mode: per-pod port-forward (full cross-replica proof)")
    print()

    try:
        pod_names = get_pod_names()
    except (subprocess.CalledProcessError, RuntimeError) as e:
        print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)

    if len(pod_names) < 2:
        print(f"WARNING: only {len(pod_names)} Plano pod(s) running.")
        print("  For a true cross-replica test you need at least 2.")
        print("  Scale up: kubectl scale deployment/plano --replicas=2 -n plano-demo")
        print()

    print(f"Found {len(pod_names)} Plano pod(s): {', '.join(pod_names)}")
    print("Opening per-pod port-forward tunnels...")
    print()

    pod_urls: dict[str, str] = {}
    contexts = []

    for i, pod in enumerate(pod_names):
        local_port = BASE_LOCAL_PORT + i
        ctx = port_forward(pod, local_port)
        url = ctx.__enter__()
        pod_urls[pod] = url
        contexts.append((ctx, url))
        print(f"  {pod} → localhost:{local_port}")

    print()

    try:
        passed = phase_cross_replica(pod_urls, args.sessions, args.rounds)

        if not args.skip_redis_inspect:
            phase_redis_inspect(args.sessions)

        print()
        print("=" * 66)
        print("Summary")
        print("=" * 66)
        if passed:
            print("All sessions were pinned consistently across replicas.")
            print("Redis session cache is working correctly in Kubernetes.")
        else:
            print("One or more sessions were NOT consistent across replicas.")
            print("Check brightstaff logs: kubectl logs -l app=plano -n plano-demo")

    finally:
        for ctx, _ in contexts:
            try:
                ctx.__exit__(None, None, None)
            except Exception:
                pass

    sys.exit(0 if passed else 1)


if __name__ == "__main__":
    main()
