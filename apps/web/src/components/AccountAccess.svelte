<script lang="ts">
  import { onMount } from 'svelte';
  import {
    authenticate,
    clearStoredAccess,
    createCharacter,
    loadArchetypes,
    loadCharacters,
    logout,
    readStoredAccess,
    writeStoredAccess,
    type AuthenticatedSession,
    type ArchetypeChoice,
    type Character,
    type CharacterSelection,
  } from '../lib/auth';

  let { onselect }: { onselect: (selection: CharacterSelection) => void } = $props();

  let mode = $state<'login' | 'register'>('login');
  let accountName = $state('');
  let password = $state('');
  let characterName = $state('');
  let archetypes = $state<ArchetypeChoice[]>([]);
  let archetypeId = $state('');
  let session = $state<AuthenticatedSession>();
  let characters = $state<Character[]>([]);
  let lastCharacterId = $state<number>();
  let busy = $state(false);
  let restoring = $state(false);
  let error = $state<string>();

  const submitLabel = $derived(mode === 'login' ? 'Sign in' : 'Create account');
  const selectedArchetype = $derived(archetypes.find((archetype) => archetype.id === archetypeId));

  async function refreshRoster(current: AuthenticatedSession): Promise<void> {
    const [nextCharacters, archetypeResult] = await Promise.all([
      loadCharacters(current.token),
      loadArchetypes()
        .then((value) => ({ value }))
        .catch((cause: unknown) => ({ cause })),
    ]);
    characters = nextCharacters;
    if ('cause' in archetypeResult) {
      archetypes = [];
      archetypeId = '';
      error = archetypeResult.cause instanceof Error
        ? archetypeResult.cause.message
        : 'The archetype catalog is unavailable.';
      return;
    }
    const nextArchetypes = archetypeResult.value;
    archetypes = nextArchetypes;
    archetypeId = archetypeId && nextArchetypes.some((archetype) => archetype.id === archetypeId)
      ? archetypeId
      : '';
  }

  async function restore(): Promise<void> {
    restoring = true;
    const stored = readStoredAccess(window.localStorage);
    if (!stored) {
      restoring = false;
      return;
    }
    try {
      session = stored;
      lastCharacterId = stored.characterId;
      await refreshRoster(stored);
    } catch (cause) {
      session = undefined;
      clearStoredAccess(window.localStorage);
      error = cause instanceof Error ? cause.message : 'Your saved session could not be restored.';
    } finally {
      restoring = false;
    }
  }

  async function submitAccess(): Promise<void> {
    busy = true;
    error = undefined;
    try {
      const authenticated = await authenticate(mode, accountName.trim(), password);
      session = authenticated;
      password = '';
      lastCharacterId = undefined;
      writeStoredAccess(window.localStorage, authenticated);
      await refreshRoster(authenticated);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'Tiamud could not complete the request.';
    } finally {
      busy = false;
    }
  }

  async function submitCharacter(): Promise<void> {
    if (!session) return;
    busy = true;
    error = undefined;
    try {
      const character = await createCharacter(
        session.token,
        characterName.trim(),
        archetypeId || undefined,
      );
      characters = [...characters, character];
      characterName = '';
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'The character could not be created.';
    } finally {
      busy = false;
    }
  }

  function selectCharacter(character: Character): void {
    if (!session) return;
    lastCharacterId = character.id;
    writeStoredAccess(window.localStorage, { ...session, characterId: character.id });
    onselect({ ...session, characterId: character.id, characterName: character.name });
  }

  async function signOut(): Promise<void> {
    const token = session?.token;
    busy = true;
    error = undefined;
    clearStoredAccess(window.localStorage);
    session = undefined;
    characters = [];
    archetypes = [];
    archetypeId = '';
    lastCharacterId = undefined;
    try {
      if (token) await logout(token);
    } catch {
      // The local credential is removed even if an expired session cannot be revoked.
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void restore();
  });
</script>

{#if restoring}
  <p class="access-status" role="status">Restoring your passage…</p>
{:else if session}
  <section class="roster" aria-labelledby="roster-title">
    <div class="access-heading">
      <div>
        <p class="access-eyebrow">Signed in as {session.account.name}</p>
        <h2 id="roster-title">Choose your adventurer</h2>
      </div>
      <button class="text-button" type="button" disabled={busy} onclick={signOut}>Sign out</button>
    </div>

    {#if characters.length > 0}
      <ul class="character-list">
        {#each characters as character (character.id)}
          <li>
            <span>{character.name}{#if character.archetype}<small>{character.archetype.label}</small>{/if}</span>
            <button type="button" disabled={busy} onclick={() => selectCharacter(character)}>
              {lastCharacterId === character.id ? 'Continue' : 'Enter'}
              <span class="sr-only"> as {character.name}</span>
              <span aria-hidden="true">→</span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="access-status">Your story needs its first adventurer.</p>
    {/if}

    <form
      class="character-form"
      onsubmit={(event) => {
        event.preventDefault();
        void submitCharacter();
      }}
    >
      <label for="character-name">Create an adventurer</label>
      <div class="character-name-row">
        <input
          id="character-name"
          bind:value={characterName}
          autocomplete="off"
          maxlength="40"
          required
          placeholder="Character name"
        />
      </div>
      {#if archetypes.length > 0}
        <label for="character-archetype">Archetype</label>
        <select
          id="character-archetype"
          bind:value={archetypeId}
          disabled={busy}
          aria-describedby="archetype-description"
        >
          <option value="">World default</option>
          {#each archetypes as archetype (archetype.id)}
            <option value={archetype.id}>{archetype.label}</option>
          {/each}
        </select>
        {#if selectedArchetype}
          <p id="archetype-description">{selectedArchetype.description}</p>
        {:else}
          <p id="archetype-description">Use the world author's suggested starting path.</p>
        {/if}
      {/if}
      <button
        class="character-create"
        type="submit"
        disabled={busy || !characterName.trim()}
      >Create</button>
    </form>
  </section>
{:else}
  <section class="account-access" aria-labelledby="access-title">
    <div class="access-heading">
      <div>
        <p class="access-eyebrow">Account access</p>
        <h2 id="access-title">{mode === 'login' ? 'Return to your story' : 'Begin a new story'}</h2>
      </div>
      <button
        class="text-button"
        type="button"
        disabled={busy}
        onclick={() => {
          mode = mode === 'login' ? 'register' : 'login';
          error = undefined;
        }}
      >{mode === 'login' ? 'Create account' : 'Use existing account'}</button>
    </div>

    <form
      class="access-form"
      onsubmit={(event) => {
        event.preventDefault();
        void submitAccess();
      }}
    >
      <label for="account-name">Account name</label>
      <input
        id="account-name"
        bind:value={accountName}
        autocomplete="username"
        maxlength="40"
        required
      />
      <label for="account-password">Password</label>
      <input
        id="account-password"
        bind:value={password}
        type="password"
        autocomplete={mode === 'login' ? 'current-password' : 'new-password'}
        minlength="8"
        maxlength="1024"
        required
      />
      <button class="access-submit" type="submit" disabled={busy || !accountName.trim() || password.length < 8}>
        <span>{busy ? 'Opening the way…' : submitLabel}</span><span aria-hidden="true">→</span>
      </button>
    </form>
  </section>
{/if}

{#if error}
  <p class="access-error" role="alert">{error}</p>
{/if}
