import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import appSource from './App.svelte?raw';

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

  it('labels local, direct, party, and world-channel social messages in history', () => {
    expect(appSource).toContain("case 'announcement'");
    expect(appSource).toContain('`${message.sender} announces`');
    expect(appSource).toContain("case 'say'");
    expect(appSource).toContain("case 'tell'");
    expect(appSource).toContain("case 'telepathy'");
    expect(appSource).toContain("case 'party'");
    expect(appSource).toContain('in #${message.channel}');
    expect(appSource).toContain('socialHistoryLabel(message.data)');
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
    expect(appSource).toContain('mapVisibleScenePresentation(scene)');
    expect(appSource).toContain('const visualLayout = $derived.by');
  });

  it('uses only bounded semantic cue classes and honours reduced motion', () => {
    for (const cue of ['attack', 'hit', 'heal', 'defeat']) {
      expect(appSource).toContain(`class:cue-${cue}`);
    }
    expect(appSource).not.toContain('class:cue-status');
    expect(appSource).not.toContain('class:cue-spawn');
    expect(appSource).toContain("case 'message.append'");
    expect(appSource).toContain('class:motion-reduced={reduceVisualMotion}');
    expect(appSource).toContain('visualSourceForCue(entity.visual, cue)');
    expect(appSource).not.toContain('innerHTML');
  });

  it('keeps same-view cues through the immediate snapshot but clears recovery state', () => {
    expect(appSource).toContain('reconcileScenePresentationCues(scene, message.data, activeVisualCues)');
    expect(appSource).not.toContain("case 'scene.snapshot':\n        resetVisualPresentation();");
    expect(appSource).toContain('function handleSyncRecovery(recovery: SyncRecoveryData)');
    expect(appSource).toMatch(/function handleSyncRecovery[\s\S]*?resetVisualPresentation\(\);/);
  });

  it('applies correlated recovery through one atomic sync callback', () => {
    expect(appSource).toContain('function handleSyncRecovery(recovery: SyncRecoveryData)');
    expect(appSource).toContain('scene = recovery.scene;');
    expect(appSource).toContain('player = recovery.player;');
    expect(appSource).toContain('onSync: handleSyncRecovery');
    expect(appSource).not.toContain("case 'sync.scene'");
    expect(appSource).not.toContain("case 'sync.player'");
    expect(appSource).not.toContain("case 'sync.snapshot'");
  });

  it('never carries a paginated container view across mutations, recovery, or sessions', () => {
    expect(appSource).toMatch(
      /function submitAction[\s\S]*?if \(sent && \[[\s\S]*?'item\.put'[\s\S]*?'item\.get'[\s\S]*?'item\.contents'[\s\S]*?containerContents = undefined;/,
    );
    for (const boundary of [
      'function handleSyncRecovery',
      'function enterWorld',
      'function returnToLogin',
      'async function signOut',
    ]) {
      const start = appSource.indexOf(boundary);
      expect(start).toBeGreaterThanOrEqual(0);
      expect(appSource.slice(start, start + 800)).toContain('containerContents = undefined;');
    }
  });
});
