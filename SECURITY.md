# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | yes       |

## Reporting a vulnerability

This is a local developer tool: it reads a project tree, counts LOC **in-process**, shells out to `git` and optionally `dust`, and writes only under that project’s `.heim/` directory.

If you find a security issue (path traversal outside the target tree, command injection, unsafe temp handling, etc.), please open a **private** GitHub security advisory on the repository, or email the maintainer listed in `Cargo.toml` / GitHub profile.

Please do **not** open a public issue for exploitable bugs until a fix is available.

## Scope notes

- heim does not send data over the network
- heim does not require credentials or API keys
- Monitored projects may contain secrets; keep `.heim/` out of version control (default)
