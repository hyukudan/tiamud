#!/usr/bin/env node

import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { collectCandidateEvidence, fileEvidence } from './soak-evidence.mjs';

const DAY_MS = 86_400_000;
const HALF_HOUR_MS = 1_800_000;
const MINUTE_MS = 60_000;
const TIMING_TOLERANCE_MS = 60_000;
const SAMPLE_COVERAGE_RATIO = 0.95;
const SAMPLE_GAP_GRACE_MS = 5_000;
const MIB = 1024 * 1024;
const root = path.resolve(import.meta.dirname, '..');
const REQUIRED_VERDICTS = [
  'loadPassed',
  'readinessPassed',
  'enoughRssSamples',
  'totalRssGrowthPassed',
  'continuousRssGrowthPassed',
];

export function verifySoakAcceptance(report) {
  const errors = [];
  requireValue(errors, report?.schemaVersion === 2, 'schemaVersion must be 2');
  requireValue(errors, report?.success === true, 'report success must be true');
  verifyProfile(errors, report?.profile);

  const startedAt = timestampValue(report?.evidence?.startedAt);
  const finishedAt = timestampValue(report?.evidence?.finishedAt);
  requireValue(
    errors,
    startedAt !== null,
    'startedAt must be an ISO timestamp',
  );
  requireValue(
    errors,
    finishedAt !== null,
    'finishedAt must be an ISO timestamp',
  );
  const wallElapsedMs =
    startedAt === null || finishedAt === null ? null : finishedAt - startedAt;
  requireValue(
    errors,
    Number.isSafeInteger(report?.evidence?.elapsedMs) &&
      report.evidence.elapsedMs >= report?.profile?.durationMs,
    'evidence.elapsedMs must cover the configured duration',
  );
  requireValue(
    errors,
    wallElapsedMs !== null && wallElapsedMs >= report?.profile?.durationMs,
    'finishedAt-startedAt must cover the configured duration',
  );
  requireValue(
    errors,
    wallElapsedMs !== null &&
      Number.isSafeInteger(report?.evidence?.elapsedMs) &&
      Math.abs(wallElapsedMs - report.evidence.elapsedMs) <= TIMING_TOLERANCE_MS,
    'wall-clock and monotonic elapsed durations must agree within one minute',
  );
  verifyRunner(errors, report?.evidence?.runner);

  const commit = report?.evidence?.commit;
  const candidate = report?.evidence?.candidate;
  requireValue(errors, gitObjectId(commit), 'evidence.commit must be a valid Git object id');
  requireValue(errors, isRecord(candidate), 'candidate evidence must be present');
  requireValue(errors, candidate?.commit === commit, 'candidate commit must match evidence.commit');
  requireValue(errors, gitObjectId(candidate?.tree), 'candidate tree must be a valid Git object id');
  requireValue(errors, candidate?.clean === true, 'candidate Git tree must be clean');
  requireValue(
    errors,
    candidate?.environmentCommitMatches === null || candidate?.environmentCommitMatches === true,
    'candidate commit must match GITHUB_SHA when supplied',
  );
  requireValue(
    errors,
    candidate?.worldPack?.path === 'worlds/lantern-square.world.json' &&
      validFileEvidence(candidate.worldPack),
    'candidate WorldPack evidence is missing or invalid',
  );
  requireValue(
    errors,
    candidate?.assets?.path === 'apps/web/public/assets' &&
      candidate.assets.algorithm === 'sha256(path\\0bytes\\0sha256\\n)' &&
      Number.isSafeInteger(candidate.assets.files) &&
      candidate.assets.files > 0 &&
      validFileEvidence(candidate.assets),
    'candidate asset-tree evidence is missing or invalid',
  );

  requireValue(errors, report?.target?.mode === 'temporary-server', 'target must be a temporary server');
  requireValue(errors, validFileEvidence(report?.target?.binary), 'target binary evidence is missing');
  verifyTargetConfiguration(errors, report?.target?.configuration, report?.profile?.users);
  for (const name of ['soak', 'evidence', 'load', 'bot']) {
    requireValue(
      errors,
      validFileEvidence(report?.evidence?.harness?.[name]),
      `harness ${name} evidence is missing`,
    );
  }

  verifyLoad(errors, report?.load, report?.profile, report?.evidence?.elapsedMs);
  verifyMetrics(errors, report?.metrics);
  verifySamplesAndAnalysis(errors, report, startedAt, finishedAt);

  requireValue(errors, isRecord(report?.verdict), 'verdict must be present');
  for (const name of REQUIRED_VERDICTS) {
    requireValue(errors, report?.verdict?.[name] === true, `verdict.${name} must pass`);
  }
  if (isRecord(report?.verdict)) {
    for (const [name, value] of Object.entries(report.verdict)) {
      requireValue(errors, value === true, `verdict.${name} must pass`);
    }
  }
  requireValue(
    errors,
    Array.isArray(report?.errors) && report.errors.length === 0,
    'report errors must be empty',
  );
  return errors;
}

