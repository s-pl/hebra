# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: **Security → Report a vulnerability** on [s-pl/hebra](https://github.com/s-pl/hebra/security/advisories/new). Do not open a public issue.

Include what is affected (`hb`, `hebra-server`, the web app), the version or commit, and the steps to reproduce. You will get an answer within a week, and credit in the advisory if you want it.

## Supported versions

Hebra is in early preview. Fixes go to the latest release and `main` only.

## Scope

Of special interest: anything that lets a tree or a pack write outside a workspace, a mandate do more than it was narrowed to, one repository read another's objects, or an agent move a protected bookmark.
