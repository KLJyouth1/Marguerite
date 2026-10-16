# Security Policy

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

Use GitHub's private reporting flow:

1. Go to the repository's **Security** tab → **Report a vulnerability**
   (this opens a private advisory visible only to the maintainers), or use the
   direct link: `https://github.com/<OWNER>/<REPO>/security/advisories/new`.
2. Include, as far as you can:
   * the affected component (`core/`, `tui/`, `gui/`, `webui/`, `pyengine/`) and
     the version or commit;
   * a minimal reproduction — exact commands, a crafted file, or a gateway
     request sequence;
   * the impact you believe it has, and whether it needs local access, a
     malicious plugin, or only a network-reachable service;
   * any suggested fix or mitigation.

If the private flow is unavailable to you, open a **minimal public issue that
contains no exploit details** and ask a maintainer to establish a private
channel. Do not paste the reproduction into that issue.

### What to expect

| Stage | Target |
|---|---|
| Acknowledgement of the report | within 3 working days |
| Initial assessment (severity, affected versions, planned fix) | within 10 working days |
| Fix or documented mitigation for confirmed high/critical issues | as fast as the release cadence allows; you will be kept informed of the schedule |
| Public disclosure | coordinated with you, after a fix or mitigation ships |

Credit is given in the advisory and the release notes unless you ask to remain
anonymous. Please give us a reasonable window to ship a fix before publishing.

## Supported versions

Marguerite is pre-1.0 and the version numbers of the individual components are
not yet unified (see the table in `README.md`/`CHANGELOG.md`). Security fixes are
issued against the latest release line only.

| Version | Supported |
|---|---|
| Latest tagged release on the default branch | ✅ |
| `master` (unreleased) | ✅ (fixes land here first) |
| Older tags | ❌ — please reproduce on the latest release |

## Security model

Understanding the intended boundaries makes reports much easier to triage. These
are guarantees the project intends to keep — a violation of any of them is a bug
worth reporting:

1. **Loopback only.** The unified gateway and the Vue console listen on
   `127.0.0.1` (default port `18790`). The ACP `serve` transport also binds
   `127.0.0.1` only. Nothing in this project listens on `0.0.0.0` or on a public
   interface, and the desktop front end never opens a public listener. The
   gateway is not authenticated for remote use because it is not reachable
   remotely — exposing it (a reverse proxy, a container port publish, an SSH
   `-R` forward onto a shared host) removes that protection and is a deployment
   error, not a supported configuration.
2. **Local trust boundary.** Once a process can talk to the loopback gateway it is
   treated as the user: it can drive sessions, tools and the desktop automation
   surface. Treat any local account or process that can reach the port as
   equivalent to the user's own shell.
3. **Governance gates before and after execution.** `PlanGate` (constitution
   C1–C4) reviews a plan before it runs and `CompletionGate` verifies the claimed
   output afterwards, including checking that self-reported files actually exist
   on disk.
4. **Sandbox and approvals.** Tool execution that touches the filesystem or the
   network goes through the sandbox and approval stack. Approvals are a security
   control, not a UX speed bump: a path that executes without surfacing the
   required approval is a vulnerability.
5. **Secrets stay out of the repository.** API keys live in the environment or in
   `.env`, which is git-ignored. Example keys in tests are obvious fixtures
   (`sk-FAKE…`, `sk-abcdefghij…`) and must never be replaced with real values.
   The engine redacts recognised secret shapes from its own logs.
6. **Plugins are code.** A plugin manifest declaring `system_prompt` or
   `full_context` is treated as a context-leak suspect, and replacing an
   already-provided service type is rejected outside an explicit `reload_plugin()`.
   These guards reduce blast radius; they do not make an untrusted plugin safe.
   Install third-party plugins the way you would install any other executable
   code.

### Out of scope

The following are known and accepted, and therefore are not treated as
vulnerabilities on their own:

* Prompt injection that makes the agent use tools **the user already approved**,
  when the injection comes from content the user chose to feed it. Reports are
  still welcome when the injection crosses an approval boundary or exfiltrates
  data without a user-visible action.
* Attacks that require an already-compromised local account with access to the
  user's environment, `~/.marguerite/` state, or the loopback port.
* Denial of service caused by the user running the engine against a hostile or
  enormous input on their own machine.
* Missing hardening on a build you compiled yourself with modified sources.

## Hardening checklist for deployments

* Keep the gateway bound to `127.0.0.1`; never publish its port.
* Keep `.env` out of version control and out of bug reports.
* Run the engine as an unprivileged user; do not run the desktop automation
  surface as an administrator.
* Pin to a tagged release and read the release notes before upgrading.
