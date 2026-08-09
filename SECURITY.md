# Security Policy

## Supported versions

AutoForge is alpha software. Security fixes are applied to the latest release
and the `dev` branch; older releases are not supported.

## Reporting a vulnerability

Please do not open a public issue for a suspected vulnerability. Use
[GitHub private vulnerability reporting](https://github.com/vima-tech/AutoForge/security/advisories/new)
and include:

- the affected version or commit;
- reproduction steps or a proof of concept;
- the expected impact;
- any suggested mitigation, if known.

Maintainers will acknowledge a complete report as soon as practical, keep you
updated while it is investigated, and coordinate disclosure after a fix is
available. Please avoid accessing data that is not yours or disrupting other
systems while researching a report.

## Security considerations

AutoForge can invoke local coding-agent CLIs, access configured repositories
and connect to user-supplied model or MCP services. Review integrations before
enabling them, grant the minimum necessary credentials, and use disposable or
backed-up repositories when evaluating alpha releases.
