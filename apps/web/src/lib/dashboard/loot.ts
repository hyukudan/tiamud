export type LootDistributionMode = 'round_robin' | 'need_greed' | 'master_loot';

export interface LootRootNeedGreed {
  ref: string;
  name: string;
  need: number;
  greed: number;
  pass: number;
  pending: number;
}

export interface LootRootMaster {
  ref: string;
  name: string;
  assignedTo?: string;
}

export interface LootRootPlain {
  ref: string;
  name: string;
}

export interface LootRollView {
  npcName: string;
  mode: LootDistributionMode;
  empty: boolean;
  roots: Array<LootRootNeedGreed | LootRootMaster | LootRootPlain>;
}

export const LOOT_MODE_LABELS: Record<LootDistributionMode, string> = {
  round_robin: 'Round-robin',
  need_greed: 'Need / Greed',
  master_loot: 'Master-loot',
};

const MODE_LABEL_TO_MODE: Record<string, LootDistributionMode> = {
  'round-robin': 'round_robin',
  'need/greed': 'need_greed',
  'master-loot': 'master_loot',
};

const HEADER_PATTERN = /^Group loot for (.+) \(mode: (round-robin|need\/greed|master-loot)\):$/;
const EMPTY_LINE = '- (no carried loot to divide)';
const ROOT_PATTERN = /^- (.+?) \(ref (o[0-9a-f]{16})\)(?::\s*(.*))?$/;
const NEED_GREED_DETAIL_PATTERN = /^(\d+) need, (\d+) greed, (\d+) pass, (\d+) pending$/;
const ASSIGNED_DETAIL_PATTERN = /^assigned to (.+)$/;

const splitLines = (message: string): string[] => message.split(/\r\n|\r|\n/);

export function isLootRollProjection(message: string): boolean {
  const lines = splitLines(message);
  const firstNonEmpty = lines.find((line) => line.trim().length > 0);
  if (firstNonEmpty === undefined) return false;
  return HEADER_PATTERN.test(firstNonEmpty.trimEnd());
}

export function parseLootRoll(message: string): LootRollView | undefined {
  const lines = splitLines(message);
  const headerIndex = lines.findIndex((line) => line.trim().length > 0);
  if (headerIndex === -1) return undefined;

  const headerMatch = HEADER_PATTERN.exec(lines[headerIndex].trimEnd());
  if (!headerMatch) return undefined;

  const npcName = headerMatch[1];
  const mode = MODE_LABEL_TO_MODE[headerMatch[2]];
  if (!mode) return undefined;

  const roots: Array<LootRootNeedGreed | LootRootMaster | LootRootPlain> = [];
  let empty = false;

  for (let i = headerIndex + 1; i < lines.length; i += 1) {
    const line = lines[i].replace(/\s+$/, '');
    if (line.length === 0) continue;

    if (line === EMPTY_LINE) {
      empty = true;
      continue;
    }

    const rootMatch = ROOT_PATTERN.exec(line);
    if (!rootMatch) continue;

    const name = rootMatch[1];
    const ref = rootMatch[2];
    const detail = rootMatch[3];

    if (mode === 'need_greed') {
      const detailMatch = detail !== undefined ? NEED_GREED_DETAIL_PATTERN.exec(detail) : null;
      roots.push({
        ref,
        name,
        need: detailMatch ? Number(detailMatch[1]) : 0,
        greed: detailMatch ? Number(detailMatch[2]) : 0,
        pass: detailMatch ? Number(detailMatch[3]) : 0,
        pending: detailMatch ? Number(detailMatch[4]) : 0,
      });
    } else if (mode === 'master_loot') {
      const assignedMatch = detail !== undefined ? ASSIGNED_DETAIL_PATTERN.exec(detail) : null;
      roots.push({
        ref,
        name,
        assignedTo: assignedMatch ? assignedMatch[1] : undefined,
      });
    } else {
      roots.push({ ref, name });
    }
  }

  return { npcName, mode, empty, roots: empty ? [] : roots };
}

export const lootRollCommand = (): string => 'loot roll';
export const lootModeCommand = (mode: LootDistributionMode): string => `loot mode ${mode}`;
export const needCommand = (ref: string): string => `need ${ref}`;
export const greedCommand = (ref: string): string => `greed ${ref}`;
export const passCommand = (ref: string): string => `pass ${ref}`;

export function lootMasterCommand(ref: string, member: string): string {
  return `loot master ${ref} to ${member.trim()}`;
}

const CONTROL_CHAR_PATTERN = /[\x00-\x1f\x7f]/;

export function lootMasterMemberValid(member: string): boolean {
  if (CONTROL_CHAR_PATTERN.test(member)) return false;
  const trimmed = member.trim();
  return trimmed.length >= 1 && trimmed.length <= 64;
}

export const rootIsNeedGreed = (
  root: LootRootNeedGreed | LootRootMaster | LootRootPlain,
): root is LootRootNeedGreed => 'need' in root;

export const rootIsMaster = (
  root: LootRootNeedGreed | LootRootMaster | LootRootPlain,
): root is LootRootMaster => 'assignedTo' in root;
