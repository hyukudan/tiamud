#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';

import { collectCandidateEvidence, fileEvidence } from './soak-evidence.mjs';
import { parseTelnetEndpoint } from './telnet-bot.mjs';

const root = path.resolve(import.meta.dirname, '..');
const options = parseArguments(process.argv.slice(2));

class LogTail {
  constructor(maximum) {
    this.maximum = maximum;
    this.value = '';
  }

  append(chunk) {
    this.value = appendBounded(this.value, chunk, this.maximum);
  }
}

if (options.help) {
  printHelp();
  process.exit(0);
}

requireNode24();

const startedAt = new Date();
const monotonicStart = performance.now();
const samples = [];
const errors = [];
const harnessEvidence = {
  soak: await fileEvidence(import.meta.filename),
  evidence: await fileEvidence(path.join(root, 'scripts', 'soak-evidence.mjs')),
  load: await fileEvidence(path.join(root, 'scripts', 'load.mjs')),
  bot: await fileEvidence(path.join(root, 'scripts', 'bot.mjs')),
  telnetBot: await fileEvidence(path.join(root, 'scripts', 'telnet-bot.mjs')),
};
const candidateEvidence = await collectCandidateEvidence(root).catch((error) => {
  errors.push(`candidate evidence failed: ${errorMessage(error)}`);
  return null;
});
let binaryEvidence = null;
let temporary;
let server;
let serverConfiguration = null;
let sampler;
let sampling = false;
let loadChild;
let interruptedSignal;
let loadReport;
let loadStderr = '';
let metricsInitial;
let metricsFinal;

const interrupt = (signal) => {
  interruptedSignal ??= signal;
  loadChild?.kill('SIGTERM');
};
const onSigint = () => interrupt('SIGINT');
const onSigterm = () => interrupt('SIGTERM');
process.once('SIGINT', onSigint);
process.once('SIGTERM', onSigterm);

let baseUrl;
let monitoredPid;
let telnetEndpoint = options.telnet;

try {
  if (options.url) {
    baseUrl = normalizeBaseUrl(options.url);
    monitoredPid = options.pid;
    await waitUntilReady(baseUrl);
  } else {
    binaryEvidence = await fileEvidence(path.resolve(options.binary));
    temporary = await mkdtemp(path.join(os.tmpdir(), 'tiamud-soak-'));
    server = await startServer(temporary, options.binary, options.users);
    serverConfiguration = server.configuration;
    baseUrl = server.baseUrl;
    telnetEndpoint = server.telnetEndpoint;
    monitoredPid = server.child.pid;
  }

  metricsInitial = await fetchMetrics(baseUrl);
  await sample(monitoredPid, baseUrl, samples);
  sampler = setInterval(async () => {
    if (sampling) return;
    sampling = true;
    try {
      await sample(monitoredPid, baseUrl, samples);
    } finally {
      sampling = false;
    }
  }, options.sampleIntervalMs);

  const loadResult = await runLoad(baseUrl, monitoredPid, telnetEndpoint, options);
  loadChild = undefined;
  loadStderr = loadResult.stderr;
  loadReport = parseLoadReport(loadResult.stdout);
  if (loadResult.code !== 0) {
    const detail = loadStderr.trim();
    errors.push(`load harness exited with code ${loadResult.code}${detail ? `: ${detail}` : ''}`);
  }
  if (!loadReport.success) errors.push('load harness reported one or more failures');
  if (interruptedSignal) errors.push(`soak interrupted by ${interruptedSignal}`);
} catch (error) {
  errors.push(errorMessage(error));
} finally {
  if (sampler) clearInterval(sampler);
  while (sampling) await delay(10);
  if (monitoredPid && baseUrl) {
    await sample(monitoredPid, baseUrl, samples).catch((error) => {
      errors.push(`final sample failed: ${errorMessage(error)}`);
    });
    metricsFinal = await fetchMetrics(baseUrl).catch((error) => {
      errors.push(`final metrics fetch failed: ${errorMessage(error)}`);
      return undefined;
    });
  }
  if (server) {
    const exitBeforeShutdown = server.child.exitCode;
    if (exitBeforeShutdown !== null) {
      errors.push(`server exited unexpectedly with code ${exitBeforeShutdown}`);
    } else {
      await stopServer(server.child);
    }
  }
  if (temporary) await rm(temporary, { recursive: true, force: true });
  process.removeListener('SIGINT', onSigint);
  process.removeListener('SIGTERM', onSigterm);
}

