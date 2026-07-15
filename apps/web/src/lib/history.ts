const STORAGE_KEY = 'tiamud.command-history.v1';
export const MAX_STORED_COMMANDS = 100;

function validCommand(value: unknown): value is string {
  return typeof value === 'string' &&
    value.trim().length > 0 &&
    value.length <= 1024 &&
    !/[\r\n\u0000-\u001f\u007f]/.test(value);
}

export function loadCommandHistory(storage: Pick<Storage, 'getItem'> | undefined): string[] {
  try {
    const parsed: unknown = JSON.parse(storage?.getItem(STORAGE_KEY) ?? '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(validCommand).slice(-MAX_STORED_COMMANDS);
  } catch {
    return [];
  }
}

export function rememberCommand(
  history: readonly string[],
  command: string,
  storage?: Pick<Storage, 'setItem'>,
): string[] {
  const normalized = command.trim();
  if (!validCommand(normalized)) return [...history];
  const next = [...history.filter((entry) => entry !== normalized), normalized]
    .slice(-MAX_STORED_COMMANDS);
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // An in-memory history still works when storage is unavailable.
  }
  return next;
}
