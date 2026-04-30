import type { SlashCommand } from "@/lib/commands/types";
import { updateProviders } from "@/lib/commands/update-providers";

const commands: SlashCommand[] = [updateProviders];

const byName = new Map<string, SlashCommand>(commands.map((c) => [c.name, c]));

export function getCommand(name: string): SlashCommand | undefined {
  return byName.get(name);
}

export function listCommands(): SlashCommand[] {
  return [...commands];
}