const analysis = analyzeSamples(samples, options);
errors.push(...analysis.errors);
const finishedAt = new Date();
const report = {
  schemaVersion: 2,
  success: errors.length === 0,
  evidence: {
    startedAt: startedAt.toISOString(),
    finishedAt: finishedAt.toISOString(),
    elapsedMs: Math.round(performance.now() - monotonicStart),
    commit: candidateEvidence?.commit ?? null,
    candidate: candidateEvidence,
    harness: harnessEvidence,
    runner: {
      platform: process.platform,
      release: os.release(),
      architecture: process.arch,
      node: process.versions.node,
    },
  },
  target: {
    mode: options.url ? 'existing-server' : 'temporary-server',
    url: baseUrl?.origin ?? null,
    telnet: telnetEndpoint?.display ?? null,
    pid: monitoredPid ?? null,
    binary: binaryEvidence,
    configuration: serverConfiguration,
  },
  profile: {
    durationMs: options.durationMs,
    users: options.users,
    telnetUsers: options.telnetUsers,
    concurrency: options.concurrency,
    paceMs: options.paceMs,
    sampleIntervalMs: options.sampleIntervalMs,
    warmupMs: options.warmupMs,
    limits: {
      totalRssGrowthBytes: mibibytes(options.maxRssGrowthMb),
      sustainedRssGrowthBytes: mibibytes(options.maxSustainedGrowthMb),
      rssSlopeBytesPerHour: mibibytes(options.maxRssSlopeMbPerHour),
    },
  },
  verdict: analysis.verdict,
  rss: analysis.rss,
  readiness: analysis.readiness,
  metrics: {
    initial: metricsInitial ?? null,
    final: metricsFinal ?? null,
  },
  load: loadReport ?? null,
  samples,
  errors,
  loadStderr: loadStderr || null,
};

const serialized = `${JSON.stringify(report, null, 2)}\n`;
if (options.report) {
  const destination = path.resolve(options.report);
  await mkdir(path.dirname(destination), { recursive: true });
  await writeFile(destination, serialized, { encoding: 'utf8', mode: 0o644 });
}
console.log(serialized.trimEnd());
if (!report.success) process.exitCode = interruptedSignal === 'SIGINT' ? 130 : 1;

