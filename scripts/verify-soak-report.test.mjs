import assert from 'node:assert/strict';
import test from 'node:test';

import {
  verifySoakAcceptance,
  verifySoakEvidenceAgainstCheckout,
} from './verify-soak-report.mjs';

const SHA256 = 'a'.repeat(64);
const COMMIT = 'b'.repeat(40);
const TREE = 'c'.repeat(40);

test('accepts complete schema-v2 temporary-server evidence', () => {
  assert.deepEqual(verifySoakAcceptance(validReport()), []);
});

test('rejects every independently required acceptance field', async (context) => {
  const cases = [
    ['schema version', (report) => (report.schemaVersion = 1), 'schemaVersion must be 2'],
    ['24-hour profile', (report) => (report.profile.durationMs = 3_600_000), 'exactly 24 hours'],
    ['elapsed duration', (report) => (report.evidence.elapsedMs = 1), 'must cover'],
    ['wall duration', (report) => (report.evidence.finishedAt = '2026-07-15T00:00:01.000Z'), 'finishedAt-startedAt'],
    ['runner platform', (report) => (report.evidence.runner.platform = 'darwin'), 'platform must be linux'],
    ['runner architecture', (report) => (report.evidence.runner.architecture = 'arm64'), 'architecture must be x64'],
    ['runner Node', (report) => (report.evidence.runner.node = '25.0.0'), 'Node 24'],
    ['commit', (report) => (report.evidence.commit = 'working-tree'), 'valid Git object id'],
    ['candidate commit', (report) => (report.evidence.candidate.commit = TREE), 'must match evidence.commit'],
    ['candidate tree', (report) => (report.evidence.candidate.tree = 'invalid'), 'valid Git object id'],
    ['clean tree', (report) => (report.evidence.candidate.clean = false), 'must be clean'],
    ['matching environment', (report) => (report.evidence.candidate.environmentCommitMatches = false), 'GITHUB_SHA'],
    ['WorldPack evidence', (report) => (report.evidence.candidate.worldPack.sha256 = null), 'WorldPack'],
    ['asset evidence', (report) => (report.evidence.candidate.assets.files = 0), 'asset-tree'],
    ['temporary target', (report) => {
      report.target.mode = 'existing-server';
      report.target.binary = null;
    }, 'temporary server'],
    ['binary evidence', (report) => (report.target.binary = null), 'binary evidence'],
    ['target configuration', (report) => (report.target.configuration.worldPack = 'host-env'), 'canonical temporary-server'],
    ['target configuration fields', (report) => (report.target.configuration.untracked = true), 'exactly the canonical'],
    ['harness evidence', (report) => delete report.evidence.harness.bot, 'harness bot'],
    ['Telnet harness evidence', (report) => delete report.evidence.harness.telnetBot, 'harness telnetBot'],
    ['verdict', (report) => (report.verdict.loadPassed = false), 'verdict.loadPassed'],
    ['success', (report) => (report.success = false), 'report success'],
    ['readiness', (report) => (report.readiness.failures = 1), 'readiness.failures'],
    ['load errors', (report) => report.load.errors.push('failure'), 'load harness errors'],
    ['RSS samples', (report) => (report.rss.postWarmupSampleCount = 3), 'rss.postWarmupSampleCount'],
    ['continuous growth', (report) => (report.rss.suspectedContinuousGrowth = true), 'rss.suspectedContinuousGrowth'],
    ['report errors', (report) => report.errors.push('failure'), 'report errors'],
  ];
  for (const [name, mutate, expected] of cases) {
    await context.test(name, () => {
      const report = validReport();
      mutate(report);
      assert.ok(
        verifySoakAcceptance(report).some((error) => error.includes(expected)),
        `expected an error containing ${expected}`,
      );
    });
  }
});

test('rejects the demonstrated forged 20-second report bypass', () => {
  const report = validReport();
  report.evidence.finishedAt = '2026-07-15T00:00:01.000Z';
  report.evidence.elapsedMs = DAY_MS;
  report.profile.users = 1;
  report.load.users = 1;
  report.load.requested.durationMs = 1_000;
  report.load.elapsedMs = 1_000;
  delete report.samples;
  delete report.metrics;

  const errors = verifySoakAcceptance(report);
  assert.notDeepEqual(errors, []);
  for (const expected of [
    'profile.users must be at least 16',
    'finishedAt-startedAt',
    'load.requested.durationMs',
    'load.elapsedMs',
    'metrics must be present',
    'samples must be present',
  ]) {
    assert.ok(errors.some((error) => error.includes(expected)), expected);
  }
});

