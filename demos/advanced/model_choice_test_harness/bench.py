# bench.py
import json, time, yaml, statistics as stats
from pydantic import BaseModel, ValidationError
from openai import OpenAI

# Plano endpoint (keys are handled by Plano)
client = OpenAI(base_url="http://localhost:12000/v1", api_key="n/a")
MODELS = ["arch.summarize.v1", "arch.reason.v1"]
FIXTURES = "evals_summarize.yaml"


# Expected output shape
class SummarizeOut(BaseModel):
    title: str
    bullets: list[str]
    next_actions: list[str]


def load_fixtures(path):
    with open(path, "r") as f:
        return yaml.safe_load(f)["fixtures"]


def must_contain(text: str, anchors: list[str]) -> bool:
    t = text.lower()
    return all(a.lower() in t for a in anchors)


def schema_fmt(model: type[BaseModel]):
    return {"type": "json_object"}  # Simplified for broad compatibility


def run_case(model, fx):
    t0 = time.perf_counter()
    schema = SummarizeOut.model_json_schema()
    resp = client.chat.completions.create(
        model=model,
        messages=[
            {
                "role": "system",
                "content": f"Be concise. Output valid JSON matching this schema:\n{json.dumps(schema)}",
            },
            {"role": "user", "content": fx["input"]},
        ],
        response_format=schema_fmt(SummarizeOut),
    )
    dt = time.perf_counter() - t0

    content = resp.choices[0].message.content or "{}"
    passed, reasons = True, []

    try:
        data = json.loads(content)
    except:
        return {"ok": False, "lat": dt, "why": "json decode"}

    try:
        SummarizeOut(**data)
    except ValidationError:
        passed = False
        reasons.append("schema")
    if not must_contain(json.dumps(data), fx.get("must_include", [])):
        passed = False
        reasons.append("anchors")

    return {"ok": passed, "lat": dt, "why": ";".join(reasons)}


def main():
    fixtures = load_fixtures(FIXTURES)
    for model in MODELS:
        results = [run_case(model, fx) for fx in fixtures]
        ok = sum(r["ok"] for r in results)
        total = len(results)
        latencies = [r["lat"] for r in results]

        print(f"\n››› {model}")
        print(f"  Success: {ok}/{total} ({ok/total:.0%})")
        if latencies:
            avg_lat = stats.mean(latencies)
            p95_lat = stats.quantiles(latencies, n=100)[94]
            print(f"  Latency (ms): avg={avg_lat*1000:.0f}, p95={p95_lat*1000:.0f}")


if __name__ == "__main__":
    main()