async function startServer(directory, binaryValue, users) {
  const binary = path.resolve(binaryValue);
  await access(binary).catch(() => {
    throw new Error(`server binary does not exist: ${binary}`);
  });
  const [httpPort, telnetPort] = await reservePorts(2);
  const log = new LogTail(64 * 1024);
  const configuration = {
    storage: 'temporary-sqlite',
    worldPack: 'embedded',
    tickMillis: 1_000,
    telnetMode: 'plaintext',
    authAttemptsPerMinute: Math.max(1_000, users * 20),
    authBurst: Math.max(100, users * 4),
    commandsPerSecond: 100,
    commandBurst: 200,
    maxSessionsPerAccount: 4,
    maxSessionsPerIp: 16,
  };
  const childEnvironment = Object.fromEntries(
    Object.entries(process.env).filter(([name]) => !name.startsWith('TIAMUD_')),
  );
  const child = spawn(binary, [], {
    cwd: root,
    env: {
      ...childEnvironment,
      RUST_LOG: process.env.RUST_LOG ?? 'tiamud_server=warn',
      TIAMUD_DB_PATH: path.join(directory, 'soak.sqlite3'),
      TIAMUD_HTTP_ADDR: `127.0.0.1:${httpPort}`,
      TIAMUD_TELNET_ADDR: `127.0.0.1:${telnetPort}`,
      TIAMUD_TELNET_MODE: configuration.telnetMode,
      TIAMUD_TICK_MILLIS: String(configuration.tickMillis),
      TIAMUD_AUTH_ATTEMPTS_PER_MINUTE: String(configuration.authAttemptsPerMinute),
      TIAMUD_AUTH_BURST: String(configuration.authBurst),
      TIAMUD_COMMANDS_PER_SECOND: String(configuration.commandsPerSecond),
      TIAMUD_COMMAND_BURST: String(configuration.commandBurst),
      TIAMUD_MAX_SESSIONS_PER_ACCOUNT: String(configuration.maxSessionsPerAccount),
      TIAMUD_MAX_SESSIONS_PER_IP: String(configuration.maxSessionsPerIp),
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.on('data', (chunk) => log.append(chunk));
  child.stderr.on('data', (chunk) => log.append(chunk));
  const serverUrl = new URL(`http://127.0.0.1:${httpPort}/`);
  try {
    await waitUntilReady(serverUrl, child);
  } catch (error) {
    await stopServer(child);
    throw new Error(`${errorMessage(error)}\nServer log tail:\n${log.value}`);
  }
  return {
    child,
    baseUrl: serverUrl,
    telnetEndpoint: {
      host: '127.0.0.1',
      port: telnetPort,
      display: `127.0.0.1:${telnetPort}`,
    },
    log,
    configuration,
  };
}

async function runLoad(url, pid, telnetEndpoint, settings) {
  const arguments_ = [
    path.join(root, 'scripts', 'load.mjs'),
    '--url',
    url.origin,
    '--pid',
    String(pid),
    '--duration',
    settings.duration,
    '--users',
    String(settings.users),
    '--telnet-users',
    String(settings.telnetUsers),
    '--concurrency',
    String(settings.concurrency),
    '--pace-ms',
    String(settings.paceMs),
    '--max-rss-growth-mb',
    String(settings.maxRssGrowthMb),
  ];
  if (settings.telnetUsers > 0) {
    arguments_.push('--telnet', telnetEndpoint.display);
  }
  return new Promise((resolve, reject) => {
    let stdout = '';
    let stderr = '';
    loadChild = spawn(process.execPath, arguments_, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    loadChild.stdout.on('data', (chunk) => {
      stdout = appendBounded(stdout, chunk, 2 * 1024 * 1024);
    });
    loadChild.stderr.on('data', (chunk) => {
      stderr = appendBounded(stderr, chunk, 256 * 1024);
    });
    loadChild.once('error', reject);
    loadChild.once('exit', (code, signal) => resolve({ code: code ?? signal, stdout, stderr }));
  });
}

async function sample(pid, url, destination) {
  const [rssBytes, ready] = await Promise.all([readRssBytes(pid), probeReady(url)]);
  destination.push({
    elapsedMs: Math.round(performance.now() - monotonicStart),
    timestamp: new Date().toISOString(),
    rssBytes: rssBytes ?? null,
    ready,
  });
}

function analyzeSamples(values, settings) {
  const analysisErrors = [];
  const readyFailures = values.filter((entry) => !entry.ready).length;
  const measured = values.filter(
    (entry) => entry.elapsedMs >= settings.warmupMs && Number.isSafeInteger(entry.rssBytes),
  );
  if (readyFailures > 0) analysisErrors.push(`${readyFailures} readiness samples failed`);
  if (measured.length < 4) {
    analysisErrors.push('fewer than four post-warmup RSS samples were collected');
  }

  const allRss = values.map((entry) => entry.rssBytes).filter(Number.isSafeInteger);
  const initial = allRss.at(0) ?? null;
  const final = allRss.at(-1) ?? null;
  const peak =
    allRss.length > 0
      ? allRss.reduce((maximum, value) => Math.max(maximum, value))
      : null;
  const totalGrowth = initial === null || final === null ? null : final - initial;
  const windowSize = Math.max(1, Math.floor(measured.length / 4));
  const firstMedian = median(measured.slice(0, windowSize).map((entry) => entry.rssBytes));
  const lastMedian = median(measured.slice(-windowSize).map((entry) => entry.rssBytes));
  const sustainedGrowth =
    firstMedian === null || lastMedian === null ? null : lastMedian - firstMedian;
  const slope = regressionSlopeBytesPerHour(measured);
  const exceedsTotal =
    totalGrowth !== null && totalGrowth > mibibytes(settings.maxRssGrowthMb);
  const exceedsSustained =
    sustainedGrowth !== null && sustainedGrowth > mibibytes(settings.maxSustainedGrowthMb);
  const exceedsSlope =
    slope !== null && slope > mibibytes(settings.maxRssSlopeMbPerHour);
  const suspectedContinuousGrowth = exceedsSustained && exceedsSlope;
  if (exceedsTotal) analysisErrors.push('total RSS growth exceeded its configured limit');
  if (suspectedContinuousGrowth) {
    analysisErrors.push('post-warmup RSS shows significant sustained growth and positive trend');
  }

  return {
    errors: analysisErrors,
    verdict: {
      loadPassed: loadReport?.success === true,
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
    readiness: {
      samples: values.length,
      failures: readyFailures,
    },
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

async function fetchMetrics(baseUrl) {
  const response = await fetch(new URL('/metrics', baseUrl), {
    signal: AbortSignal.timeout(2_000),
  });
  if (!response.ok) throw new Error(`/metrics returned HTTP ${response.status}`);
  return parsePrometheus(await response.text());
}

function parsePrometheus(body) {
  const result = {};
  for (const line of body.split('\n')) {
    if (!line || line.startsWith('#')) continue;
    const match = /^(\S+)\s+(-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)$/.exec(line);
    if (!match || Object.keys(result).length >= 256) continue;
    result[match[1]] = Number(match[2]);
  }
  return result;
}

async function probeReady(baseUrl) {
  try {
    const response = await fetch(new URL('/health/ready', baseUrl), {
      signal: AbortSignal.timeout(2_000),
    });
    if (!response.ok) return false;
    const body = await response.json();
    return body?.status === 'ready' && body.world === true && body.storage?.ready === true;
  } catch {
    return false;
  }
}

async function waitUntilReady(baseUrl, child) {
  const deadline = performance.now() + 30_000;
  while (performance.now() < deadline) {
    if (child && child.exitCode !== null) {
      throw new Error(`server exited before readiness with code ${child.exitCode}`);
    }
    if (await probeReady(baseUrl)) return;
    await delay(100);
  }
  throw new Error(`server at ${baseUrl.origin} did not become ready within 30 seconds`);
}

async function readRssBytes(pid) {
  if (process.platform !== 'linux') return undefined;
  try {
    const status = await readFile(`/proc/${pid}/status`, 'utf8');
    const kibibytes = Number.parseInt(status.match(/^VmRSS:\s+(\d+)\s+kB$/m)?.[1] ?? '', 10);
    return Number.isSafeInteger(kibibytes) ? kibibytes * 1024 : undefined;
  } catch {
    return undefined;
  }
}

async function reservePorts(count) {
  const listeners = [];
  try {
    for (let index = 0; index < count; index += 1) {
      const listener = net.createServer();
      await new Promise((resolve, reject) => {
        listener.once('error', reject);
        listener.listen(0, '127.0.0.1', resolve);
      });
      listeners.push(listener);
    }
    return listeners.map((listener) => {
      const address = listener.address();
      if (!address || typeof address === 'string') throw new Error('could not reserve a port');
      return address.port;
    });
  } finally {
    await Promise.all(listeners.map((listener) => new Promise((resolve) => listener.close(resolve))));
  }
}

async function stopServer(child) {
  if (child.exitCode !== null) return;
  child.kill('SIGTERM');
  if (!(await waitForExit(child, 15_000))) {
    child.kill('SIGKILL');
    await waitForExit(child, 2_000);
  }
}

async function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null) return true;
  return new Promise((resolve) => {
    const exited = () => {
      clearTimeout(timer);
      resolve(true);
    };
    const timer = setTimeout(() => {
      child.removeListener('exit', exited);
      resolve(false);
    }, timeoutMs);
    child.once('exit', exited);
  });
}

function parseLoadReport(stdout) {
  try {
    return JSON.parse(stdout);
  } catch (error) {
    throw new Error(`could not parse load report: ${errorMessage(error)}`);
  }
}

function parseArguments(arguments_) {
  const parsed = {
    duration: '5m',
    durationMs: 300_000,
    users: 8,
    telnetUsers: 0,
    concurrency: 4,
    paceMs: 150,
    sampleIntervalMs: 10_000,
    warmupMs: 60_000,
    maxRssGrowthMb: 96,
    maxSustainedGrowthMb: 16,
    maxRssSlopeMbPerHour: 1,
    url: undefined,
    telnet: undefined,
    pid: undefined,
    binary: path.join(root, 'target', 'release', 'tiamud-server'),
    report: undefined,
    help: false,
  };
  const seen = new Set();
  const value = (name, index) => {
    const result = arguments_[index + 1];
    if (!result || result.startsWith('--')) throw new Error(`${name} requires a value`);
    return result;
  };

  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    if (argument === '--help' || argument === '-h') {
      parsed.help = true;
      continue;
    }
    if (seen.has(argument)) throw new Error(`${argument} may only be specified once`);
    seen.add(argument);
    switch (argument) {
      case '--duration': {
        parsed.duration = value(argument, index++);
        parsed.durationMs = parseDuration(parsed.duration, argument, 1_000, 86_400_000);
        break;
      }
      case '--users':
        parsed.users = boundedInteger(value(argument, index++), argument, 1, 64);
        break;
      case '--telnet-users':
        parsed.telnetUsers = boundedInteger(value(argument, index++), argument, 0, 64);
        break;
      case '--concurrency':
        parsed.concurrency = boundedInteger(value(argument, index++), argument, 1, 16);
        break;
      case '--pace-ms':
        parsed.paceMs = boundedInteger(value(argument, index++), argument, 0, 60_000);
        break;
      case '--sample-interval':
        parsed.sampleIntervalMs = parseDuration(value(argument, index++), argument, 1_000, 600_000);
        break;
      case '--warmup':
        parsed.warmupMs = parseDuration(value(argument, index++), argument, 0, 43_200_000);
        break;
      case '--max-rss-growth-mb':
        parsed.maxRssGrowthMb = boundedInteger(value(argument, index++), argument, 0, 4_096);
        break;
      case '--max-sustained-growth-mb':
        parsed.maxSustainedGrowthMb = boundedNumber(value(argument, index++), argument, 0, 4_096);
        break;
      case '--max-rss-slope-mb-per-hour':
        parsed.maxRssSlopeMbPerHour = boundedNumber(value(argument, index++), argument, 0, 4_096);
        break;
      case '--url':
        parsed.url = value(argument, index++);
        break;
      case '--telnet':
        parsed.telnet = parseTelnetEndpoint(value(argument, index++));
        break;
      case '--pid':
        parsed.pid = boundedInteger(value(argument, index++), argument, 1, 2 ** 31 - 1);
        break;
      case '--binary':
        parsed.binary = value(argument, index++);
        break;
      case '--report':
        parsed.report = value(argument, index++);
        break;
      default:
        throw new Error(`unknown argument ${argument}; use --help for usage`);
    }
  }

  if (parsed.concurrency > parsed.users) throw new Error('--concurrency cannot exceed --users');
  if (parsed.telnetUsers > parsed.users) {
    throw new Error('--telnet-users cannot exceed --users');
  }
  if (parsed.warmupMs >= parsed.durationMs) throw new Error('--warmup must be shorter than --duration');
  if (parsed.url && !parsed.pid) throw new Error('--pid is required with --url for RSS evidence');
  if (!parsed.url && seen.has('--pid')) throw new Error('--pid is only valid with --url');
  if (parsed.url && seen.has('--binary')) throw new Error('--binary cannot be combined with --url');
  if (parsed.url && parsed.telnetUsers > 0 && !parsed.telnet) {
    throw new Error('--telnet host:port is required with --url when Telnet bots are enabled');
  }
  if (!parsed.url && parsed.telnet) {
    throw new Error('--telnet is only used with --url; temporary mode supplies its own port');
  }
  if (parsed.telnet && parsed.telnetUsers === 0) {
    throw new Error('--telnet requires --telnet-users greater than zero');
  }
  return parsed;
}

function parseDuration(value, name, minimum, maximum) {
  const match = /^(\d+)(ms|s|m|h)$/.exec(value);
  if (!match) throw new Error(`${name} must use an integer with ms, s, m, or h`);
  const units = { ms: 1, s: 1_000, m: 60_000, h: 3_600_000 };
  const milliseconds = Number(match[1]) * units[match[2]];
  if (!Number.isSafeInteger(milliseconds) || milliseconds < minimum || milliseconds > maximum) {
    throw new Error(`${name} is outside its supported range`);
  }
  return milliseconds;
}

function boundedInteger(value, name, minimum, maximum) {
  if (!/^\d+$/.test(value)) throw new Error(`${name} must be an integer`);
  const result = Number(value);
  if (!Number.isSafeInteger(result) || result < minimum || result > maximum) {
    throw new Error(`${name} must be between ${minimum} and ${maximum}`);
  }
  return result;
}

function boundedNumber(value, name, minimum, maximum) {
  if (!/^(?:\d+|\d+\.\d+)$/.test(value)) throw new Error(`${name} must be a non-negative number`);
  const result = Number(value);
  if (!Number.isFinite(result) || result < minimum || result > maximum) {
    throw new Error(`${name} must be between ${minimum} and ${maximum}`);
  }
  return result;
}

function normalizeBaseUrl(value) {
  const url = new URL(value);
  if (!['http:', 'https:'].includes(url.protocol)) throw new Error('--url must use HTTP or HTTPS');
  if (url.username || url.password || url.search || url.hash) {
    throw new Error('--url must not contain credentials, a query, or a fragment');
  }
  return new URL(`${url.origin}/`);
}

function median(values) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[middle - 1] + sorted[middle]) / 2 : sorted[middle];
}

function mibibytes(value) {
  return Math.round(value * 1024 * 1024);
}

function appendBounded(current, chunk, maximum) {
  const combined = current + chunk.toString('utf8');
  return Buffer.byteLength(combined) <= maximum ? combined : combined.slice(-maximum);
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function requireNode24() {
  const major = Number.parseInt(process.versions.node.split('.')[0], 10);
  if (major < 24) throw new Error(`Node 24 or newer is required; found ${process.versions.node}`);
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function printHelp() {
  console.log(`Tiamud retained soak-evidence runner (Node 24, Linux RSS)

Usage:
  node scripts/soak.mjs [options]

Target:
  --binary <path>                    Start this server binary with temporary state.
                                     Default: target/release/tiamud-server
  --url <http(s)://host> --pid <pid> Exercise an existing server and sample its RSS.
  --telnet <host:port>                Plaintext target required with --url when
                                      Telnet bots are enabled.
  --report <path>                    Also write the complete JSON report to this path.

Workload:
  --duration <1s..24h>               Default: 5m
  --users <1..64>                    Default: 8
  --telnet-users <0..users>          Real plaintext Telnet users; default: 0
  --concurrency <1..16>              Default: 4
  --pace-ms <0..60000>               Default: 150

Memory evidence:
  --sample-interval <1s..10m>        Default: 10s
  --warmup <0ms..12h>                Excluded from trend analysis; default: 1m
  --max-rss-growth-mb <0..4096>      Initial-to-final limit; default: 96
  --max-sustained-growth-mb <number> First/last window median limit; default: 16
  --max-rss-slope-mb-per-hour <num>  Regression limit; default: 1

The report retains the exact profile, load-harness result, readiness checks,
Prometheus snapshots, periodic RSS samples, window medians, and post-warmup
regression. Continuous growth fails only when both the sustained-growth and
slope limits are exceeded; total RSS growth has its own independent limit.

Short evidence profile:
  node scripts/soak.mjs --binary target/debug/tiamud-server --duration 30s \\
    --warmup 5s --sample-interval 5s --users 2 --concurrency 2 \\
    --telnet-users 1 --report artifacts/soak-short.json

24-hour acceptance profile (an invocation is not evidence until it completes):
  node scripts/soak.mjs --duration 24h --warmup 30m --sample-interval 1m \\
    --users 16 --telnet-users 4 --concurrency 4 --pace-ms 150 \\
    --report artifacts/soak-24h.json
`);
}
