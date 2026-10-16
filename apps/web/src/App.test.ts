import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import appSource from './App.svelte?raw';
import historyFormatSource from './lib/app/history-format.ts?raw';
import sessionSource from './lib/app/game-session.svelte.ts?raw';
import worldViewsSource from './lib/app/world-views.ts?raw';

describe('Tiamud entry flow', () => {
  it('shows account access at the illustrated gate and does not expose guest entry', () => {
    const { body } = render(App);

    expect(body).toContain('<h1 id="welcome-title">Tiamud</h1>');
    expect(body).toContain('Return to your story');
    expect(body).not.toContain('Enter Tiamud');
    expect(body).not.toContain('Awaiting the first scene');
  });

  it('exposes one named live log instead of mirroring world events into the notice region', () => {
    expect(appSource).toContain('role="log"');
    expect(appSource).toContain('aria-label="Command and world event history"');
    expect(appSource).not.toContain("notice = { level: 'info', message: message.data.text }");
  });

  it('labels local, spatial, direct, authored-emote, party, and channel messages in history', () => {
    expect(historyFormatSource).toContain("case 'announcement'");
    expect(historyFormatSource).toContain('`${message.sender} announces`');
    expect(historyFormatSource).toContain("case 'say'");
    expect(historyFormatSource).toContain("case 'yell'");
    expect(historyFormatSource).toContain('`${message.sender} yells`');
    expect(historyFormatSource).toContain("case 'shout'");
    expect(historyFormatSource).toContain('`${message.sender} shouts`');
    expect(historyFormatSource).toContain("case 'tell'");
    expect(historyFormatSource).toContain("case 'telepathy'");
    expect(historyFormatSource).toContain("case 'emote'");
    expect(historyFormatSource).toContain("return 'Emote'");
    expect(historyFormatSource).toContain("case 'party'");
    expect(historyFormatSource).toContain('in #${message.channel}');
    expect(sessionSource).toContain('socialHistoryLabel(message.data)');
  });

  it('keeps a 500-entry anchored scrollback with an explicit latest affordance', () => {
    expect(appSource).toContain('MAX_SCROLLBACK_ENTRIES');
    expect(appSource).toContain('isScrollAnchored');
    expect(appSource).toContain('Jump to latest');
  });

  it('renders loot access textually and submits native status and pass actions', () => {
    expect(appSource).toContain('Reserved for you');
    expect(appSource).toContain('Reserved for another adventurer');
    expect(appSource).toContain("submitAction('loot.status', undefined, 'loot status')");
    expect(appSource).toContain("submitAction('loot.pass', { target: item.id }, action.command)");
    expect(appSource).toContain("'loot.pass'");
  });

  it('keeps optional scene sprites decorative, bounded and non-interactive', () => {
    expect(appSource).toContain('layoutSceneEntities');
    expect(appSource).toContain('maxVisible: 10');
    expect(appSource).toContain('class="entity-visual-layer" aria-hidden="true"');
    expect(appSource).toContain('alt=""');
    expect(appSource).toContain('visualLayout.overflow.slice(0, 5)');
    expect(sessionSource).toContain('mapVisibleScenePresentation(this.scene)');
    expect(appSource).toContain('const visualLayout = $derived.by');
  });

  it('uses only bounded semantic cue classes and honours reduced motion', () => {
    for (const cue of ['attack', 'hit', 'heal', 'defeat']) {
      expect(appSource).toContain(`class:cue-${cue}`);
    }
    expect(appSource).not.toContain('class:cue-status');
    expect(appSource).not.toContain('class:cue-spawn');
    expect(sessionSource).toContain("message.type === 'message.append'");
    expect(appSource).toContain('class:motion-reduced={reduceVisualMotion}');
    expect(appSource).toContain('visualSourceForCue(entity.visual, cue)');
    expect(appSource).not.toContain('innerHTML');
  });

  it('keeps session state and message dispatch out of the layout component', () => {
    expect(appSource).toContain('new GameSession(');
    expect(appSource).not.toContain('function handleMessage');
    expect(appSource).not.toContain('new TiamudConnection(');
    expect(sessionSource).toContain('onSync: this.handleSyncRecovery');
    expect(sessionSource).toContain('reconcileScenePresentationCues(');
    for (const source of [appSource, sessionSource, worldViewsSource]) {
      expect(source).not.toContain("case 'sync.scene'");
      expect(source).not.toContain("case 'sync.player'");
      expect(source).not.toContain("case 'sync.snapshot'");
    }
  });

  it('loads the player dashboard lazily with a visible loading and retry state', () => {
    expect(appSource).not.toContain("import PlayerDashboard from './components/PlayerDashboard.svelte'");
    expect(appSource).toContain('loadPlayerDashboard()');
    expect(appSource).toContain('Loading character panels');
    expect(appSource).toContain('Retry loading panels');
  });
});
