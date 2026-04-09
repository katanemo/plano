import uuid

from openai import OpenAI

client = OpenAI(base_url="http://localhost:12000/v1", api_key="EMPTY")


def chat_with_affinity(messages: list[dict[str, str]], affinity_id: str):
    # Intentionally matches the OpenAI SDK usage expected by this demo.
    response = client.chat.completions.create(
        model="gpt-5.2",
        messages=messages,
        extra_headers={"X-Model-Affinity": affinity_id},
    )
    return response


def show(label: str, response):
    content = response.choices[0].message.content or ""
    print(f"{label}")
    print(f"  model: {response.model}")
    print(f"  text : {content[:120].replace(chr(10), ' ')}")
    print()


def main():
    affinity_id = str(uuid.uuid4())
    print("== Affinity Demo (OpenAI SDK) ==")
    print(f"affinity id: {affinity_id}")
    print()

    code_messages = [
        {"role": "user", "content": "Write Python code for binary search."},
    ]
    reasoning_messages = [
        {
            "role": "user",
            "content": "Explain whether free will can exist with determinism.",
        },
    ]

    first = chat_with_affinity(code_messages, affinity_id)
    show("1) first call (new affinity, routes and caches)", first)

    second = chat_with_affinity(reasoning_messages, affinity_id)
    show("2) second call (same affinity, should stay pinned)", second)

    new_affinity_id = str(uuid.uuid4())
    third = chat_with_affinity(reasoning_messages, new_affinity_id)
    show("3) third call (new affinity, fresh routing)", third)

    print("If 1 and 2 use the same model, affinity pinning is working.")


if __name__ == "__main__":
    main()
