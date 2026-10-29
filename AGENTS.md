# Repository Guidelines

## Mandatory RTK Usage

All agents working in this repository MUST run terminal commands through RTK explicitly, including builds, checks, tests, searches, and file reads. This repository rule overrides general guidance that commands may be run normally.

- Use the dedicated wrapper when available: `rtk cargo build`, `rtk cargo check`, `rtk cargo test`, `rtk cargo clippy`, `rtk npm run build`, `rtk git status`, `rtk rg`, and `rtk read`.
- For tools or PowerShell scripts without a dedicated wrapper, use `rtk summary <command> ...`. Use `rtk run` only when needed to preserve execution semantics. In ad hoc shell blocks and pipelines, wrap each invoked tool explicitly.
- Always try the filtered command first. Treat its output and exit code as authoritative. A command failing does not by itself justify bypassing RTK when the diagnostics already explain the failure.
- If essential diagnostic details are missing, first follow the recovery hint with `rtk recall <hash>`; request only the relevant lines. If no recovery is available or it remains insufficient, rerun only the affected command with `rtk bypass <command> ...` to obtain the needed raw output.
- Bypass is also allowed when an exact, unmodified artifact is required and filtering changes it. Limit bypass to that extraction. Briefly state the concrete reason for each bypass and return to filtered RTK commands immediately afterward.
- Minimize bypasses: do not use them preemptively, rerun successful commands just to obtain verbose logs, or route raw output through another shell to evade filtering. Pass these rules to any delegated agents.

## Project Structure & Module Organization

OSCMidi is a Windows x64 MIDI/RTP-MIDI/OSC bridge with isolated VST workers.

- `src/`: React/TypeScript interface; pages in `pages/`, shared controls in `components/`, runtime state in `providers/`, and English/French strings in `locales/`.
- `src-tauri/src/`: Rust routing, audio, configuration, Tauri commands, and worker supervision. Integration tests live in `src-tauri/tests/`; application icons live in `src-tauri/icons/`.
- `vendor/`: locally maintained MIDI and VST dependencies; `ASIOSDK/` supplies ASIO headers.
- `tools/diagnostics/`: standalone probes and stress tools. `scripts/` contains build and validation automation.

See `src-tauri/ARCHITECTURE.md` before changing process boundaries or audio routing.

## Build, Test, and Development Commands

Use Node.js 20.19+ or 22.12+ and the pinned Rust toolchain. Desktop builds require Windows MSVC tools, CMake, and LLVM; see `README.md` for setup.

- `rtk npm ci`: install locked frontend dependencies.
- `rtk npm run dev`: start the Vite frontend server.
- `rtk npm run tauri:dev`: stage the debug VST worker and launch the desktop application.
- `rtk npm run lint`: run ESLint and strict TypeScript checks.
- `rtk npm test`: run frontend tests once.
- `rtk npm run build`: build the production frontend.
- `rtk npm run tauri:build`: build the release application and MSI.
- `rtk summary powershell -NoProfile -File ./scripts/validate.ps1 -Scope Rust`: run Rust formatting, Clippy, tests, and native audio checks on Windows; use `-Scope All` for combined validation.

## Coding Style & Naming Conventions

Use two-space indentation in TypeScript/TSX and rustfmt defaults in Rust. Match surrounding quote and semicolon conventions. Use PascalCase for React components, `useCamelCase` for hooks, and snake_case for Rust modules/functions. Prefer explicit `import type` for TypeScript types. Keep translated UI strings aligned in `src/locales/en.ts` and `fr.ts`.

## Testing Guidelines

Frontend tests use Vitest, jsdom, and React Testing Library; colocate `*.test.ts`/`*.test.tsx` files with code. Run a focused suite with `rtk npm test -- src/pages/SettingsPage.test.tsx`. Rust uses inline unit tests and integration tests. Add regression coverage for behavior changes, especially configuration persistence, routing, and worker failures. No numeric coverage threshold is configured.

## Commit & Pull Request Guidelines

Follow existing Conventional Commit patterns: `fix(ui): ...`, `fix(config): ...`, or `chore(release): ...`. Keep commits focused. PRs should describe the problem, resulting behavior, validation performed, and related issues; include screenshots for visible UI changes. Run relevant validation before requesting review. Keep credentials, signing keys, generated binaries, and diagnostic artifacts out of commits.
