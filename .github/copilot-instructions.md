---
applyTo: "**"
---

# GameGate Review Instructions

GameGate is a Tauri 2 desktop app for managing local game and development servers. The backend is Rust + Tokio under `src-tauri`, and the frontend is Svelte + TypeScript under `src`.

When reviewing pull requests, prioritize correctness, maintainability, security, and alignment with the current foundation milestone. Raise concrete issues when code creates defects, weakens architecture, lowers code quality, or introduces security risk. Avoid blocking on subjective style preferences unless they affect readability, maintainability, or consistency with the existing codebase.

## Code Quality

- Require narrow, reviewable changes that match the existing project structure.
- Flag broad refactors, unrelated formatting churn, or behavior changes that are not needed for the PR goal.
- Check that Rust and TypeScript data contracts stay explicit and serializable: use `serde` types on the backend and explicit TypeScript interfaces on the frontend.
- Prefer typed APIs and structured validation over ad hoc parsing, stringly typed state, or duplicated logic.
- Tauri command handlers under `src-tauri/src/commands` should stay thin: validate inputs, call backend services, and return typed DTOs.
- Frontend feature code should stay scoped under `src/lib/features`; shared frontend API wrappers belong under `src/lib/api`.
- Flag missing tests or missing quality checks when a change affects shared behavior, command contracts, security boundaries, process handling, filesystem access, networking, or persistent configuration.

## Security Review

- Treat security regressions as high-priority findings. Do not approve code that creates obvious vulnerabilities.
- Never accept execution of raw shell strings from user input. Future process execution must use structured command definitions and explicit arguments.
- Never expose local ports publicly without an explicit user action and clear opt-in flow.
- Do not add remote code execution.
- Do not add dynamic plugin execution unless there is a documented sandboxing, signing, and permission model.
- Keep plugin work limited to metadata and registry scaffolding unless the PR explicitly introduces a reviewed sandbox design.
- Keep tunnel providers opt-in. No provider should expose ports automatically.
- Keep Tauri permissions minimal and question newly added capabilities, filesystem scope, process permissions, network access, and updater behavior.
- Check Rust async code for unbounded tasks, blocking work on async executors, missed error handling, and state races.
- Check frontend code for unsafe HTML injection, unchecked external URLs, leaky local path display, and trust in client-side validation for privileged actions.

## Product And UI Boundaries

- The current milestone is foundation work: architecture, UI shell, configuration models, CI, and safe extension boundaries.
- Do not accept full server execution, tunnel provider implementations, or plugin runtime behavior unless the PR explicitly asks for it and includes appropriate safety design.
- Do not add fake data, fake live status, placeholder metrics, or misleading populated UI states.
- UI should remain compact and desktop-app oriented, with neutral glass surfaces, readable contrast, and stable layouts.
- Avoid blue-heavy dark themes, large marketing-style hero layouts, and decorative UI that distracts from repeated server-management workflows.

## Review Style

- Lead with actionable findings and explain the concrete failure mode or risk.
- Include file and line references when possible.
- Distinguish required fixes from optional improvements.
- Do not request changes only because another implementation style is possible.
- If a PR changes security-relevant behavior, verify that the PR description explains process execution, filesystem access, networking, plugin behavior, and other privileged boundaries clearly.
