import type { SlackAck } from "@/lib/slack/respond";

/**
 * Parsed Slack slash command payload.
 *
 * See https://api.slack.com/interactivity/slash-commands#app_command_handling
 */
export interface SlashCommandContext {
  command: string;
  text: string;
  userId: string;
  userName: string;
  channelId: string;
  channelName: string;
  teamId: string;
  responseUrl: string;
  triggerId: string;
}

export interface SlashCommand {
  name: string;
  description: string;
  handler: (ctx: SlashCommandContext) => Promise<SlackAck>;
}
