#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDirectory, "..");
const webRoot = join(projectRoot, "apps", "web");
const noticePath = join(projectRoot, "THIRD_PARTY_NOTICES.md");
const policyPath = join(scriptDirectory, "third-party-license-policy.json");
const checkOnly = process.argv.length === 3 && process.argv[2] === "--check";

if (process.argv.length > (checkOnly ? 3 : 2)) {
  fail("usage: node scripts/generate-third-party-notices.mjs [--check]");
}
if (process.argv.length === 3 && !checkOnly) {
  fail("usage: node scripts/generate-third-party-notices.mjs [--check]");
}

const policy = JSON.parse(readFileSync(policyPath, "utf8"));
if (policy.schemaVersion !== 1 || typeof policy.releaseTarget !== "string") {
  fail(`unsupported third-party license policy: ${relative(projectRoot, policyPath)}`);
}

const cargoMetadata = commandJson(
  "cargo",
  [
    "metadata",
    "--locked",
    "--format-version",
    "1",
    "--filter-platform",
    policy.releaseTarget,
    "--features",
    "world-pack/editorial-ai",
  ],
  projectRoot,
);
const pnpmLicenses = commandJson(
  "pnpm",
  ["licenses", "list", "--json"],
  webRoot,
);
const pnpmGraph = commandJson(
  "pnpm",
  ["list", "--json", "--depth", "Infinity"],
  webRoot,
);

const rustPackages = collectRustPackages(cargoMetadata);
const webPackages = collectWebPackages(pnpmLicenses);
validateWebGraph(pnpmGraph, webPackages);
validatePolicy("rust", rustPackages);
validatePolicy("web", webPackages);

const noticeGroups = new Map();
collectPackageNotices("rust", rustPackages, noticeGroups);
collectPackageNotices("web", webPackages, noticeGroups);

const rendered = renderNotice(rustPackages, webPackages, noticeGroups);
if (checkOnly) {
  let current;
  try {
    current = readFileSync(noticePath, "utf8");
  } catch (error) {
    fail(`cannot read ${relative(projectRoot, noticePath)}: ${error.message}`);
  }
  if (current !== rendered) {
    fail(
      "THIRD_PARTY_NOTICES.md is stale; run `node scripts/generate-third-party-notices.mjs` and review the result",
    );
  }
  process.stdout.write(
    `verified ${rustPackages.length} Rust and ${webPackages.length} web dependencies in THIRD_PARTY_NOTICES.md\n`,
  );
} else {
  writeFileSync(noticePath, rendered, "utf8");
  process.stdout.write(
    `wrote THIRD_PARTY_NOTICES.md for ${rustPackages.length} Rust and ${webPackages.length} web dependencies\n`,
  );
}

function commandJson(command, args, cwd) {
  try {
    return JSON.parse(
      execFileSync(command, args, {
        cwd,
        encoding: "utf8",
        maxBuffer: 128 * 1024 * 1024,
        stdio: ["ignore", "pipe", "inherit"],
      }),
    );
  } catch (error) {
    fail(`${command} failed while resolving the locked dependency graph: ${error.message}`);
  }
}

function collectRustPackages(metadata) {
  const packageById = new Map(metadata.packages.map((entry) => [entry.id, entry]));
  const nodeById = new Map(metadata.resolve.nodes.map((entry) => [entry.id, entry]));
  const reachable = new Set();
  const pending = [...metadata.workspace_members];

  while (pending.length > 0) {
    const id = pending.pop();
    if (reachable.has(id)) continue;
    reachable.add(id);
    const node = nodeById.get(id);
    if (!node) fail(`Cargo metadata is missing a resolve node for ${id}`);
    for (const dependency of node.deps) {
      const isReleaseDependency = dependency.dep_kinds.some(
        (kind) => kind.kind !== "dev",
      );
      if (isReleaseDependency && !reachable.has(dependency.pkg)) {
        pending.push(dependency.pkg);
      }
    }
  }

  return [...reachable]
    .map((id) => packageById.get(id))
    .filter((entry) => entry?.source)
    .map((entry) => ({
      name: entry.name,
      version: entry.version,
      license: requiredString(entry.license, `Cargo package ${entry.name}@${entry.version} license`),
      source:
        normalizeRepository(entry.repository) ||
        normalizeRepository(entry.homepage) ||
        cratesIoUrl(entry.name, entry.version),
      directory: dirname(entry.manifest_path),
      explicitLicenseFile: entry.license_file
        ? resolve(dirname(entry.manifest_path), entry.license_file)
        : null,
    }))
    .sort(comparePackages);
}

