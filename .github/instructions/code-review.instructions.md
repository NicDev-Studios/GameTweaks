---
applyTo: "**"
excludeAgent: "cloud-agent"
---

# GameTweaks code review instructions

- Lead with concrete defects, regressions, security risks, or missing tests. Give
  a file and tight line range when possible.
- Verify backend enforcement instead of accepting frontend-only validation.
- Treat weakened origin, digest, archive, path, symlink, process, DACL, secret,
  marker, collision, rollback, and ownership checks as high-priority findings.
- Flag any path that can overwrite or remove files not proven to be owned by the
  current GameTweaks transaction.
- Flag Tauri commands that expose local paths, download URLs, secrets, or broad
  filesystem and shell capabilities.
- Check multi-mod dependency ordering, conflicts, update ownership, uninstall
  dependency protection, one-time plans, and rollback of partially committed
  files.
- Check Agent messages for authentication, bounded framing, strict message types,
  process identity, ambiguous instances, and preservation of unconfirmed values.
- Require tests when security boundaries, parsers, transactions, configs, IPC,
  or public command contracts change.
- Do not accuse a contributor of using AI based on prose or coding style. Flag
  objective quality problems such as fabricated data, contradictory claims,
  irrelevant churn, unverifiable test claims, or the explicit select-all
  attention checkbox being checked in the pull request body.
- Do not block on subjective style preferences when behavior is correct and
  consistent with the repository.