test('rejects standard-profile and load cross-field tampering', async (context) => {
  const cases = [
    ['users', (report) => (report.profile.users = 15), 'at least 16'],
    ['Telnet users', (report) => (report.profile.telnetUsers = 0), 'between 1'],
    ['concurrency', (report) => (report.profile.concurrency = 3), 'at least 4'],
    ['concurrency/users', (report) => (report.profile.concurrency = 17), 'must not exceed'],
    ['pace', (report) => (report.profile.paceMs = 151), 'no more than 150'],
    ['sample interval', (report) => (report.profile.sampleIntervalMs = 60_001), 'one minute'],
    ['short warmup', (report) => (report.profile.warmupMs = 1_799_999), 'exactly 30 minutes'],
    ['long warmup', (report) => (report.profile.warmupMs = DAY_MS - 300_000), 'exactly 30 minutes'],
    ['total limit', (report) => (report.profile.limits.totalRssGrowthBytes += 1), 'totalRssGrowthBytes'],
    ['sustained limit', (report) => (report.profile.limits.sustainedRssGrowthBytes += 1), 'sustainedRssGrowthBytes'],
    ['slope limit', (report) => (report.profile.limits.rssSlopeBytesPerHour += 1), 'rssSlopeBytesPerHour'],
    ['load users', (report) => (report.load.users = 15), 'load.users'],
    ['load Telnet users', (report) => (report.load.telnetUsers = 3), 'load.telnetUsers'],
    ['load concurrency', (report) => (report.load.concurrency = 3), 'load.concurrency'],
    ['load pace', (report) => (report.load.paceMs = 149), 'load.paceMs'],
    ['load request', (report) => (report.load.requested.durationMs = 20_000), 'load.requested'],
    ['load elapsed', (report) => (report.load.elapsedMs = 20_000), 'load.elapsedMs'],
    ['bootstraps', (report) => (report.load.completed.bootstraps = 1), 'bootstraps'],
    ['rounds', (report) => (report.load.completed.rounds = 0), 'rounds'],
    ['Telnet transport errors', (report) => report.load.transports.telnet.errors.push({ phase: 'round' }), 'telnet.errors'],
  ];
  for (const [name, mutate, expected] of cases) {
    await context.test(name, () => {
      const report = validReport();
      mutate(report);
      assert.ok(verifySoakAcceptance(report).some((error) => error.includes(expected)));
    });
  }
});

test('rejects sample, RSS, readiness, and metric tampering', async (context) => {
  const cases = [
    ['missing samples', (report) => delete report.samples, 'samples must be present'],
    ['elapsed order', (report) => (report.samples[2].elapsedMs = report.samples[1].elapsedMs), 'strictly monotonic'],
    ['timestamp order', (report) => (report.samples[2].timestamp = report.samples[0].timestamp), 'strictly monotonic'],
    ['timestamp correlation', (report) => {
      const shifted = new Date(Date.parse(report.samples[2].timestamp) + 60_001);
      report.samples[2].timestamp = shifted.toISOString();
    }, 'agree with elapsedMs'],
    ['duration coverage', (report) => (report.samples.at(-1).elapsedMs = DAY_MS - 1), 'samples must cover'],
    ['sample gap', (report) => report.samples.splice(2, 3), 'gap exceeds'],
    ['sample density', (report) => (report.samples = report.samples.filter((_, index) => index % 2 === 0)), '95%'],
    ['readiness sample', (report) => (report.samples[2].ready = false), 'ready must be true'],
    ['RSS input', (report) => (report.samples[2].rssBytes += MIB), 'rss.peakBytes'],
    ['RSS output', (report) => (report.rss.totalGrowthBytes = 1), 'rss.totalGrowthBytes'],
    ['readiness output', (report) => (report.readiness.samples = 4), 'readiness.samples'],
    ['action delta', (report) => (report.metrics.final.tiamud_actions_total = 0), 'positive final-minus-initial'],
    ['error delta', (report) => (report.metrics.final.tiamud_action_errors_total = 1), 'zero final-minus-initial'],
  ];
  for (const [name, mutate, expected] of cases) {
    await context.test(name, () => {
      const report = validReport();
      mutate(report);
      assert.ok(verifySoakAcceptance(report).some((error) => error.includes(expected)));
    });
  }
});

test('rejects additional future verdicts unless they pass', () => {
  const report = validReport();
  report.verdict.futureCandidateGate = false;
  assert.ok(
    verifySoakAcceptance(report).includes('verdict.futureCandidateGate must pass'),
  );
});

test('compares retained digests against recomputed checkout evidence', () => {
  const report = validReport();
  const checkout = checkoutFor(report);
  assert.deepEqual(verifySoakEvidenceAgainstCheckout(report, checkout), []);

  checkout.candidate.assets.sha256 = 'd'.repeat(64);
  checkout.harness.bot.sha256 = 'e'.repeat(64);
  checkout.binary.sha256 = 'f'.repeat(64);
  const errors = verifySoakEvidenceAgainstCheckout(report, checkout);
  assert.ok(errors.some((error) => error.includes('candidate evidence')));
  assert.ok(errors.some((error) => error.includes('harness bot')));
  assert.ok(errors.some((error) => error.includes('binary digest')));
});

