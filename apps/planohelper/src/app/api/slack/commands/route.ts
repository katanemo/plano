import { NextResponse } from "next/server";
import { isAllowed } from "@/lib/auth";
import { getCommand } from "@/lib/commands/registry";
import type { SlashCommandContext } from "@/lib/commands/types";
import { verifySlackSignature } from "@/lib/slack/verify";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

function ephemeral(text: string) {
  return NextResponse.json({ response_type: "ephemeral", text });
}

export async function POST(req: Request) {
  const rawBody = await req.text();

  const signingSecret = process.env.SLACK_SIGNING_SECRET ?? "";
  const verification = verifySlackSignature({
    signingSecret,
    timestamp: req.headers.get("x-slack-request-timestamp"),
    signature: req.headers.get("x-slack-signature"),
    rawBody,
  });
  if (!verification.ok) {
    return new NextResponse(`Unauthorized: ${verification.reason}`, {
      status: 401,
    });
  }

  const params = new URLSearchParams(rawBody);
  const ctx: SlashCommandContext = {
    command: params.get("command") ?? "",
    text: params.get("text") ?? "",
    userId: params.get("user_id") ?? "",
    userName: params.get("user_name") ?? "",
    channelId: params.get("channel_id") ?? "",
    channelName: params.get("channel_name") ?? "",
    teamId: params.get("team_id") ?? "",
    responseUrl: params.get("response_url") ?? "",
    triggerId: params.get("trigger_id") ?? "",
  };

  if (!ctx.command) {
    return ephemeral("Missing `command` field.");
  }

  if (!isAllowed(ctx.userId)) {
    return ephemeral(
      `Sorry, you aren't on the allowlist for PlanoHelper commands. Ask an admin to add \`${ctx.userId}\` to \`SLACK_ALLOWED_USER_IDS\`.`,
    );
  }

  const cmd = getCommand(ctx.command);
  if (!cmd) {
    return ephemeral(`Unknown command \`${ctx.command}\`.`);
  }

  try {
    const ack = await cmd.handler(ctx);
    return NextResponse.json(ack);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[planohelper] ${ctx.command} failed:`, err);
    return ephemeral(`:warning: Failed to run \`${ctx.command}\`: ${msg}`);
  }
}
