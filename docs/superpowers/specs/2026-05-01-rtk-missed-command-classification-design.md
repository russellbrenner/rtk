# RTK Missed Command Classification Design

Date: 2026-05-01

## Context

`rtk discover` in `/Users/rbrenner/git/homelab` reports substantial missed savings even though many listed commands are already supported by RTK. The sample output showed about 1.3M estimated saveable tokens, with high-volume examples including `kubectl get`, `git push`, `grep -r`, `find`, `ls -la`, `cat`, `curl`, `psql`, `gh pr`, and `helm show`.

The same report also surfaced malformed or misleading command families such as `# Check`, `# Get`, `# Try`, `\ git`, `\ kubectl`, and env-prefixed commands containing token-looking values. This indicates that RTK is missing two related capabilities:

1. Shared command normalisation for shell-shaped command history.
2. A data-driven classification registry used consistently by `rtk discover` and hook rewriting.

## Goals

- Improve `rtk discover` accuracy by grouping raw Bash history under safe canonical command families.
- Improve hook rewrite coverage for commands RTK already supports.
- Redact secret-looking environment values before discover output displays examples.
- Keep hook rewrites conservative so RTK never changes complex shell semantics unexpectedly.
- Establish a small declarative registry for command family support metadata.

## Non-goals

- Do not implement broad `ssh` output compression in this phase.
- Do not rewrite nested remote commands inside `ssh`, `bash -lc`, or similar quoted strings.
- Do not attempt full POSIX shell parsing.
- Do not convert the registry to an external TOML DSL yet, although the Rust structure should make that possible later.

## Architecture

Add a shared command intelligence layer used by both discover and hook rewrite code.

The data flow is:

```text
raw Bash command
  -> command normaliser
  -> canonical command identity
  -> command registry match
  -> discover report and hook rewrite decision
```

The normaliser cleans and classifies shell-shaped command strings. The registry maps canonical command families to RTK support metadata.

## Components

### Command normaliser

The normaliser accepts a raw command string and returns a structured result.

Example input:

```text
GITEA_TOKEN=abc kubectl -n postgres get pods -o wide
```

Example output:

```rust
NormalizedCommand {
    executable: "kubectl",
    subcommand: Some("get"),
    canonical_family: "kubectl get",
    sanitized_display: "GITEA_TOKEN=<redacted> kubectl -n postgres get pods -o wide",
    ignored: false,
}
```

Responsibilities:

- Skip comment-only commands and comment-prefixed pseudo commands.
- Handle environment assignments before the executable.
- Redact secret-looking environment values in display strings.
- Handle lightweight wrappers such as `sudo`, `env`, and `command`.
- Recognise multiline artefacts such as leading `\` before real commands.
- Identify subcommands when global flags appear before them, especially for `kubectl`, `helm`, `git`, and `gh`.
- Return an unclassified result rather than failing when parsing is unsafe or unclear.

### Command registry

The registry maps canonical families to support metadata.

Example entries:

```rust
CommandPattern {
    family: "kubectl get",
    rtk_equivalent: "rtk kubectl",
    status: Existing,
    rewrite_safe: true,
}

CommandPattern {
    family: "kubectl exec",
    rtk_equivalent: "rtk kubectl",
    status: Candidate,
    rewrite_safe: false,
}
```

Statuses:

- `Existing`: RTK already supports the command family.
- `Candidate`: high-value command family worth future filter or routing work.
- `Ignore`: shell noise, comments, bookkeeping, or non-command lines.
- `Unclassified`: no confident registry match.

The registry should initially live in Rust for testability. The structure should remain declarative enough to move to TOML later if needed.

### Discover integration

`rtk discover` should:

1. Read command history as it does today.
2. Normalise each raw command.
3. Ignore noise.
4. Group by canonical family, not raw textual prefix.
5. Report commands RTK already handles but were not rewritten.
6. Report genuinely unsupported high-volume candidates separately.
7. Display only sanitised examples.

Expected cleanup:

- `# Check`, `# Get`, and `# Try` disappear from top unhandled command families.
- `\ git` and `\ kubectl` collapse into `git ...` and `kubectl ...` families.
- Env-prefixed commands group under their real command family.
- Token-looking values do not appear in examples.