function verifyRunner(errors, runner) {
  requireValue(errors, isRecord(runner), 'evidence.runner must be present');
  requireValue(errors, runner?.platform === 'linux', 'evidence.runner.platform must be linux');
  requireValue(errors, runner?.architecture === 'x64', 'evidence.runner.architecture must be x64');
  requireValue(
    errors,
    typeof runner?.node === 'string' && /^24\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(runner.node),
    'evidence.runner.node must be a Node 24 version',
  );
}

function verifyProfile(errors, profile) {
  requireValue(errors, isRecord(profile), 'profile must be present');
  requireValue(
    errors,
    profile?.durationMs === DAY_MS,
    'profile.durationMs must be exactly 24 hours',
  );
  requireValue(
    errors,
    Number.isSafeInteger(profile?.users) && profile.users >= 16,
    'profile.users must be at least 16',
  );
  requireValue(
    errors,
    Number.isSafeInteger(profile?.concurrency) && profile.concurrency >= 4,
    'profile.concurrency must be at least 4',
  );
  requireValue(
    errors,
    Number.isSafeInteger(profile?.concurrency) &&
      Number.isSafeInteger(profile?.users) &&
      profile.concurrency <= profile.users,
    'profile.concurrency must not exceed profile.users',
  );
  requireValue(
    errors,
    Number.isSafeInteger(profile?.paceMs) && profile.paceMs >= 0 && profile.paceMs <= 150,
    'profile.paceMs must be no more than 150ms',
  );
  requireValue(
    errors,
    Number.isSafeInteger(profile?.sampleIntervalMs) &&
      profile.sampleIntervalMs > 0 &&
      profile.sampleIntervalMs <= MINUTE_MS,
    'profile.sampleIntervalMs must be no more than one minute',
  );
  requireValue(
    errors,
    profile?.warmupMs === HALF_HOUR_MS,
    'profile.warmupMs must be exactly 30 minutes',
  );
  requireValue(errors, isRecord(profile?.limits), 'profile.limits must be present');
  verifyLimit(errors, profile?.limits?.totalRssGrowthBytes, 96 * MIB, 'totalRssGrowthBytes');
  verifyLimit(
    errors,
    profile?.limits?.sustainedRssGrowthBytes,
    16 * MIB,
    'sustainedRssGrowthBytes',
  );
  verifyLimit(errors, profile?.limits?.rssSlopeBytesPerHour, MIB, 'rssSlopeBytesPerHour');
}

function verifyLimit(errors, value, maximum, name) {
  requireValue(
    errors,
    Number.isSafeInteger(value) && value >= 0 && value <= maximum,
    `profile.limits.${name} must be no more than ${maximum} bytes`,
  );
}

function verifyTargetConfiguration(errors, configuration, users) {
  requireValue(errors, isRecord(configuration), 'target.configuration must be present');
  const expected = {
    storage: 'temporary-sqlite',
    worldPack: 'embedded',
    tickMillis: 1_000,
    telnetMode: 'plaintext',
    authAttemptsPerMinute: Number.isSafeInteger(users) ? Math.max(1_000, users * 20) : null,
    authBurst: Number.isSafeInteger(users) ? Math.max(100, users * 4) : null,
    commandsPerSecond: 100,
    commandBurst: 200,
    maxSessionsPerAccount: 4,
    maxSessionsPerIp: 16,
  };
  requireValue(
    errors,
    isRecord(configuration) &&
      Object.keys(configuration).length === Object.keys(expected).length &&
      Object.keys(configuration).every((name) => Object.hasOwn(expected, name)),
    'target.configuration must contain exactly the canonical temporary-server fields',
  );
  for (const [name, value] of Object.entries(expected)) {
    requireValue(
      errors,
      configuration?.[name] === value,
      `target.configuration.${name} must match the canonical temporary-server profile`,
    );
  }
}

