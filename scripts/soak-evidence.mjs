import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

export async function fileEvidence(value) {
  const bytes = await readFile(value);
  return {
    bytes: bytes.byteLength,
    sha256: createHash('sha256').update(bytes).digest('hex'),
  };
}

export async function collectCandidateEvidence(root) {
  const [{ stdout: commitOutput }, { stdout: treeOutput }, { stdout: statusOutput }] =
    await Promise.all([
      execFileAsync('git', ['rev-parse', '--verify', 'HEAD^{commit}'], {
        cwd: root,
        encoding: 'utf8',
      }),
      execFileAsync('git', ['rev-parse', '--verify', 'HEAD^{tree}'], {
        cwd: root,
        encoding: 'utf8',
      }),
      execFileAsync('git', ['status', '--porcelain=v1', '--untracked-files=normal'], {
        cwd: root,
        encoding: 'utf8',
      }),
    ]);
  const commit = commitOutput.trim();
  const tree = treeOutput.trim();
  const environmentCommit = process.env.GITHUB_SHA;
  const validEnvironmentCommit = environmentCommit === undefined || gitObjectId(environmentCommit);

  return {
    commit,
    tree,
    clean: statusOutput.length === 0,
    environmentCommitMatches:
      environmentCommit === undefined ? null : validEnvironmentCommit && environmentCommit === commit,
    worldPack: {
      path: 'worlds/lantern-square.world.json',
      ...(await fileEvidence(path.join(root, 'worlds', 'lantern-square.world.json'))),
    },
    assets: await directoryEvidence(path.join(root, 'apps', 'web', 'public', 'assets')),
  };
}

async function directoryEvidence(directory) {
  const entries = await directoryFiles(directory);
  const digest = createHash('sha256');
  let bytes = 0;
  for (const entry of entries) {
    const contents = await readFile(path.join(directory, ...entry.split('/')));
    const entryDigest = createHash('sha256').update(contents).digest('hex');
    bytes += contents.byteLength;
    digest.update(`${entry}\0${contents.byteLength}\0${entryDigest}\n`);
  }
  return {
    path: 'apps/web/public/assets',
    algorithm: 'sha256(path\\0bytes\\0sha256\\n)',
    files: entries.length,
    bytes,
    sha256: digest.digest('hex'),
  };
}

async function directoryFiles(directory, prefix = '') {
  const entries = await readdir(path.join(directory, ...prefix.split('/').filter(Boolean)), {
    withFileTypes: true,
  });
  const result = [];
  entries.sort((left, right) => (left.name < right.name ? -1 : left.name > right.name ? 1 : 0));
  for (const entry of entries) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      result.push(...(await directoryFiles(directory, relative)));
    } else if (entry.isFile()) {
      result.push(relative);
    } else {
      throw new Error(`candidate asset is not a regular file: ${relative}`);
    }
  }
  return result;
}

function gitObjectId(value) {
  return /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(value);
}
