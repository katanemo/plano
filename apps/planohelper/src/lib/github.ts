/**
 * Dispatch a `repository_dispatch` event to the configured GitHub repo.
 *
 * See https://docs.github.com/en/rest/repos/repos#create-a-repository-dispatch-event
 *
 * Requires a PAT or fine-grained token with `actions:write` on the target repo,
 * available in `GITHUB_TOKEN`. `GITHUB_REPO` is `owner/repo`.
 */
export async function dispatchWorkflow(
  eventType: string,
  clientPayload: Record<string, unknown>,
): Promise<void> {
  const token = process.env.GITHUB_TOKEN;
  const repo = process.env.GITHUB_REPO;
  if (!token) throw new Error("GITHUB_TOKEN is not set");
  if (!repo) throw new Error("GITHUB_REPO is not set");

  const res = await fetch(`https://api.github.com/repos/${repo}/dispatches`, {
    method: "POST",
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
      "Content-Type": "application/json",
      "User-Agent": "planohelper",
    },
    body: JSON.stringify({
      event_type: eventType,
      client_payload: clientPayload,
    }),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(
      `GitHub dispatch ${eventType} failed (${res.status}): ${body}`,
    );
  }
}
