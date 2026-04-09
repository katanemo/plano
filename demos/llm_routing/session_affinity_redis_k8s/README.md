# Session Affinity — Multi-Replica Kubernetes Deployment

Production-style Kubernetes demo that proves Redis-backed session affinity
(`X-Model-Affinity`) works correctly when Plano runs as multiple replicas
behind a load balancer.

## Architecture

```
                     ┌─────────────────────────────────────────┐
                     │           Kubernetes Cluster             │
                     │                                          │
   Client ──────────►│  LoadBalancer Service (port 12000)       │
                     │       │               │                  │
                     │  ┌────▼────┐    ┌─────▼───┐             │
                     │  │ Plano   │    │ Plano   │  (replicas) │
                     │  │ Pod 0   │    │ Pod 1   │             │
                     │  └────┬────┘    └────┬────┘             │
                     │       └──────┬───────┘                  │
                     │         ┌────▼────┐                     │
                     │         │  Redis  │ (StatefulSet)        │
                     │         │ Pod     │ shared session store │
                     │         └─────────┘                     │
                     │                                          │
                     │  ┌──────────┐                           │
                     │  │  Jaeger  │  distributed tracing       │
                     │  └──────────┘                           │
                     └─────────────────────────────────────────┘
```

**What makes this production-like:**

| Feature | Detail |
|---------|--------|
| 2 Plano replicas | `replicas: 2` with HPA (scales 2–5 on CPU) |
| Shared Redis | StatefulSet with PVC — sessions survive pod restarts |
| Session TTL | 600 s, enforced natively by Redis `EX` |
| Eviction policy | `allkeys-lru` — Redis auto-evicts oldest sessions under memory pressure |
| Distributed tracing | Jaeger collects spans from both pods |
| Health probes | Readiness + liveness gates traffic away from unhealthy pods |

## Quick Start (local — no registry needed)

```bash
# 1. Install kind if needed
#    https://kind.sigs.k8s.io/docs/user/quick-start/#installation
#    brew install kind   (macOS)

# 2. Set your API key
export OPENAI_API_KEY=sk-...
# or copy and edit:
cp .env.example .env

# 3. Build, deploy, and verify in one command
./run-local.sh
```

