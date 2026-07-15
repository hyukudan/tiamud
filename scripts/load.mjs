#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { access, mkdtemp, readFile, rm } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';

import { provisionBot } from './bot.mjs';

const root = path.resolve(import.meta.dirname, '..');
const MAX_LOG_TAIL_BYTES = 64 * 1024;
const RSS_SAMPLE_INTERVAL_MS = 1_000;

class Metrics {
  latencies = new Map();
  errors = [];
  warnings = [];
  bootstrapCompleted = 0;
  roundsCompleted = 0;

  recordLatency(name, milliseconds) {
    const samples = this.latencies.get(name) ?? [];
    samples.push(milliseconds);
    this.latencies.set(name, samples);
  }

  recordError(phase, error) {
    this.errors.push({ phase, message: errorMessage(error) });
  }

  latencyReport() {
    return Object.fromEntries(
      [...this.latencies.entries()]
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([name, samples]) => [name, summarize(samples)]),
    );
  }
}

class LogTail {
  #maximum;
  value = '';

  constructor(maximum) {
    this.#maximum = maximum;
  }

  append(chunk) {
    this.value += chunk.toString('utf8');
    if (Buffer.byteLength(this.value) > this.#maximum) {
      this.value = this.value.slice(-this.#maximum);
    }
  }
}

const options = parseArguments(process.argv.slice(2));

if (options.help) {
  printHelp();
  process.exit(0);
}

requireNode24();

const metrics = new Metrics();
const startedAt = performance.now();
const temporary = options.url ? undefined : await mkdtemp(path.join(os.tmpdir(), 'tiamud-load-'));
let server;
let bots = [];
let rssSampler;
let rssInitial;
let rssFinal;
let rssPeak;
let monitoredPid = options.pid;
let interruptedSignal;

const interrupt = (signal) => {
  interruptedSignal ??= signal;
};
const onSigint = () => interrupt('SIGINT');
const onSigterm = () => interrupt('SIGTERM');
process.once('SIGINT', onSigint);
process.once('SIGTERM', onSigterm);

try {
  let baseUrl;
  if (options.url) {
    baseUrl = normalizeBaseUrl(options.url);
    await waitUntilReady(baseUrl);
  } else {
    server = await startTemporaryServer(temporary, options);
    baseUrl = server.baseUrl;
    monitoredPid = server.child.pid;
  }

  rssInitial = await readRssBytes(monitoredPid);
  rssPeak = rssInitial;
  if (monitoredPid && rssInitial !== undefined) {
    rssSampler = setInterval(async () => {
      const sample = await readRssBytes(monitoredPid);
      if (sample !== undefined) rssPeak = Math.max(rssPeak ?? sample, sample);
    }, RSS_SAMPLE_INTERVAL_MS);
    rssSampler.unref();
  }

  const runId = `${Date.now().toString(36)}-${process.pid.toString(36)}`;
  bots = await mapLimit(
    Array.from({ length: options.users }, (_, index) => index + 1),
    options.concurrency,
    async (index) => {
      throwIfInterrupted();
      const bot = await provisionBot(baseUrl, runId, index, (name, milliseconds) =>
        metrics.recordLatency(name, milliseconds),
      );
      bot.paceMs = options.paceMs;
      return bot;
    },
  );

  await mapLimit(bots, options.concurrency, async (bot) => {
    throwIfInterrupted();
    await bot.bootstrap();
    metrics.bootstrapCompleted += 1;
  });

  if (options.durationMs !== undefined) {
    const deadline = performance.now() + options.durationMs;
    while (performance.now() < deadline) {
      throwIfInterrupted();
      await mapLimit(bots, options.concurrency, async (bot) => {
        if (performance.now() >= deadline) return;
        await bot.runRound();
        metrics.roundsCompleted += 1;
      });
    }
  } else {
    for (let round = 0; round < options.rounds; round += 1) {
      throwIfInterrupted();
      await mapLimit(bots, options.concurrency, async (bot) => {
        await bot.runRound();
        metrics.roundsCompleted += 1;
      });
    }
  }
} catch (error) {
  metrics.recordError('harness', error);
} finally {
  if (rssSampler) clearInterval(rssSampler);
  await Promise.allSettled(bots.map((bot) => bot.close()));
  await delay(100);
  rssFinal = await readRssBytes(monitoredPid);
  if (rssFinal !== undefined) rssPeak = Math.max(rssPeak ?? rssFinal, rssFinal);
  if (server && server.child.exitCode !== null) {
    metrics.recordError(
      'server',
      new Error(`Temporary server exited unexpectedly with code ${server.child.exitCode}.`),
    );
  }
  if (server) await stopServer(server.child);
  if (temporary) await rm(temporary, { recursive: true, force: true });
  process.removeListener('SIGINT', onSigint);
  process.removeListener('SIGTERM', onSigterm);
}

const rssGrowth =
  rssInitial === undefined || rssFinal === undefined ? undefined : rssFinal - rssInitial;
const rssLimitBytes = options.maxRssGrowthMb * 1024 * 1024;
const suspectedLeak = rssGrowth !== undefined && rssGrowth > rssLimitBytes;
if (suspectedLeak) {
  metrics.recordError(
    'rss',
    new Error(
      `RSS grew by ${formatBytes(rssGrowth)}, above the ${formatBytes(rssLimitBytes)} limit.`,
    ),
  );
}
if (options.url && !monitoredPid) {
  metrics.warnings.push('Server RSS unavailable for --url mode; pass --pid to enable leak checks.');
} else if (monitoredPid && (rssInitial === undefined || rssFinal === undefined)) {
  metrics.warnings.push(`Could not read Linux RSS for server pid ${monitoredPid}.`);
}

const report = {
  success: metrics.errors.length === 0,
  mode: options.url ? 'existing-server' : 'temporary-server',
  users: options.users,
  concurrency: options.concurrency,
  paceMs: options.paceMs,
  requested:
    options.durationMs === undefined
      ? { roundsPerUser: options.rounds }
      : { durationMs: options.durationMs },
  completed: {
    bootstraps: metrics.bootstrapCompleted,
    rounds: metrics.roundsCompleted,
  },
  elapsedMs: rounded(performance.now() - startedAt),
  latenciesMs: metrics.latencyReport(),
  rss: {
    pid: monitoredPid ?? null,
    initialBytes: rssInitial ?? null,
    finalBytes: rssFinal ?? null,
    peakBytes: rssPeak ?? null,
    growthBytes: rssGrowth ?? null,
    allowedGrowthBytes: rssLimitBytes,
    suspectedLeak,
  },
  errors: metrics.errors,
  warnings: metrics.warnings,
};

console.log(JSON.stringify(report, null, 2));
if (!report.success) {
  if (server?.logTail) console.error(`\nServer log tail:\n${server.logTail.value}`);
  process.exitCode = interruptedSignal === 'SIGINT' ? 130 : 1;
}

async function startTemporaryServer(directory, settings) {
  const binary = settings.binary
    ? path.resolve(settings.binary)
    : path.join(root, 'target', 'debug', process.platform === 'win32' ? 'tiamud-server.exe' : 'tiamud-server');
  if (!settings.binary && !settings.skipBuild) {
    await runCommand('cargo', ['build', '--locked', '-p', 'tiamud-server', '--bin', 'tiamud-server']);
  }
  await access(binary).catch(() => {
    throw new Error(
      `Server binary ${binary} does not exist; remove --skip-build or pass --binary <path>.`,
    );
  });

  const [httpPort, telnetPort] = await reservePorts(2);
  const logTail = new LogTail(MAX_LOG_TAIL_BYTES);
  const child = spawn(binary, [], {
    cwd: root,
    env: {
      ...process.env,
      RUST_LOG: process.env.RUST_LOG ?? 'tiamud_server=warn',
      TIAMUD_DB_PATH: path.join(directory, 'load.sqlite3'),
      TIAMUD_HTTP_ADDR: `127.0.0.1:${httpPort}`,
      TIAMUD_TELNET_ADDR: `127.0.0.1:${telnetPort}`,
      TIAMUD_TELNET_MODE: 'plaintext',
      TIAMUD_AUTH_ATTEMPTS_PER_MINUTE: String(Math.max(1_000, settings.users * 20)),
      TIAMUD_AUTH_BURST: String(Math.max(100, settings.users * 4)),
      TIAMUD_COMMANDS_PER_SECOND: '100',
      TIAMUD_COMMAND_BURST: '200',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.on('data', (chunk) => logTail.append(chunk));
  child.stderr.on('data', (chunk) => logTail.append(chunk));
  const baseUrl = new URL(`http://127.0.0.1:${httpPort}/`);
  try {
    await waitUntilReady(baseUrl, child);
    return { child, baseUrl, logTail };
  } catch (error) {
    await stopServer(child);
    throw new Error(`${errorMessage(error)}\nServer log tail:\n${logTail.value}`);
  }
}

async function waitUntilReady(baseUrl, child) {
  const deadline = performance.now() + 30_000;
  while (performance.now() < deadline) {
    if (child && child.exitCode !== null) {
      throw new Error(`Server exited before readiness with code ${child.exitCode}.`);
    }
    try {
      const response = await fetch(new URL('/health/ready', baseUrl), {
        signal: AbortSignal.timeout(1_000),
      });
      if (response.ok) {
        const body = await response.json();
        if (
          body?.status === 'ready' &&
          body.world === true &&
          body.storage?.ready === true
        ) {
          return;
        }
      }
    } catch {
      // A temporary connection failure is expected while the listener starts.
    }
    await delay(50);
  }
  throw new Error(`Server at ${baseUrl} did not become ready within 30 seconds.`);
}

async function stopServer(child) {
  if (child.exitCode !== null) return;
  child.kill('SIGTERM');
  await Promise.race([onceExit(child), delay(5_000)]);
  if (child.exitCode === null) {
    child.kill('SIGKILL');
    await Promise.race([onceExit(child), delay(2_000)]);
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
      if (!address || typeof address === 'string') throw new Error('Could not reserve a local port.');
      return address.port;
    });
  } finally {
    await Promise.all(
      listeners.map((listener) => new Promise((resolve) => listener.close(resolve))),
    );
  }
}

async function runCommand(command, arguments_) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, arguments_, { cwd: root, stdio: 'inherit' });
    child.once('error', reject);
    child.once('exit', (code, signal) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} exited with ${code ?? signal}.`));
    });
  });
}

async function readRssBytes(pid) {
  if (!pid || process.platform !== 'linux') return undefined;
  try {
    const status = await readFile(`/proc/${pid}/status`, 'utf8');
    const kibibytes = Number.parseInt(status.match(/^VmRSS:\s+(\d+)\s+kB$/m)?.[1] ?? '', 10);
    return Number.isSafeInteger(kibibytes) ? kibibytes * 1024 : undefined;
  } catch {
    return undefined;
  }
}

async function mapLimit(values, limit, operation) {
  const results = new Array(values.length);
  let cursor = 0;
  const workers = Array.from({ length: Math.min(limit, values.length) }, async () => {
    for (;;) {
      const index = cursor;
      cursor += 1;
      if (index >= values.length) return;
      results[index] = await operation(values[index], index);
    }
  });
  await Promise.all(workers);
  return results;
}

function summarize(samples) {
  const sorted = [...samples].sort((left, right) => left - right);
  return {
    count: sorted.length,
    p50: rounded(percentile(sorted, 0.5)),
    p95: rounded(percentile(sorted, 0.95)),
    p99: rounded(percentile(sorted, 0.99)),
    max: rounded(sorted.at(-1) ?? 0),
  };
}

function percentile(sorted, quantile) {
  if (sorted.length === 0) return 0;
  return sorted[Math.ceil(sorted.length * quantile) - 1];
}

function parseArguments(arguments_) {
  const parsed = {
    users: 4,
    rounds: undefined,
    durationMs: undefined,
    concurrency: undefined,
    paceMs: 100,
    maxRssGrowthMb: 64,
    url: undefined,
    pid: undefined,
    binary: undefined,
    skipBuild: false,
    help: false,
  };
  const seen = new Set();
  const value = (name, index) => {
    const result = arguments_[index + 1];
    if (!result || result.startsWith('--')) throw new Error(`${name} requires a value.`);
    return result;
  };

  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    if (argument === '--help' || argument === '-h') {
      parsed.help = true;
      continue;
    }
    if (seen.has(argument)) throw new Error(`${argument} may only be specified once.`);
    seen.add(argument);
    switch (argument) {
      case '--users':
        parsed.users = boundedInteger(value(argument, index++), argument, 1, 64);
        break;
      case '--rounds':
        parsed.rounds = boundedInteger(value(argument, index++), argument, 1, 100_000);
        break;
      case '--duration':
        parsed.durationMs = parseDuration(value(argument, index++));
        break;
      case '--concurrency':
        parsed.concurrency = boundedInteger(value(argument, index++), argument, 1, 16);
        break;
      case '--pace-ms':
        parsed.paceMs = boundedInteger(value(argument, index++), argument, 0, 60_000);
        break;
      case '--max-rss-growth-mb':
        parsed.maxRssGrowthMb = boundedInteger(value(argument, index++), argument, 0, 4_096);
        break;
      case '--url':
        parsed.url = value(argument, index++);
        break;
      case '--pid':
        parsed.pid = boundedInteger(value(argument, index++), argument, 1, 2 ** 31 - 1);
        break;
      case '--binary':
        parsed.binary = value(argument, index++);
        break;
      case '--skip-build':
        parsed.skipBuild = true;
        break;
      default:
        throw new Error(`Unknown argument ${argument}. Use --help for usage.`);
    }
  }

  if (parsed.rounds !== undefined && parsed.durationMs !== undefined) {
    throw new Error('--rounds and --duration are mutually exclusive.');
  }
  parsed.rounds ??= parsed.durationMs === undefined ? 3 : undefined;
  parsed.concurrency ??= Math.min(4, parsed.users);
  if (parsed.concurrency > parsed.users) throw new Error('--concurrency cannot exceed --users.');
  if (!parsed.url && parsed.pid) throw new Error('--pid is only used with --url.');
  if (parsed.url && (parsed.binary || parsed.skipBuild)) {
    throw new Error('--binary and --skip-build cannot be combined with --url.');
  }
  return parsed;
}

function parseDuration(value) {
  const match = /^(\d+)(ms|s|m|h)$/.exec(value);
  if (!match) throw new Error('--duration must use an integer with ms, s, m, or h.');
  const units = { ms: 1, s: 1_000, m: 60_000, h: 3_600_000 };
  const milliseconds = Number(match[1]) * units[match[2]];
  if (!Number.isSafeInteger(milliseconds) || milliseconds < 1_000 || milliseconds > 86_400_000) {
    throw new Error('--duration must be between 1s and 24h.');
  }
  return milliseconds;
}

function boundedInteger(value, name, minimum, maximum) {
  if (!/^\d+$/.test(value)) throw new Error(`${name} must be an integer.`);
  const integer = Number(value);
  if (!Number.isSafeInteger(integer) || integer < minimum || integer > maximum) {
    throw new Error(`${name} must be between ${minimum} and ${maximum}.`);
  }
  return integer;
}

function normalizeBaseUrl(value) {
  const url = new URL(value);
  if (!['http:', 'https:'].includes(url.protocol)) throw new Error('--url must use http or https.');
  if (url.username || url.password || url.search || url.hash) {
    throw new Error('--url must not contain credentials, a query, or a fragment.');
  }
  return new URL(`${url.origin}/`);
}

function requireNode24() {
  const major = Number.parseInt(process.versions.node.split('.')[0], 10);
  if (major < 24) throw new Error(`Node 24 or newer is required; found ${process.versions.node}.`);
}

function throwIfInterrupted() {
  if (interruptedSignal) throw new Error(`Interrupted by ${interruptedSignal}.`);
}

function onceExit(child) {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolve) => child.once('exit', resolve));
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function rounded(value) {
  return Math.round(value * 100) / 100;
}

function formatBytes(value) {
  return `${(value / (1024 * 1024)).toFixed(2)} MiB`;
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function printHelp() {
  console.log(`Tiamud headless load and soak harness (Node 24, no external dependencies)

Usage:
  node scripts/load.mjs [options]

Server mode:
  By default the harness builds and starts tiamud-server with a temporary SQLite
  database and temporary HTTP/Telnet ports, then stops and removes it.