function verifyLoad(errors, load, profile, evidenceElapsedMs) {
  requireValue(errors, isRecord(load), 'load report must be present');
  requireValue(errors, load?.success === true, 'load harness must pass');
  requireValue(errors, load?.mode === 'existing-server', 'load mode must target the soak server');
  for (const name of ['users', 'concurrency', 'paceMs']) {
    requireValue(errors, load?.[name] === profile?.[name], `load.${name} must match profile.${name}`);
  }
  requireValue(
    errors,
    isRecord(load?.requested) && load.requested.durationMs === profile?.durationMs,
    'load.requested.durationMs must match profile.durationMs',
  );
  requireValue(
    errors,
    Number.isFinite(load?.elapsedMs) &&
      load.elapsedMs >= profile?.durationMs &&
      load.elapsedMs <= evidenceElapsedMs + 1,
    'load.elapsedMs must cover the requested duration without exceeding total elapsed time',
  );
  requireValue(
    errors,
    isRecord(load?.completed) && load.completed.bootstraps === profile?.users,
    'load.completed.bootstraps must equal profile.users',
  );
  requireValue(
    errors,
    Number.isSafeInteger(load?.completed?.rounds) && load.completed.rounds > 0,
    'load.completed.rounds must be a positive integer',
  );
  requireValue(
    errors,
    Array.isArray(load?.errors) && load.errors.length === 0,
    'load harness errors must be empty',
  );
}

function verifyMetrics(errors, metrics) {
  requireValue(errors, isRecord(metrics), 'metrics must be present');
  requireValue(errors, isRecord(metrics?.initial), 'metrics.initial must be present');
  requireValue(errors, isRecord(metrics?.final), 'metrics.final must be present');
  const actionKey = 'tiamud_actions_total';
  const errorKeys = [
    'tiamud_action_errors_total',
    'tiamud_auth_rate_limited_total',
    'tiamud_output_dropped_total{transport="websocket"}',
    'tiamud_output_dropped_total{transport="telnet"}',
    'tiamud_slow_client_disconnects_total{transport="websocket"}',
    'tiamud_slow_client_disconnects_total{transport="telnet"}',
    'tiamud_database_errors_total',
    'tiamud_tick_errors_total',
    'tiamud_script_errors_total',
    'tiamud_script_timeouts_total',
  ];
  requireValue(
    errors,
    validCounter(metrics?.initial?.[actionKey]) &&
      validCounter(metrics?.final?.[actionKey]) &&
      metrics.final[actionKey] - metrics.initial[actionKey] > 0,
    'tiamud_actions_total must have a positive final-minus-initial delta',
  );
  for (const name of errorKeys) {
    requireValue(
      errors,
      validCounter(metrics?.initial?.[name]) &&
        validCounter(metrics?.final?.[name]) &&
        metrics.final[name] - metrics.initial[name] === 0,
      `${name} must have a zero final-minus-initial delta`,
    );
  }
}

