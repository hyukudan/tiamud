# Security Policy

## Supported versions

Tiamud has not yet published a supported tagged release. Until it does,
security fixes are developed against the repository's current default branch
on a best-effort basis. Development snapshots are not a compatibility or
long-term-support promise.

| Target | Security support |
| --- | --- |
| Current default branch | Best-effort fixes for current source |
| Tagged releases | None published yet |
| Older commits, forks, and third-party builds | No separate support commitment |

This table will identify supported release lines when a tagged release is
published. Do not assume that an older version receives security updates unless
it is listed here.

## Report a vulnerability privately

Use the repository's [GitHub Security Advisory
form](https://github.com/hyukudan/tiamud/security/advisories/new) to report a
suspected vulnerability. In GitHub, this is available from **Security →
Advisories → Report a vulnerability** when private vulnerability reporting is
enabled.

Do not put exploitable details in a public issue, pull request, discussion, log,
or chat. If GitHub does not offer the private form, open a public issue that
only asks the maintainers to provide a private reporting path; do not include
the vulnerability details there.

Include, when available:

- the affected commit, version, platform, storage backend, and deployment mode;
- the impact, required access, and attack preconditions;
- minimal reproduction steps or a small proof of concept against a disposable
  installation you control;
- sanitized logs, stack traces, and relevant configuration;
- a proposed mitigation or fix, if known; and
- any disclosure constraints or known active exploitation.

Never submit live passwords, session cookies, API tokens, private keys,
database URLs, personal data, or production database contents. Replace secrets
with unmistakable placeholders and rotate any credential that may already have
been exposed. Do not test against systems or accounts you do not own or have
explicit permission to assess.

The project handles reports on a best-effort basis and does not promise a
specific acknowledgement or remediation deadline. Maintainers will try to
validate the impact, keep sensitive coordination inside the advisory, prepare
a fix and release guidance where appropriate, and coordinate disclosure with
the reporter. This project does not currently promise a vulnerability bounty.

## Scope

Useful reports include vulnerabilities in maintained Tiamud source,
configuration, deployment examples, and official release automation,
especially:

- authentication, session, role, moderation, and authorization bypasses;
- cross-player information disclosure or control of another live character;
- remote code execution, persistent script or browser injection, and unsafe
  handling of WorldPack, asset, or legacy-import input;
- WebSocket, HTTP, Telnet, database, and migration flaws that compromise
  confidentiality, integrity, or availability;
- reproducible denial of service that bypasses documented bounds or rate
  limits; and
- dependency or build/release supply-chain issues with a reachable effect on
  Tiamud.

The following are generally outside the maintained security boundary:

- forks, downstream patches, unofficial binaries, and infrastructure not
  controlled by this project;
- public plaintext Telnet deployments, publicly exposed metrics, or other
  configurations that explicitly contradict the [single-host
  runbook](docs/operations/single-host.md);
- a downstream operator's leaked credential without a Tiamud defect that
  caused the disclosure;
- bugs or licensing problems in third-party narrative content that do not
  exploit Tiamud's handling of that content; and
- scanner output or theoretical concerns without a reproducible Tiamud impact.

Reports can still be submitted when scope is uncertain. The distinction above
does not exclude a Tiamud flaw merely because malformed or hostile input is
needed to trigger it.

## Deployment hardening

Tiamud's reference deployment is a single-host baseline, not a managed security
service. Operators remain responsible for host, network, database, certificate,
backup, monitoring, and incident-response controls. At minimum:

- follow the [single-host runbook](docs/operations/single-host.md), run the
  process as a dedicated unprivileged account, and keep the host and
  dependencies patched;
- bind HTTP to loopback behind an HTTPS/WSS reverse proxy, keep `/metrics`
  private, and use firewall and proxy connection limits in addition to Tiamud's
  rate limits;
- use valid direct TLS for every public Telnet listener; the explicit
  `plaintext` mode is only for development or a trusted private network;
- restrict environment files, TLS private keys, databases, WorldPacks, logs,
  and backups to the identities that need them, and keep secrets out of source
  control and command-line history;
- review and validate imported content in a disposable environment before
  publication, and run the service with no filesystem or network privileges
  beyond those documented;
- create encrypted off-host backups, verify them, and exercise restoration;
  and
- verify release checksums and provenance as described in the [release
  runbook](docs/operations/releases.md) once an official tagged release exists.

The repository examples reduce common exposure but cannot replace a deployment
threat model, timely patching, least privilege, external monitoring, or an
operator's tested incident-response and recovery plan.