  --url <http(s)://host>       Target an already-running server instead.
  --pid <pid>                  Measure an existing server's Linux RSS in --url mode.
  --binary <path>              Start this prebuilt server binary; skips cargo build.
  --skip-build                 Use target/debug/tiamud-server without rebuilding.

Bounded workload:
  --users <1..64>              Users and characters to create (default: 4).
  --rounds <1..100000>         Rounds per user (default: 3).
  --duration <1s..24h>         Run until a duration instead of --rounds; units ms/s/m/h.
  --concurrency <1..16>        Maximum simultaneous bots (default: min(users, 4)).
  --pace-ms <0..60000>         Minimum delay between each bot's commands (default: 100).
  --max-rss-growth-mb <0..4096>
                               Fail if final server RSS growth exceeds this (default: 64).
  -h, --help                   Show this help.

Each bot provisions a registered account and character, negotiates only tiamud.v1,
validates every server frame strictly, checks opaque per-view IDs, moves north,
replays the same requestId to prove idempotence, requests sync.snapshot, reconnects,
then exercises south/north/look. Output includes p50/p95/p99/max latency, errors,
and initial/final/peak Linux RSS. Any protocol/action error or suspected RSS leak
returns a non-zero exit status.

Short local profile:
  node scripts/load.mjs --users 2 --rounds 2 --concurrency 2

Existing server (RSS checking enabled when its PID is supplied):
  node scripts/load.mjs --url http://127.0.0.1:8080 --pid 1234 --users 4 --rounds 10

Manual 24-hour soak recipe (this command is documented, not claimed as executed):
  node scripts/load.mjs --duration 24h --users 16 --concurrency 4 --pace-ms 150
`);
}