function validReport() {
  const file = () => ({ bytes: 123, sha256: SHA256 });
  const rssBytes = 100 * MIB;
  const start = Date.parse('2026-07-15T00:00:00.000Z');
  const samples = Array.from({ length: DAY_MS / MINUTE_MS + 1 }, (_, index) => {
    const elapsedMs = index * MINUTE_MS;
    return sample(elapsedMs, new Date(start + elapsedMs).toISOString(), rssBytes);
  });
  return {
    schemaVersion: 2,
    success: true,
    evidence: {
      startedAt: '2026-07-15T00:00:00.000Z',
      finishedAt: '2026-07-16T00:00:01.000Z',
      elapsedMs: 86_401_000,
      commit: COMMIT,
      candidate: {
        commit: COMMIT,
        tree: TREE,
        clean: true,
        environmentCommitMatches: true,
        worldPack: {
          path: 'worlds/lantern-square.world.json',
          ...file(),
        },
        assets: {
          path: 'apps/web/public/assets',
          algorithm: 'sha256(path\\0bytes\\0sha256\\n)',
          files: 7,
          ...file(),
        },
      },
      harness: {
        soak: file(),
        evidence: file(),
        load: file(),
        bot: file(),
        telnetBot: file(),
      },
      runner: {
        platform: 'linux',
        release: '6.8.0-test',
        architecture: 'x64',
        node: '24.13.1',
      },
    },
    target: {
      mode: 'temporary-server',
      binary: file(),
      configuration: {
        storage: 'temporary-sqlite',
        worldPack: 'embedded',
        tickMillis: 1_000,
        telnetMode: 'plaintext',
        authAttemptsPerMinute: 1_000,
        authBurst: 100,
        commandsPerSecond: 100,
        commandBurst: 200,
        maxSessionsPerAccount: 4,
        maxSessionsPerIp: 16,
      },
    },
    profile: {
      durationMs: DAY_MS,
      users: 16,
      telnetUsers: 4,
      concurrency: 4,
      paceMs: 150,
      sampleIntervalMs: 60_000,
      warmupMs: 1_800_000,
      limits: {
        totalRssGrowthBytes: 96 * MIB,
        sustainedRssGrowthBytes: 16 * MIB,
        rssSlopeBytesPerHour: MIB,
      },
    },
    verdict: {
      loadPassed: true,
      readinessPassed: true,
      enoughRssSamples: true,
      totalRssGrowthPassed: true,
      continuousRssGrowthPassed: true,
    },
    readiness: { samples: samples.length, failures: 0 },
    metrics: metrics(),
    load: {
      success: true,
      mode: 'existing-server',
      users: 16,
      telnetUsers: 4,
      websocketUsers: 12,
      concurrency: 4,
      paceMs: 150,
      requested: { durationMs: DAY_MS },
      completed: { bootstraps: 16, rounds: 100 },
      transports: {
        websocket: { users: 12, bootstraps: 12, rounds: 75, errors: [], latenciesMs: {} },
        telnet: { users: 4, bootstraps: 4, rounds: 25, errors: [], latenciesMs: {} },
      },
      elapsedMs: DAY_MS,
      errors: [],
    },
    rss: {
      sampleCount: samples.length,
      postWarmupSampleCount: samples.length - 30,
      initialBytes: rssBytes,
      finalBytes: rssBytes,
      peakBytes: rssBytes,
      totalGrowthBytes: 0,
      firstWindowMedianBytes: rssBytes,
      lastWindowMedianBytes: rssBytes,
      sustainedGrowthBytes: 0,
      regressionSlopeBytesPerHour: 0,
      suspectedContinuousGrowth: false,
    },
    samples,
    errors: [],
  };
}

function sample(elapsedMs, timestamp, rssBytes) {
  return { elapsedMs, timestamp, rssBytes, ready: true };
}

function metrics() {
  const initial = {
    tiamud_actions_total: 0,
    tiamud_action_errors_total: 0,
    tiamud_auth_rate_limited_total: 0,
    'tiamud_output_dropped_total{transport="websocket"}': 0,
    'tiamud_output_dropped_total{transport="telnet"}': 0,
    'tiamud_slow_client_disconnects_total{transport="websocket"}': 0,
    'tiamud_slow_client_disconnects_total{transport="telnet"}': 0,
    tiamud_database_errors_total: 0,
    tiamud_tick_errors_total: 0,
    tiamud_script_errors_total: 0,
    tiamud_script_timeouts_total: 0,
  };
  return { initial, final: { ...initial, tiamud_actions_total: 100 } };
}

function checkoutFor(report) {
  const checkout = structuredClone({
    candidate: report.evidence.candidate,
    harness: report.evidence.harness,
    binary: report.target.binary,
  });
  checkout.candidate.environmentCommitMatches = null;
  return checkout;
}

const DAY_MS = 86_400_000;
const MINUTE_MS = 60_000;
const MIB = 1024 * 1024;
