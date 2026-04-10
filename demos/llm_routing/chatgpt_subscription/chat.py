#!/usr/bin/env python3
"""Interactive chat with a model through Plano using the OpenAI SDK."""

import sys
from openai import OpenAI

client = OpenAI(base_url="http://localhost:12000/v1", api_key="unused")


def run_chat(model):
    print(f"Chatting with {model} via Plano (Ctrl+C to quit)\n")
    history = []
    while True:
        try:
            user_input = input("you> ")
        except (KeyboardInterrupt, EOFError):
            print("\nbye")
            break
        if not user_input.strip():
            continue

        history.append({"role": "user", "content": user_input})

        stream = client.responses.create(model=model, input=history, stream=True)
        print(f"{model}> ", end="", flush=True)
        full = ""
        for event in stream:
            if event.type == "response.output_text.delta":
                print(event.delta, end="", flush=True)
                full += event.delta
        print()

        history.append({"role": "assistant", "content": full})


if __name__ == "__main__":
    model = sys.argv[1] if len(sys.argv) > 1 else "gpt-5.2"
    run_chat(model)