`run-local.sh` creates a kind cluster named `plano-demo` (if it doesn't exist),
builds the image locally, loads it into the cluster with `kind load docker-image`
— **no registry, no push required**.

Individual steps:

```bash
./run-local.sh --build-only      # (re-)build and reload image into kind
./run-local.sh --deploy-only     # (re-)apply k8s manifests
./run-local.sh --verify          # run verify_affinity.py
./run-local.sh --down            # delete k8s resources (keeps kind cluster)
./run-local.sh --delete-cluster  # delete k8s resources + kind cluster
```

---

## Prerequisites

| Tool | Notes |
|------|-------|
| `kubectl` | Configured to reach a Kubernetes cluster |
| `docker` | To build and push the custom image |
| Container registry (optional) | Needed only when you are not using the local kind flow |
| `OPENAI_API_KEY` | For model inference |
| Python 3.11+ | Only for `verify_affinity.py` |

**Cluster:** `run-local.sh` creates and manages a kind cluster named `plano-demo` automatically. Install kind from https://kind.sigs.k8s.io or `brew install kind`.

## Step 1 — Build the Image

Build a custom image from the repo root:

```bash
# From this demo directory:
./build-and-push.sh ghcr.io/yourorg/plano-redis:latest

# Or manually from the repo root:
docker build \
  -f demos/llm_routing/session_affinity_redis_k8s/Dockerfile \
  -t ghcr.io/yourorg/plano-redis:latest \
  .
docker push ghcr.io/yourorg/plano-redis:latest
```

Then update the image reference in `k8s/plano.yaml` (skip this when using `run-local.sh`, which uses `plano-redis:local` automatically):

```yaml
image: ghcr.io/yourorg/plano-redis:latest  # ← replace YOUR_REGISTRY/plano-redis:latest
```

## Step 2 — Deploy

```bash
./deploy.sh
```

The script:
1. Creates the `plano-demo` namespace
2. Prompts for `OPENAI_API_KEY` and creates a Kubernetes Secret
3. Applies Redis, Jaeger, ConfigMap, and Plano manifests in order
4. Waits for rollouts to complete

Expected output:

```
==> Applying namespace...
==> Creating API key secret...
  OPENAI_API_KEY: [hidden]
==> Applying Redis (StatefulSet + Services)...
==> Applying Jaeger...
==> Applying Plano config (ConfigMap)...
==> Applying Plano deployment + HPA...
==> Waiting for Redis to be ready...
==> Waiting for Plano pods to be ready...

Deployment complete!

=== Pods ===
NAME                     READY   STATUS    NODE
redis-0                  1/1     Running   node-1
plano-6d8f9b-xk2pq       1/1     Running   node-1
plano-6d8f9b-r7nlw       1/1     Running   node-2
jaeger-5c7d8f-q9mnb      1/1     Running   node-1

=== Services ===
NAME      TYPE           CLUSTER-IP     EXTERNAL-IP    PORT(S)
plano     LoadBalancer   10.96.12.50    203.0.113.42   12000:32000/TCP
redis     ClusterIP      None           <none>          6379/TCP
jaeger    ClusterIP      10.96.8.71     <none>          16686/TCP,...
```

## Step 3 — Verify Session Affinity Across Replicas

```bash
python verify_affinity.py
```

The script opens a dedicated `kubectl port-forward` tunnel to **each pod
individually**. This is the definitive test: it routes requests to specific
pods rather than relying on random load-balancer assignment.

```
Mode: per-pod port-forward (full cross-replica proof)

Found 2 Plano pod(s): plano-6d8f9b-xk2pq, plano-6d8f9b-r7nlw
Opening per-pod port-forward tunnels...

  plano-6d8f9b-xk2pq → localhost:19100
  plano-6d8f9b-r7nlw → localhost:19101

==================================================================
Phase 1: Cross-replica session pinning
  Pods under test : plano-6d8f9b-xk2pq, plano-6d8f9b-r7nlw
  Sessions        : 4
  Rounds/session  : 4

  Each session is PINNED via one pod and VERIFIED via another.
  If Redis is shared, every round must return the same model.
==================================================================

  PASS  k8s-session-001
        model      : gpt-4o-mini-2024-07-18
        pod order  : plano-6d8f9b-xk2pq → plano-6d8f9b-r7nlw → plano-6d8f9b-xk2pq → plano-6d8f9b-r7nlw

  PASS  k8s-session-002
        model      : gpt-5.2
        pod order  : plano-6d8f9b-r7nlw → plano-6d8f9b-xk2pq → plano-6d8f9b-r7nlw → plano-6d8f9b-xk2pq

  PASS  k8s-session-003
        model      : gpt-4o-mini-2024-07-18
        pod order  : plano-6d8f9b-xk2pq → plano-6d8f9b-r7nlw → plano-6d8f9b-xk2pq → plano-6d8f9b-r7nlw

  PASS  k8s-session-004
        model      : gpt-5.2
        pod order  : plano-6d8f9b-r7nlw → plano-6d8f9b-xk2pq → plano-6d8f9b-r7nlw → plano-6d8f9b-xk2pq

==================================================================
Phase 2: Redis key inspection
==================================================================
  k8s-session-001
    model_name : gpt-4o-mini-2024-07-18
    route_name : fast_responses
    TTL        : 587s remaining
  k8s-session-002
    model_name : gpt-5.2
    route_name : deep_reasoning
    TTL        : 581s remaining
  ...

==================================================================
Summary
==================================================================
All sessions were pinned consistently across replicas.
Redis session cache is working correctly in Kubernetes.
```

## What to Look For

### The cross-replica proof

Each session's `pod order` line shows it alternating between the two pods:

```
pod order: pod-A → pod-B → pod-A → pod-B
```

Round 1 sets the Redis key (via pod-A). Rounds 2, 3, 4 read from Redis on
alternating pods. If the model stays the same across all rounds, Redis is the
shared source of truth — **not** any in-process state.

### Redis keys

```bash
kubectl exec -it redis-0 -n plano-demo -- redis-cli

127.0.0.1:6379> KEYS *
1) "k8s-session-001"
2) "k8s-session-002"

127.0.0.1:6379> GET k8s-session-001
{"model_name":"gpt-4o-mini-2024-07-18","route_name":"fast_responses"}

127.0.0.1:6379> TTL k8s-session-001
(integer) 587
```

### Jaeger traces

```bash
kubectl port-forward svc/jaeger 16686:16686 -n plano-demo
```

Open **http://localhost:16686**, select service `plano`.

- **Pinned requests** — no span to the Arch-Router (decision served from Redis)
- **First request** per session — spans include the router call + a Redis `SET`
- Both Plano pods appear as separate instances in the trace list

### Scaling up (HPA in action)

```bash
# Scale to 3 replicas manually
kubectl scale deployment/plano --replicas=3 -n plano-demo

# Run verification again — now 3 pods alternate
python verify_affinity.py --sessions 6
```

Existing sessions in Redis are unaffected by the scale event. New pods
immediately participate in the shared session pool.

## Teardown

```bash
./deploy.sh --destroy
# Then optionally:
kubectl delete namespace plano-demo
```

## Notes

- The Redis StatefulSet uses a `PersistentVolumeClaim`. Session data survives
  pod restarts within a TTL window but is not HA. For production, replace with
  Redis Sentinel, Redis Cluster, or a managed service (ElastiCache, MemoryStore).
- `session_max_entries` is not enforced by this backend — Redis uses
  `maxmemory-policy: allkeys-lru` instead, which is a global limit across all
  keys rather than a per-application cap.
- On **minikube**, run `minikube tunnel` in a separate terminal to get an
  external IP for the LoadBalancer service.
- On **kind**, switch to `NodePort` (see the comment in `k8s/plano.yaml`).
