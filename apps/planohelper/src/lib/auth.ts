/**
 * Check whether a Slack user is allowed to invoke privileged commands.
 *
 * `SLACK_ALLOWED_USER_IDS` is a comma-separated list of Slack user IDs
 * (e.g. `U01ABCDE,U02FGHIJ`). Empty or unset means no one is allowed.
 */
export function isAllowed(userId: string | undefined | null): boolean {
  if (!userId) return false;
  const raw = process.env.SLACK_ALLOWED_USER_IDS ?? "";
  const allowed = raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  return allowed.includes(userId);
}
