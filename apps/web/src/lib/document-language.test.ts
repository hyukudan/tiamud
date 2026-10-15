import { describe, expect, it } from 'vitest';
import {
  AUTHORED_LOCALE,
  applyDocumentLanguage,
  resolveDocumentLanguage,
} from './document-language';

describe('document language', () => {
  it('declares the authored English shell while no translated UI ships', () => {
    expect(resolveDocumentLanguage()).toBe(AUTHORED_LOCALE);
    expect(resolveDocumentLanguage({ preferred: ['es-ES', 'fr'] })).toBe('en');
    expect(resolveDocumentLanguage({ preferred: ['en-GB'] })).toBe('en');
  });

  it('follows reader preference order across shipped locales', () => {
    const shipped = ['en', 'es', 'pt-BR'];
    expect(resolveDocumentLanguage({ preferred: ['de', 'es-MX', 'en'], shipped })).toBe('es');
    expect(resolveDocumentLanguage({ preferred: ['pt-br'], shipped })).toBe('pt-BR');
    expect(resolveDocumentLanguage({ preferred: ['pt-PT'], shipped })).toBe('pt-BR');
    expect(resolveDocumentLanguage({ preferred: ['ja'], shipped })).toBe('en');
  });

  it('prefers an explicit content locale over reader preference', () => {
    expect(resolveDocumentLanguage({ contentLocale: 'es-es', preferred: ['en'] })).toBe('es-ES');
  });

  it('ignores malformed, oversized and non-string tags and bounds the candidate list', () => {
    expect(resolveDocumentLanguage({
      contentLocale: 'x'.repeat(80),
      preferred: [42, '', '../../etc', 'en_US', `es-${'a'.repeat(40)}`],
      shipped: ['en', 'es'],
    })).toBe('en');
    const many = [...Array.from({ length: 16 }, () => 'zz'), 'es'];
    expect(resolveDocumentLanguage({ preferred: many, shipped: ['en', 'es'] })).toBe('en');
  });

  it('writes the resolved tag onto the root element', () => {
    const root = { lang: 'xx' };
    expect(applyDocumentLanguage(root, { preferred: ['es'] })).toBe('en');
    expect(root.lang).toBe('en');
  });
});
