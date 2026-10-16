/**
 * Document language for `<html lang>`.
 *
 * `tiamud.v1` carries no content/server locale (docs/protocol/tiamud-v1.md:
 * `server.welcome` has only serverName, motd and sessionId), and this client
 * must not invent one. The shell, controls and reviewed Lantern Square prose
 * are authored in English, so `lang` must name the language the page actually
 * renders, not merely what the reader would prefer: declaring `es` over
 * English text makes screen readers mispronounce every word (WCAG 3.1.1).
 *
 * Resolution order:
 * 1. an explicit content locale, if a future protocol revision supplies one;
 * 2. the reader's ordered `navigator.languages`, matched against the locales
 *    this client actually ships (exact tag, then primary language);
 * 3. the authored fallback, English.
 *
 * With English as the only shipped UI locale, step 2 can only confirm `en`;
 * it starts choosing other tags once a translated shell exists.
 */
export const AUTHORED_LOCALE = 'en';
export const SHIPPED_UI_LOCALES: readonly string[] = [AUTHORED_LOCALE];

const MAX_CANDIDATES = 16;
const MAX_TAG_LENGTH = 35;
const TAG_PATTERN = /^[a-z]{2,3}(?:-[a-z0-9]{1,8})*$/i;

function canonical(tag: unknown): string | undefined {
  if (typeof tag !== 'string') return undefined;
  const trimmed = tag.trim();
  if (trimmed.length === 0 || trimmed.length > MAX_TAG_LENGTH || !TAG_PATTERN.test(trimmed)) {
    return undefined;
  }
  try {
    return Intl.getCanonicalLocales(trimmed)[0];
  } catch {
    return undefined;
  }
}

function match(tag: string, shipped: readonly string[]): string | undefined {
  const lower = tag.toLowerCase();
  const exact = shipped.find((candidate) => candidate.toLowerCase() === lower);
  if (exact) return exact;
  const primary = lower.split('-', 1)[0];
  return shipped.find((candidate) => candidate.toLowerCase().split('-', 1)[0] === primary);
}

export interface DocumentLanguageInput {
  contentLocale?: string;
  preferred?: readonly unknown[];
  shipped?: readonly string[];
}

export function resolveDocumentLanguage(input: DocumentLanguageInput = {}): string {
  const shipped = input.shipped ?? SHIPPED_UI_LOCALES;
  const content = canonical(input.contentLocale);
  if (content) return content;
  for (const candidate of (input.preferred ?? []).slice(0, MAX_CANDIDATES)) {
    const tag = canonical(candidate);
    const found = tag ? match(tag, shipped) : undefined;
    if (found) return found;
  }
  return AUTHORED_LOCALE;
}

export function browserPreferredLanguages(): readonly string[] {
  if (typeof navigator === 'undefined') return [];
  if (Array.isArray(navigator.languages) && navigator.languages.length > 0) {
    return navigator.languages;
  }
  return typeof navigator.language === 'string' ? [navigator.language] : [];
}

export function applyDocumentLanguage(
  root: { lang: string } | undefined = typeof document === 'undefined'
    ? undefined
    : document.documentElement,
  input: DocumentLanguageInput = { preferred: browserPreferredLanguages() },
): string {
  const language = resolveDocumentLanguage(input);
  if (root && root.lang !== language) root.lang = language;
  return language;
}