function collectWebPackages(groupedLicenses) {
  const packages = new Map();
  for (const [declaredLicense, entries] of Object.entries(groupedLicenses)) {
    for (const entry of entries) {
      for (const packagePath of entry.paths || []) {
        const manifest = JSON.parse(readFileSync(join(packagePath, "package.json"), "utf8"));
        const name = requiredString(manifest.name, `web package name at ${packagePath}`);
        const version = requiredString(manifest.version, `web package ${name} version`);
        const key = `${name}@${version}`;
        const record = {
          name,
          version,
          license: requiredString(
            declaredLicense || manifest.license,
            `web package ${key} license`,
          ),
          source:
            normalizeRepository(manifest.repository) ||
            normalizeRepository(entry.homepage) ||
            normalizeRepository(manifest.homepage) ||
            `https://www.npmjs.com/package/${encodeURIComponent(name)}/v/${encodeURIComponent(version)}`,
          directory: realpathSync(packagePath),
          explicitLicenseFile: null,
        };
        const existing = packages.get(key);
        if (existing && existing.license !== record.license) {
          fail(`conflicting licenses reported for web package ${key}`);
        }
        packages.set(key, existing || record);
      }
    }
  }
  return [...packages.values()].sort(comparePackages);
}

function validatePolicy(ecosystem, packages) {
  const approved = new Set(
    policy.approvedDeclaredLicenseExpressions?.[ecosystem] || [],
  );
  const observed = new Set(packages.map((entry) => entry.license));
  const unapproved = [...observed].filter((entry) => !approved.has(entry)).sort();
  if (unapproved.length > 0) {
    fail(
      `${ecosystem} dependencies contain unreviewed license expression(s): ${unapproved.join(", ")}`,
    );
  }
}

function validateWebGraph(graph, licensedPackages) {
  if (!Array.isArray(graph) || graph.length !== 1) {
    fail("pnpm list did not return exactly one workspace root");
  }
  const installed = new Set();

  function visit(parent) {
    for (const dependencyKind of [
      "dependencies",
      "devDependencies",
      "optionalDependencies",
    ]) {
      for (const [name, dependency] of Object.entries(parent[dependencyKind] || {})) {
        if (!dependency.path || !existsSync(dependency.path)) continue;
        const key = `${name}@${requiredString(dependency.version, `${name} version`)}`;
        installed.add(key);
        visit(dependency);
      }
    }
  }

  visit(graph[0]);
  const noticed = new Set(
    licensedPackages.map((entry) => `${entry.name}@${entry.version}`),
  );
  const missing = [...installed].filter((entry) => !noticed.has(entry)).sort();
  const extra = [...noticed].filter((entry) => !installed.has(entry)).sort();
  if (missing.length > 0 || extra.length > 0) {
    fail(
      `pnpm license inventory does not match the installed frozen graph; missing=[${missing.join(", ")}], extra=[${extra.join(", ")}]`,
    );
  }
}

