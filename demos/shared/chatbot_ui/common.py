from datetime import datetime
import json
import logging
import os
import yaml
import gradio as gr
from typing import List, Optional, Tuple
from functools import partial

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(levelname)s - %(message)s",
)

log = logging.getLogger(__name__)

GRADIO_CSS_STYLE = """
.json-container {
    height: 80vh !important;
    overflow-y: auto !important;
}
.chatbot {
    height: calc(80vh - 100px) !important;
    overflow-y: auto !important;
}
footer {visibility: hidden}
"""


def chat(
    query: Optional[str],
    conversation: Optional[List[Tuple[str, str]]],
    history: List[dict],
    client,
):
    history.append({"role": "user", "content": query})

    try:
        response = client.chat.completions.create(
            # we select model from plano_config file
            model="None",
            messages=history,
            temperature=1.0,
            stream=True,
        )
    except Exception as e:
        # remove last user message in case of exception
        history.pop()
        log.info("Error calling gateway API: {}".format(e))
        raise gr.Error("Error calling gateway API: {}".format(e))

    conversation.append((query, ""))

    for chunk in response:
        tokens = process_stream_chunk(chunk, history)
        if tokens:
            conversation[-1] = (
                conversation[-1][0],
                conversation[-1][1] + tokens,
            )

            yield "", conversation, history


def create_gradio_app(demo_description, client):
    with gr.Blocks(
        theme=gr.themes.Default(
            font_mono=[gr.themes.GoogleFont("IBM Plex Mono"), "Arial", "sans-serif"]
        ),
        fill_height=True,
        css=GRADIO_CSS_STYLE,
    ) as demo:
        with gr.Row(equal_height=True):
            history = gr.State([])

            with gr.Column(scale=1):
                gr.Markdown(demo_description),
                with gr.Accordion("Available Tools/APIs", open=True):
                    with gr.Column(scale=1):
                        gr.JSON(
                            value=get_prompt_targets(),
                            show_indices=False,
                            elem_classes="json-container",
                            min_height="80vh",
                        )

            with gr.Column(scale=2):
                chatbot = gr.Chatbot(
                    label="Plano Chatbot",
                    elem_classes="chatbot",
                )
                textbox = gr.Textbox(
                    show_label=False,
                    placeholder="Enter text and press enter",
                    autofocus=True,
                    elem_classes="textbox",
                )
            chat_with_client = partial(chat, client=client)

            textbox.submit(
                chat_with_client,
                [textbox, chatbot, history],
                [textbox, chatbot, history],
            )

    return demo


def process_stream_chunk(chunk, history):
    delta = chunk.choices[0].delta
    if delta.role and delta.role != history[-1]["role"]:
        # create new history item if role changes
        # this is likely due to Plano tool call and api response
        history.append({"role": delta.role})

    history[-1]["model"] = chunk.model
    # append tool calls to history if there are any in the chunk
    if delta.tool_calls:
        history[-1]["tool_calls"] = delta.tool_calls

    if delta.content:
        # append content to the last history item
        if history[-1]["model"] != "Arch-Function-Chat":
            history[-1]["content"] = history[-1].get("content", "") + delta.content
        # yield content if it is from assistant
        if history[-1]["model"] == "Arch-Function":
            return None
        if history[-1]["role"] == "assistant":
            return delta.content

    return None


def convert_prompt_target_to_openai_format(target):
    tool = {
        "description": target["description"],
        "parameters": {"type": "object", "properties": {}, "required": []},
    }

    if "parameters" in target:
        for param_info in target["parameters"]:
            parameter = {
                "type": param_info["type"],
                "description": param_info["description"],
            }

            for key in ["default", "format", "enum", "items", "minimum", "maximum"]:
                if key in param_info:
                    parameter[key] = param_info[key]

            tool["parameters"]["properties"][param_info["name"]] = parameter

            required = param_info.get("required", False)
            if required:
                tool["parameters"]["required"].append(param_info["name"])

    return {"name": target["name"], "info": tool}


def get_prompt_targets():
    try:
        with open(os.getenv("PLANO_CONFIG", "config.yaml"), "r") as file:
            config = yaml.safe_load(file)

            available_tools = []
            if "prompt_targets" in config:
                for target in config["prompt_targets"]:
                    if not target.get("default", False):
                        available_tools.append(
                            convert_prompt_target_to_openai_format(target)
                        )

                return {tool["name"]: tool["info"] for tool in available_tools}
            elif "llm_providers" in config:
                return config["llm_providers"]

    except Exception as e:
        log.info(e)
        return None


def get_llm_models():
    try:
        with open(os.getenv("PLANO_CONFIG", "config.yaml"), "r") as file:
            config = yaml.safe_load(file)

            available_models = [""]
            default_llm = None
            for llm_providers in config["llm_providers"]:
                if llm_providers.get("default", False):
                    default_llm = llm_providers["name"]
                else:
                    available_models.append(llm_providers["name"])

            # place default model at the beginning of the list
            if default_llm:
                available_models.insert(0, default_llm)
            return available_models
    except Exception as e:
        log.info(e)
        return []


def format_log(message):
    time_now = datetime.now().strftime("%Y-%m-%d %H:%M:%S,%f")[:-3]
    return f"{time_now} - {message}"
