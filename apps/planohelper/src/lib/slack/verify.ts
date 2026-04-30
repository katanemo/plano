import { createHmac, timingSafeEqual } from "node:crypto";

const REPLAY_WINDOW_SECONDS = 60 * 5;

export type VerifyResult = { ok: true } | { ok: false; reason: string };

/**
 * Verify a Slack request signature.
 *
 * See https://api.slack.com/authentication/verifying-requests-from-slack
 *
 * Slack signs `v0:{timestamp}:{rawBody}` with HMAC-SHA256 using the signing
 * secret, hex-encoded and prefixed with `v0=`. We reject timestamps older than
 * 5 minutes to prevent replay attacks.
 */
export function verifySlackSignature(params: {
  signingSecret: string;
  timestamp: string | null;
  signature: string | null;
  rawBody: string;
  now?: number;
}): VerifyResult {
  const { signingSecret, timestamp, signature, rawBody } = params;

  if (!signingSecret) {
    return { ok: false, reason: "missing signing secret" };
  }
  if (!timestamp || !signature) {
    return { ok: false, reason: "missing signature headers" };
  }

  const tsNum = Number.parseInt(timestamp, 10);
  if (!Number.isFinite(tsNum)) {
    return { ok: false, reason: "invalid timestamp" };
  }

  const nowSeconds = Math.floor((params.now ?? Date.now()) / 1000);
  if (Math.abs(nowSeconds - tsNum) > REPLAY_WINDOW_SECONDS) {
    return { ok: false, reason: "stale timestamp" };
  }

  const base = `v0:${timestamp}:${rawBody}`;
  const digest = createHmac("sha256", signingSecret).update(base).digest("hex");
  const expected = `v0=${digest}`;

  const expectedBuf = Buffer.from(expected, "utf8");
  const actualBuf = Buffer.from(signature, "utf8");
  if (expectedBuf.length !== actualBuf.length) {
    return { ok: false, reason: "signature mismatch" };
  }
  if (!timingSafeEqual(expectedBuf, actualBuf)) {
    return { ok: false, reason: "signature mismatch" };
  }

  return { ok: true };
}