function verifySamplesAndAnalysis(errors, report, startedAt, finishedAt) {
  const samples = report?.samples;
  requireValue(errors, Array.isArray(samples) && samples.length > 0, 'samples must be present');
  if (!Array.isArray(samples) || samples.length === 0) return;

  let previousElapsed = -1;
  let previousTimestamp = null;
  for (const [index, sample] of samples.entries()) {
    const timestamp = timestampValue(sample?.timestamp);
    requireValue(errors, isRecord(sample), `samples[${index}] must be an object`);
    requireValue(
      errors,
      Number.isSafeInteger(sample?.elapsedMs) &&
        sample.elapsedMs >= 0 &&
        sample.elapsedMs > previousElapsed &&
        sample.elapsedMs <= report?.evidence?.elapsedMs,
      `samples[${index}].elapsedMs must be strictly monotonic and within total elapsed time`,
    );
    if (index > 0) {
      requireValue(
        errors,
        Number.isSafeInteger(sample?.elapsedMs) &&
          sample.elapsedMs - previousElapsed <=
            report?.profile?.sampleIntervalMs * 2 + SAMPLE_GAP_GRACE_MS,
        `samples[${index}].elapsedMs gap exceeds twice the configured interval plus five seconds`,
      );
    }
    requireValue(
      errors,
      timestamp !== null &&
        (previousTimestamp === null || timestamp > previousTimestamp) &&
        (startedAt === null || timestamp >= startedAt) &&
        (finishedAt === null || timestamp <= finishedAt) &&
        (startedAt === null ||
          !Number.isSafeInteger(sample?.elapsedMs) ||
          Math.abs(timestamp - startedAt - sample.elapsedMs) <= TIMING_TOLERANCE_MS),
      `samples[${index}].timestamp must be strictly monotonic and agree with elapsedMs`,
    );
    requireValue(
      errors,
      Number.isSafeInteger(sample?.rssBytes) && sample.rssBytes > 0,
      `samples[${index}].rssBytes must be a positive integer`,
    );
    requireValue(errors, sample?.ready === true, `samples[${index}].ready must be true`);
    if (Number.isSafeInteger(sample?.elapsedMs)) previousElapsed = sample.elapsedMs;
    if (timestamp !== null) previousTimestamp = timestamp;
  }
  requireValue(
    errors,
    samples.at(-1)?.elapsedMs >= report?.profile?.durationMs,
    'samples must cover the configured duration',
  );
  const expectedSampleCount =
    Math.floor(report?.profile?.durationMs / report?.profile?.sampleIntervalMs) + 1;
  requireValue(
    errors,
    Number.isSafeInteger(expectedSampleCount) &&
      samples.length >= Math.ceil(expectedSampleCount * SAMPLE_COVERAGE_RATIO),
    'samples must retain at least 95% of the configured sampling cadence',
  );

  const recomputed = analyzeSamples(samples, report?.profile, report?.load);
  requireValue(
    errors,
    report?.readiness?.samples === recomputed.readiness.samples,
    'readiness.samples must match samples',
  );
  requireValue(
    errors,
    report?.readiness?.failures === recomputed.readiness.failures,
    'readiness.failures must match samples',
  );
  for (const name of Object.keys(recomputed.rss)) {
    requireValue(errors, report?.rss?.[name] === recomputed.rss[name], `rss.${name} must match samples`);
  }
  for (const name of REQUIRED_VERDICTS) {
    requireValue(
      errors,
      report?.verdict?.[name] === recomputed.verdict[name],
      `verdict.${name} must match recomputed evidence`,
    );
  }
}

function analyzeSamples(samples, profile, load) {
  const readyFailures = samples.filter((entry) => !entry?.ready).length;
  const allRss = samples.map((entry) => entry?.rssBytes).filter(Number.isSafeInteger);
  const measured = samples.filter(
    (entry) => entry?.elapsedMs >= profile?.warmupMs && Number.isSafeInteger(entry?.rssBytes),
  );
  const initial = allRss.at(0) ?? null;
  const final = allRss.at(-1) ?? null;
  const peak = allRss.length > 0 ? Math.max(...allRss) : null;
  const totalGrowth = initial === null || final === null ? null : final - initial;
  const windowSize = Math.max(1, Math.floor(measured.length / 4));
  const firstMedian = median(measured.slice(0, windowSize).map((entry) => entry.rssBytes));
  const lastMedian = median(measured.slice(-windowSize).map((entry) => entry.rssBytes));
  const sustainedGrowth =
    firstMedian === null || lastMedian === null ? null : lastMedian - firstMedian;
  const slope = regressionSlopeBytesPerHour(measured);
  const exceedsTotal =
    totalGrowth !== null && totalGrowth > profile?.limits?.totalRssGrowthBytes;
  const exceedsSustained =
    sustainedGrowth !== null && sustainedGrowth > profile?.limits?.sustainedRssGrowthBytes;
  const exceedsSlope = slope !== null && slope > profile?.limits?.rssSlopeBytesPerHour;
  const suspectedContinuousGrowth = exceedsSustained && exceedsSlope;
  return {
    verdict: {
      loadPassed: load?.success === true,
      readinessPassed: readyFailures === 0,
      enoughRssSamples: measured.length >= 4,
      totalRssGrowthPassed: !exceedsTotal,
      continuousRssGrowthPassed: !suspectedContinuousGrowth,
    },
    rss: {
      sampleCount: allRss.length,
      postWarmupSampleCount: measured.length,
      initialBytes: initial,
      finalBytes: final,
      peakBytes: peak,
      totalGrowthBytes: totalGrowth,
      firstWindowMedianBytes: firstMedian,
      lastWindowMedianBytes: lastMedian,
      sustainedGrowthBytes: sustainedGrowth,
      regressionSlopeBytesPerHour: slope === null ? null : Math.round(slope),
      suspectedContinuousGrowth,
    },
    readiness: { samples: samples.length, failures: readyFailures },
  };
}

