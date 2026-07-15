import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import AccountAccess from './AccountAccess.svelte';

describe('AccountAccess', () => {
  it('renders an accessible login form without an unauthenticated enter button', () => {
    const { body } = render(AccountAccess, { props: { onselect: () => undefined } });

    expect(body).toContain('Return to your story');
    expect(body).toContain('for="account-name"');
    expect(body).toContain('autocomplete="username"');
    expect(body).toContain('type="password"');
    expect(body).toContain('Create account');
    expect(body).not.toContain('Enter Tiamud');
  });
});
