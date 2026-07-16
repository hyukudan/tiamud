<script lang="ts">
  import { onMount } from 'svelte';
  import {
    authenticate,
    clearStoredAccess,
    createCharacter,
    loadCharacterOptions,
    loadCharacters,
    logout,
    readStoredAccess,
    writeStoredAccess,
    type AuthenticatedSession,
    type Character,
    type CharacterCreationCatalog,
    type CharacterCreationSelection,
    type CharacterSelection,
  } from '../lib/auth';

  let { onselect }: { onselect: (selection: CharacterSelection) => void } = $props();

  let mode = $state<'login' | 'register'>('login');
  let accountName = $state('');
  let password = $state('');
  let characterName = $state('');
  let characterOptions = $state<CharacterCreationCatalog>();
  let lineageId = $state('');
  let backgroundChoice = $state<'@default' | '@none' | string>('@default');
  let baseClassId = $state('');
  let session = $state<AuthenticatedSession>();
  let characters = $state<Character[]>([]);
  let lastCharacterId = $state<number>();
  let busy = $state(false);
  let restoring = $state(false);
  let error = $state<string>();

  const submitLabel = $derived(mode === 'login' ? 'Sign in' : 'Create account');
  const defaultLineage = $derived(
    characterOptions?.lineages.find((option) => option.id === characterOptions?.defaultLineageId),
  );
  const selectedLineage = $derived(
    lineageId ? characterOptions?.lineages.find((option) => option.id === lineageId) : defaultLineage,
  );
  const defaultBackground = $derived(
    characterOptions?.backgrounds.find((option) => option.id === characterOptions?.defaultBackgroundId),
  );
  const selectedBackground = $derived(
    backgroundChoice === '@default'
      ? defaultBackground
      : characterOptions?.backgrounds.find((option) => option.id === backgroundChoice),
  );
  const defaultBaseClass = $derived(
    characterOptions?.baseClasses.find((option) => option.id === characterOptions?.defaultBaseClassId),
  );
  const selectedBaseClass = $derived(
    baseClassId
      ? characterOptions?.baseClasses.find((option) => option.id === baseClassId)
      : defaultBaseClass,
  );

  async function refreshRoster(current: AuthenticatedSession): Promise<void> {
    const [nextCharacters, optionsResult] = await Promise.all([
      loadCharacters(current.token),
      loadCharacterOptions()
        .then((value) => ({ value }))
        .catch((cause: unknown) => ({ cause })),
    ]);
    characters = nextCharacters;
    if ('cause' in optionsResult) {
      characterOptions = undefined;
      resetCreationChoices();
      error = optionsResult.cause instanceof Error
        ? optionsResult.cause.message
        : 'Character options are unavailable.';
      return;
    }
    const nextOptions = optionsResult.value;
    characterOptions = nextOptions;
    lineageId = lineageId && nextOptions.lineages.some((option) => option.id === lineageId) ? lineageId : '';
    baseClassId = baseClassId && nextOptions.baseClasses.some((option) => option.id === baseClassId)
      ? baseClassId
      : '';
    backgroundChoice = backgroundChoice === '@none' ||
      (backgroundChoice !== '@default' && nextOptions.backgrounds.some((option) => option.id === backgroundChoice))
      ? backgroundChoice
      : '@default';
  }

  function resetCreationChoices(): void {
    lineageId = '';
    backgroundChoice = '@default';
    baseClassId = '';
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
      const selection: CharacterCreationSelection = {
        ...(lineageId ? { lineageId } : {}),
        ...(baseClassId ? { baseClassId } : {}),
        ...(backgroundChoice === '@none'
          ? { backgroundId: null }
          : backgroundChoice === '@default' ? {} : { backgroundId: backgroundChoice }),
      };
      const character = await createCharacter(session.token, characterName.trim(), selection);
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
    characterOptions = undefined;
    resetCreationChoices();
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
            <span>{character.name}</span>
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
      {#if characterOptions}
        <fieldset class="character-origin-fields">
          <legend>Origins and starting path</legend>
        <label for="character-lineage">Lineage</label>
        <select
          id="character-lineage"
          bind:value={lineageId}
          disabled={busy}
          aria-describedby="lineage-description"
        >
          <option value="">World default ({defaultLineage?.label ?? characterOptions.defaultLineageId})</option>
          {#each characterOptions.lineages as option (option.id)}
            <option value={option.id}>{option.label}</option>
          {/each}
        </select>
        <p id="lineage-description">{selectedLineage?.description ?? 'Use the world author’s suggested lineage.'}</p>

        <label for="character-background">Background</label>
        <select
          id="character-background"
          bind:value={backgroundChoice}
          disabled={busy}
          aria-describedby="background-description"
        >
          <option value="@default">World default{defaultBackground ? ` (${defaultBackground.label})` : ' (none)'}</option>
          <option value="@none">No background</option>
          {#each characterOptions.backgrounds as option (option.id)}
            <option value={option.id}>{option.label}</option>
          {/each}
        </select>
        {#if backgroundChoice === '@none'}
          <p id="background-description">Begin without a background package. This differs from the world default.</p>
        {:else if selectedBackground}
          <p id="background-description">{selectedBackground.description}</p>
        {:else}
          <p id="background-description">The world author has chosen no default background.</p>
        {/if}

        <label for="character-base-class">Base class</label>
        <select
          id="character-base-class"
          bind:value={baseClassId}
          disabled={busy}
          aria-describedby="base-class-description"
        >
          <option value="">World default ({defaultBaseClass?.label ?? characterOptions.defaultBaseClassId})</option>
          {#each characterOptions.baseClasses as option (option.id)}
            <option value={option.id}>{option.label}</option>
          {/each}
        </select>
        <p id="base-class-description">{selectedBaseClass?.description ?? 'Use the world author’s suggested base class.'}</p>
        </fieldset>
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
