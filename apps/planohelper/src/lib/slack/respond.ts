export type SlackResponseType = "ephemeral" | "in_channel";

export interface SlackBlock {
  type: string;
  [key: string]: unknown;
}

export interface SlackAck {
  response_type: SlackResponseType;
  text: string;
  blocks?: SlackBlock[];
  replace_original?: boolean;
}

/**
 * Post a delayed response to the `response_url` Slack included with a slash
 * command. `response_url` is valid for 30 minutes and accepts up to 5 POSTs.
 */
export async function postToResponseUrl(
  responseUrl: string,
  body: SlackAck,
): Promise<void> {
  const res = await fetch(responseUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    throw new Error(
      `Slack response_url ${responseUrl} returned ${res.status}: ${await res.text()}`,
    );
  }
}
