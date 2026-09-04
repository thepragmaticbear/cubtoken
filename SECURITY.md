# Security policy

## Supported versions

Security fixes are made on the latest released version. Upgrade to the newest
release before reporting a problem that may already be fixed.

## Reporting a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/brandonfla/cubtoken/security/advisories/new).
Do not include exploit details or sensitive data in a public issue.

Include the affected cubtoken and Claude Code versions, operating system,
reproduction steps, impact, and any suggested mitigation. You can expect an
acknowledgement through the private advisory and coordinated disclosure after
a fix is available.

## Security model

cubtoken runs locally with the same permissions as Claude Code. It does not
make network requests or execute content from tool output. It reads hook JSON
from stdin, optionally rewrites the output, and writes only local configuration
and ledger files.

The `.cubtoken/` ledger stores file paths, estimated token counts, and tool
durations; it does not store file contents. The separate `ctk record` debugging
command does store raw hook payloads and may therefore capture source code,
paths, command output, or secrets. Keep recordings private and delete them when
they are no longer needed.