function regressionSlopeBytesPerHour(values) {
  if (values.length < 2) return null;
  const origin = values[0].elapsedMs;
  const points = values.map((entry) => [
    (entry.elapsedMs - origin) / 3_600_000,
    entry.rssBytes,
  ]);
  const meanX = points.reduce((sum, [x]) => sum + x, 0) / points.length;
  const meanY = points.reduce((sum, [, y]) => sum + y, 0) / points.length;
  let numerator = 0;
  let denominator = 0;
  for (const [x, y] of points) {
    numerator += (x - meanX) * (y - meanY);
    denominator += (x - meanX) ** 2;
  }
  return denominator === 0 ? null : numerator / denominator;
}

function median(values) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[middle - 1] + sorted[middle]) / 2 : sorted[middle];
}

function validCounter(value) {
  return Number.isFinite(value) && value >= 0;
}

export function verifySoakEvidenceAgainstCheckout(report, checkout) {
  const errors = [];
  requireValue(
    errors,
    sameCandidateEvidence(report?.evidence?.candidate, checkout?.candidate),
    'recorded candidate evidence must match the current checkout',
  );
  for (const name of ['soak', 'evidence', 'load', 'bot']) {
    requireValue(
      errors,
      sameFileEvidence(report?.evidence?.harness?.[name], checkout?.harness?.[name]),
      `recorded harness ${name} digest must match the current checkout`,
    );
  }
  requireValue(
    errors,
    sameFileEvidence(report?.target?.binary, checkout?.binary),
    'recorded binary digest must match --binary',
  );
  return errors;
}

function validFileEvidence(value) {
  return (
    isRecord(value) &&
    Number.isSafeInteger(value.bytes) &&
    value.bytes > 0 &&
    typeof value.sha256 === 'string' &&
    /^[0-9a-f]{64}$/.test(value.sha256)
  );
}

function gitObjectId(value) {
  return typeof value === 'string' && /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(value);
}

function timestampValue(value) {
  if (typeof value !== 'string') return null;
  const date = new Date(value);
  return Number.isFinite(date.valueOf()) && date.toISOString() === value ? date.valueOf() : null;
}

function isRecord(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function requireValue(errors, condition, message) {
  if (!condition) errors.push(message);
}

async function main() {
  const arguments_ = process.argv.slice(2);
  if (arguments_.length !== 3 || arguments_[0] !== '--binary') {
    console.error(
      'Usage: node scripts/verify-soak-report.mjs --binary <server-binary> <soak-report.json>',
    );
    process.exitCode = 2;
    return;
  }
  const binaryPath = path.resolve(arguments_[1]);
  const reportPath = path.resolve(arguments_[2]);
  let report;
  try {
    report = JSON.parse(await readFile(reportPath, 'utf8'));
  } catch (error) {
    console.error(`Could not read soak report: ${errorMessage(error)}`);
    process.exitCode = 2;
    return;
  }
  const errors = verifySoakAcceptance(report);
  try {
    const checkout = {
      candidate: await collectCandidateEvidence(root),
      harness: {
        soak: await fileEvidence(path.join(root, 'scripts', 'soak.mjs')),
        evidence: await fileEvidence(path.join(root, 'scripts', 'soak-evidence.mjs')),
        load: await fileEvidence(path.join(root, 'scripts', 'load.mjs')),
        bot: await fileEvidence(path.join(root, 'scripts', 'bot.mjs')),
      },
      binary: await fileEvidence(binaryPath),
    };
    errors.push(...verifySoakEvidenceAgainstCheckout(report, checkout));
  } catch (error) {
    errors.push(`could not recompute checkout evidence: ${errorMessage(error)}`);
  }
  if (errors.length > 0) {
    console.error('24-hour soak acceptance failed:');
    for (const error of errors) console.error(`- ${error}`);
    process.exitCode = 1;
    return;
  }
  console.log(`24-hour soak acceptance passed for ${report.evidence.commit}.`);
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function sameCandidateEvidence(recorded, current) {
  return (
    isRecord(recorded) &&
    isRecord(current) &&
    recorded.commit === current.commit &&
    recorded.tree === current.tree &&
    recorded.clean === current.clean &&
    sameFileEvidence(recorded.worldPack, current.worldPack) &&
    recorded.worldPack.path === current.worldPack.path &&
    sameFileEvidence(recorded.assets, current.assets) &&
    recorded.assets.path === current.assets.path &&
    recorded.assets.algorithm === current.assets.algorithm &&
    recorded.assets.files === current.assets.files
  );
}

function sameFileEvidence(recorded, current) {
  return (
    validFileEvidence(recorded) &&
    validFileEvidence(current) &&
    recorded.bytes === current.bytes &&
    recorded.sha256 === current.sha256
  );
}

const entrypoint = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : undefined;
if (entrypoint === import.meta.url) await main();
