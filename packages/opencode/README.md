# @cubtoken/opencode

Runs [cubtoken](https://github.com/brandonfla/cubtoken)'s compressor inside
OpenCode. Large `read` results become tree-sitter signature skeletons before
they reach the model.

## Install

The plugin shells out to the `ctk` binary, so install that first:

```sh
cargo install --git https://github.com/brandonfla/cubtoken ctk-cli
```

Then add the plugin to your `opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "plugin": ["@cubtoken/opencode"]
}
```

OpenCode installs npm plugins with Bun at startup — no separate `npm install`.

If `ctk` isn't on your `PATH`, point at it explicitly:

```sh
export CUBTOKEN_CTK_BIN=/path/to/ctk
```

## What it does

| OpenCode tool  | Behaviour                                                        |
| -------------- | ---------------------------------------------------------------- |
| `read`         | Over the size threshold, replaced with a signature skeleton       |
| `read` w/ range| Passed through — a targeted read is the escape hatch              |
| `edit`/`write` | Records the path so that file is never compressed again this session |
| everything else| Passed through untouched                                          |

`grep`, `glob` and `bash` folding exist in `ctk` but are not wired up here yet:
the project's measured savings are on large reads, and the others fired rarely
enough in dogfooding that their value is still unproven.

Configuration lives in `.cubtoken.toml` in your project root, same as for the
Claude Code install. See the main README.

## Fail open

Any error — no `ctk` binary, an unrecognised output shape, a crash, a timeout —
leaves the original tool output untouched. The plugin never breaks a session.

## Test

```sh
CUBTOKEN_CTK_BIN=/path/to/ctk node test.js
```

`cargo test` in the repo root runs this too, against a freshly built binary.