### Hook rewrite integration

The Claude Code hook rewrite path should use the same classification result to decide whether to rewrite a command.

Safe rewrite example:

```text
kubectl -n argocd get pods
```

rewrites to:

```text
rtk kubectl -n argocd get pods
```

Rewrite rules:

- Rewrite only registry entries marked `Existing` and `rewrite_safe`.
- Do not double-wrap commands already prefixed with `rtk`.
- Do not rewrite ignored, unclassified, or candidate-only commands.
- Do not rewrite complex shell expressions where stdout shape or execution semantics may matter.

## Safety rules

The hook rewrite side must be stricter than discover.

Rewrite allowed:

```text
git push
kubectl -n argocd get pods
GIT_TRACE=1 git status
sudo kubectl get pods
```

Rewrite skipped:

```text
kubectl get pods | jq .
for pod in ...; do kubectl get ...
VAR=$(secret-tool read ...) kubectl get pods
ssh host "kubectl get pods"
```

Rationale:

- Pipelines and loops may depend on exact stdout shape.
- Command substitutions can be security-sensitive.
- Nested remote commands should not be rewritten locally.
- Discover can still report these as savings opportunities without changing runtime behaviour.

## Secret redaction

Discover output must redact environment values whose variable names suggest credentials.

Match variable names containing:

- `TOKEN`
- `SECRET`
- `PASSWORD`
- `PASS`
- `KEY`
- `AUTH`
- `CREDENTIAL`

Example raw command:

```text
GITEA_TOKEN="abc123" gh api repos/example/repo
```

Discover display:

```text
GITEA_TOKEN=<redacted> gh api repos/example/repo
```

## Error handling

Normalisation is best-effort and must not block workflows.

If a command cannot be parsed safely, return `Unclassified`. Then:

- discover may report it as unhandled or skip it if it is noise;
- hook rewrite must not rewrite it;
- command execution behaviour remains unchanged.

This follows RTK's existing fallback philosophy: filtering and routing support must never prevent the underlying command from running.

## Testing

Add unit tests for the normaliser:

- raw command to canonical family;
- ignored comments;
- env assignment stripping and redaction;
- wrapper handling;
- `kubectl` global flags before subcommands;
- multiline `git commit` and `kubectl` artefacts;
- already-prefixed `rtk` commands.

Add registry tests:

- known supported commands map to expected RTK equivalents;
- candidates remain non-rewriteable unless explicitly marked safe;
- unknown commands remain unclassified.

Add hook rewrite tests:

- safe commands rewrite;
- already-prefixed `rtk` commands do not double-wrap;
- pipelines, loops, command substitutions, and nested remote commands are skipped.

Add discover grouping tests:

- malformed buckets collapse into canonical families;
- comments disappear;
- redacted examples are used;
- env-prefixed supported commands are reported as existing RTK opportunities.

Verification commands:

```bash
rtk cargo fmt --all
rtk cargo clippy --all-targets
rtk cargo test --all
rtk discover
```

Manual comparison target:

```bash
cd /Users/rbrenner/git/homelab
rtk discover
```

## Success criteria

- `# Check`, `# Get`, `# Try`, `\ git`, and `\ kubectl` no longer appear as top unhandled command families.
- Env-prefixed commands are grouped under their real command family.
- `kubectl -n ... get` is classified as existing `rtk kubectl` support.
- Hook rewrite covers safe already-supported command shapes.
- Complex shell expressions are not rewritten.
- Discover examples contain no raw secret-looking environment values.
- Full Rust quality gate passes.