function collectPackageNotices(ecosystem, packages, groups) {
  const missing = [];
  for (const packageEntry of packages) {
    const packageKey = `${packageEntry.name}@${packageEntry.version}`;
    const files = findNoticeFiles(
      packageEntry.directory,
      packageEntry.explicitLicenseFile,
    );
    const supplementalFiles =
      policy.supplementalNoticeFiles?.[ecosystem]?.[packageKey] || [];
    for (const supplemental of supplementalFiles) {
      if (
        typeof supplemental.path !== "string" ||
        !supplemental.path.startsWith("third-party-notices/") ||
        typeof supplemental.sha256 !== "string" ||
        typeof supplemental.source !== "string" ||
        typeof supplemental.review !== "string"
      ) {
        fail(`invalid supplemental notice policy for ${ecosystem} package ${packageKey}`);
      }
      const path = resolve(scriptDirectory, supplemental.path);
      const content = normalizeNoticeText(readFileSync(path));
      const digest = sha256(Buffer.from(content, "utf8"));
      if (digest !== supplemental.sha256) {
        fail(
          `supplemental notice digest mismatch for ${ecosystem} package ${packageKey}: ${supplemental.path}`,
        );
      }
      files.push({
        path,
        label: `scripts/${supplemental.path}`,
        source: supplemental.source,
        review: supplemental.review,
      });
    }
    if (files.length === 0) {
      missing.push(packageKey);
      continue;
    }
    for (const file of files) {
      const content = normalizeNoticeText(readFileSync(file.path));
      const digest = sha256(Buffer.from(content, "utf8"));
      const group = groups.get(digest) || {
        digest,
        content,
        filenames: new Set(),
        sources: new Set(),
        reviews: new Set(),
        packages: new Set(),
      };
      group.filenames.add(file.label);
      if (file.source) group.sources.add(file.source);
      if (file.review) group.reviews.add(file.review);
      group.packages.add(`${ecosystem}:${packageKey}`);
      groups.set(digest, group);
    }
  }
  if (missing.length > 0) {
    fail(
      `${ecosystem} dependencies without discoverable license/notice files: ${missing.sort().join(", ")}`,
    );
  }
}

function findNoticeFiles(packageDirectory, explicitLicenseFile) {
  const matches = new Map();
  if (explicitLicenseFile) {
    if (!statSync(explicitLicenseFile).isFile()) {
      fail(`declared license file is not a regular file: ${explicitLicenseFile}`);
    }
    matches.set(realpathSync(explicitLicenseFile), relative(packageDirectory, explicitLicenseFile));
  }

  const pending = [packageDirectory];
  while (pending.length > 0) {
    const directory = pending.pop();
    const entries = readdirSync(directory, { withFileTypes: true }).sort((left, right) =>
      left.name.localeCompare(right.name, "en"),
    );
    for (const entry of entries) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== "node_modules" && entry.name !== ".git") pending.push(path);
      } else if (
        (entry.isFile() || entry.isSymbolicLink()) &&
        /^(?:LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|PATENTS)(?:$|[._-])/i.test(entry.name)
      ) {
        const resolved = entry.isSymbolicLink() ? realpathSync(path) : path;
        if (statSync(resolved).isFile()) {
          matches.set(resolved, relative(packageDirectory, path));
        }
      }
    }
  }
  return [...matches.entries()]
    .map(([path, label]) => ({ path, label, source: null, review: null }))
    .sort((left, right) => left.label.localeCompare(right.label, "en"));
}

function normalizeNoticeText(buffer) {
  if (buffer.includes(0)) fail("license/notice file contains NUL bytes");
  const text = buffer
    .toString("utf8")
    .replaceAll("\r\n", "\n")
    .replaceAll("\r", "\n")
    .split("\n")
    .map((line) => line.replaceAll("\t", "    ").trimEnd())
    .join("\n");
  return `${text.trimEnd()}\n`;
}

