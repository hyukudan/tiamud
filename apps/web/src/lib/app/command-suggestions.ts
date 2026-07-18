const BASE_COMMAND_SUGGESTIONS = [
  'look',
  'inventory',
  'loot',
  'north',
  'east',
  'south',
  'west',
  'up',
  'down',
  'say ',
  'tell ',
  'who',
  'help',
  'ignore ',
  'unignore ',
  'block ',
  'unblock ',
  'report ',
] as const;

export function computeCommandSuggestions(
  exitCommands: readonly string[],
  rememberedCommands: readonly string[],
): string[] {
  const commands = [
    ...BASE_COMMAND_SUGGESTIONS,
    ...exitCommands,
    ...rememberedCommands.toReversed(),
  ];
  return [...new Set(commands)].slice(0, 24);
}
