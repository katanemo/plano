import json


ARCH_STATE_HEADER = "x-arch-state"


def get_data_chunks(stream, n=1):
    chunks = []
    for chunk in stream.iter_lines():
        if chunk:
            chunk = chunk.decode("utf-8")
            chunk_data_id = chunk[0:6]
            assert chunk_data_id == "data: "
            chunk_data = chunk[6:]
            chunk_data = chunk_data.strip()
            chunks.append(chunk_data)
            if len(chunks) >= n:
                break
    return chunks


def get_plano_messages(response_json):
    plano_messages = []
    if response_json and "metadata" in response_json:
        # load plano_state from metadata
        plano_state_str = response_json.get("metadata", {}).get(ARCH_STATE_HEADER, "{}")
        # parse plano_state into json object
        plano_state = json.loads(plano_state_str)
        # load messages from plano_state
        plano_messages_str = plano_state.get("messages", "[]")
        # parse messages into json object
        plano_messages = json.loads(plano_messages_str)
        # append messages from plano gateway to history
        return plano_messages
    return []
