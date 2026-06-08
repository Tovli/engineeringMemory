# Security Policy

## Supported Versions

`tovli` is currently pre-release software. Security fixes are handled on the
main development line unless a release branch policy is introduced later.

| Version | Supported |
|---------|-----------|
| `main`  | Yes       |
| `<1.0` releases | Best effort |

## Reporting a Vulnerability

Please do not publish vulnerability details in a public issue.

Use the repository's private vulnerability reporting feature if available. If
the repository does not expose a private reporting channel yet, contact the
maintainer through the private contact listed in the repository profile. If no
private contact is available, open a public issue that asks for a security
contact without including exploit details.

Useful information to include in a private report:

- Affected command, module, or file path.
- A minimal reproduction.
- Expected impact.
- Whether the issue involves local files, generated `.tovli/` state, document
  content, provider integrations, or command-line arguments.
- Any suggested fix or mitigation.

## Project Security Scope

Current areas of interest:

- Local file ingestion and path handling.
- Parser behavior for untrusted or malformed documents.
- Storage in `.tovli/`.
- Handling of private engineering documents.
- Future provider adapters that may involve API keys or network calls.
- Future HTTP API or bot integrations.

The default build uses deterministic local mock providers and should not require
network credentials.

## Responsible Disclosure

Maintainers will make a best effort to acknowledge valid reports, coordinate a
fix, and credit reporters when requested. Do not disclose details publicly until
a fix or mitigation is available.
