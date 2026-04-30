import type { SlashCommand } from "@/lib/commands/types";
import { dispatchWorkflow } from "@/lib/github";

export const updateProviders: SlashCommand = {
  name: "/update-providers",
  description: "Refresh provider_models.yaml and open a PR.",
  async handler(ctx) {
    await dispatchWorkflow("update-providers", {
      response_url: ctx.responseUrl,
      user_id: ctx.userId,
      user_name: ctx.userName,
      channel_id: ctx.channelId,
    });

    return {
      response_type: "ephemeral",
      text: "Ok - I'm updating provider_models.yaml and will reply here with the PR link when it's ready.",
      blocks: [
        {
          type: "section",
          text: {
            type: "mrkdwn",
            text: ":hourglass_flowing_sand: Kicking off provider model refresh. I'll reply here with the PR link when it's ready.",
          },
        },
      ],
    };
  },
};