function renderNotice(rustPackages, webPackages, groups) {
  const cargoDigest = sha256(readFileSync(join(projectRoot, "Cargo.lock")));
  const pnpmDigest = sha256(readFileSync(join(webRoot, "pnpm-lock.yaml")));
  const lines = [
    "<!-- Generated by scripts/generate-third-party-notices.mjs. Do not edit by hand. -->",
    "",
    "# Third-Party Notices",
    "",
    "Tiamud distributions contain or are produced with the third-party components",
    "listed below. This inventory is generated deterministically from the locked",
    "release dependency graphs and includes the license and notice texts shipped by",
    "their packages. It does not replace the terms of those licenses.",
    "",
    `- Release target: \`${policy.releaseTarget}\``,
    `- Cargo lockfile SHA-256: \`${cargoDigest}\``,
    `- pnpm lockfile SHA-256: \`${pnpmDigest}\``,
    `- Rust release dependencies: ${rustPackages.length}`,
    `- Web build/runtime dependencies: ${webPackages.length}`,
    "",
    "Regenerate this file with `node scripts/generate-third-party-notices.mjs` and",
    "verify it with `node scripts/generate-third-party-notices.mjs --check`. License",
    "expressions must also be explicitly admitted by",
    "`scripts/third-party-license-policy.json`; a new expression or a missing notice",
    "file fails generation pending human review.",
    "",
    "## Rust release dependencies",
    "",
    "| Package | Declared license | Upstream |",
    "| --- | --- | --- |",
    ...rustPackages.map(renderPackageRow),
    "",
    "## Web build/runtime dependencies",
    "",
    "The web inventory is conservative: it includes the complete frozen pnpm graph",
    "used to build and verify the browser bundle, including build and test tooling.",
    "",
    "| Package | Declared license | Upstream |",
    "| --- | --- | --- |",
    ...webPackages.map(renderPackageRow),
    "",
    "## License and notice texts",
    "",
  ];

  const sortedGroups = [...groups.values()].sort((left, right) =>
    left.digest.localeCompare(right.digest, "en"),
  );
  for (const group of sortedGroups) {
    lines.push(`### \`sha256:${group.digest}\``, "", "Applies to:", "");
    for (const packageKey of [...group.packages].sort()) {
      const [ecosystem, identifier] = packageKey.split(":", 2);
      lines.push(`- ${ecosystem === "rust" ? "Rust" : "Web"} \`${escapeInline(identifier)}\``);
    }
    lines.push(
      "",
      `Source filename(s): ${[...group.filenames]
        .sort()
        .map((entry) => `\`${escapeInline(entry)}\``)
        .join(", ")}`,
    );
    if (group.sources.size > 0) {
      lines.push(
        "",
        `Supplemental source(s): ${[...group.sources]
          .sort()
          .map((entry) => `<${escapeTable(entry)}>`)
          .join(", ")}`,
      );
    }
    if (group.reviews.size > 0) {
      lines.push("", "Supplemental review:", "");
      for (const review of [...group.reviews].sort()) {
        lines.push(`- ${review}`);
      }
    }
    lines.push(
      "",
      ...group.content
        .trimEnd()
        .split("\n")
        .map((line) => (line === "" ? "" : `    ${line}`)),
      "",
    );
  }
  return `${lines.join("\n").trimEnd()}\n`;
}

function renderPackageRow(entry) {
  const packageName = escapeTable(`${entry.name} ${entry.version}`);
  const license = escapeTable(entry.license);
  const source = escapeTable(entry.source);
  return `| \`${packageName}\` | \`${license}\` | <${source}> |`;
}

function normalizeRepository(repository) {
  if (typeof repository === "string") {
    let value = repository.trim();
    if (/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value)) {
      value = `https://github.com/${value}`;
    } else if (value.startsWith("github:")) {
      value = `https://github.com/${value.slice("github:".length)}`;
    } else if (value.startsWith("gitlab:")) {
      value = `https://gitlab.com/${value.slice("gitlab:".length)}`;
    } else if (value.startsWith("bitbucket:")) {
      value = `https://bitbucket.org/${value.slice("bitbucket:".length)}`;
    }
    return value
      .replace(/^git\+/, "")
      .replace(/^git:\/\/github\.com\//, "https://github.com/")
      .replace(/^git@github\.com:/, "https://github.com/")
      .replace(/^ssh:\/\/git@github\.com\//, "https://github.com/")
      .replace(/^http:\/\/github\.com\//, "https://github.com/")
      .replace(/\.git$/, "");
  }
  if (repository && typeof repository.url === "string") {
    return normalizeRepository(repository.url);
  }
  return null;
}

function cratesIoUrl(name, version) {
  return `https://crates.io/crates/${encodeURIComponent(name)}/${encodeURIComponent(version)}`;
}

function requiredString(value, label) {
  if (typeof value !== "string" || value.trim() === "") fail(`${label} is missing`);
  return value.trim();
}

function comparePackages(left, right) {
  return (
    left.name.localeCompare(right.name, "en") ||
    left.version.localeCompare(right.version, "en")
  );
}

function escapeTable(value) {
  return String(value).replaceAll("|", "\\|").replaceAll("\n", " ");
}

function escapeInline(value) {
  return String(value).replaceAll("`", "\\`");
}

function sha256(content) {
  return createHash("sha256").update(content).digest("hex");
}

function fail(message) {
  process.stderr.write(`third-party notices: ${message}\n`);
  process.exit(1);
}
